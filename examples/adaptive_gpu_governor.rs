use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Error as IoError, Read, Write};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const GRBM_STATUS_REG: u32 = 0x2004;
const GUI_ACTIVE_BIT_MASK: u32 = 1 << 31;

const MIN_FREQ_MHZ: u16 = 350;
const MAX_FREQ_MHZ: u16 = 2000;
const FREQ_STEP_MHZ: u16 = 50;

const MIN_VOLTAGE_MV: u16 = 700;
const MAX_VOLTAGE_MV: u16 = 1000;

const HIGH_LOAD_THRESHOLD: f32 = 90.0;
const LOW_LOAD_THRESHOLD: f32 = 50.0;
const SAMPLE_WINDOW_SIZE: usize = 100;
const MIN_CHANGE_INTERVAL_SECS: u64 = 2;

// Durée phase d'apprentissage (5 minutes)
const LEARNING_DURATION_SECS: u64 = 300;
// Seuil minimum de confort pour rester locked
const MIN_COMFORT_SCORE: f32 = 95.0;
// Durée avant réévaluation si locked (30 minutes)
const REEVALUATION_INTERVAL_SECS: u64 = 10;

/// Moniteur de charge GPU avec fenêtre glissante
struct GpuLoadMonitor {
    samples: VecDeque<bool>,
    capacity: usize,
}

impl GpuLoadMonitor {
    fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn add_sample(&mut self, is_active: bool) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(is_active);
    }

    fn load_percent(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let active_count = self.samples.iter().filter(|&&s| s).count();
        (active_count as f32 / self.samples.len() as f32) * 100.0
    }

    fn is_full(&self) -> bool {
        self.samples.len() >= self.capacity
    }
}

/// Statistiques pour une fréquence donnée
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FrequencyStats {
    freq_mhz: u16,
    #[serde(with = "duration_serde")]
    time_spent: Duration,
    load_samples: Vec<f32>,
    #[serde(skip)]
    last_entry: Option<Instant>,
}

mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_secs_f64().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = f64::deserialize(deserializer)?;
        Ok(Duration::from_secs_f64(secs))
    }
}

impl FrequencyStats {
    fn new(freq_mhz: u16) -> Self {
        Self {
            freq_mhz,
            time_spent: Duration::ZERO,
            load_samples: Vec::new(),
            last_entry: None,
        }
    }

    fn enter(&mut self) {
        self.last_entry = Some(Instant::now());
    }

    fn exit(&mut self) {
        if let Some(entry_time) = self.last_entry.take() {
            self.time_spent += entry_time.elapsed();
        }
    }

    fn add_load_sample(&mut self, load: f32) {
        self.load_samples.push(load);
    }

    fn average_load(&self) -> f32 {
        if self.load_samples.is_empty() {
            return 0.0;
        }
        self.load_samples.iter().sum::<f32>() / self.load_samples.len() as f32
    }

    fn comfort_score(&self) -> f32 {
        // Score de confort: pénalise les charges trop hautes (goulot) ou trop basses (gaspillage)
        // Score optimal entre 60-80%
        let avg_load = self.average_load();
        let ideal_load = 70.0;
        let deviation = (avg_load - ideal_load).abs();

        // Score de 0 à 100, 100 = parfait
        (100.0 - deviation).max(0.0)
    }
}

/// Collecteur de statistiques pour toutes les fréquences
struct StatsCollector {
    stats: BTreeMap<u16, FrequencyStats>,
    current_freq: Option<u16>,
}

impl StatsCollector {
    fn new() -> Self {
        let mut stats = BTreeMap::new();

        // Initialiser les stats pour toutes les fréquences possibles
        let mut freq = MIN_FREQ_MHZ;
        while freq <= MAX_FREQ_MHZ {
            stats.insert(freq, FrequencyStats::new(freq));
            freq += FREQ_STEP_MHZ;
        }

        Self {
            stats,
            current_freq: None,
        }
    }

    fn set_frequency(&mut self, freq: u16, load: f32) {
        // Sortir de la fréquence précédente
        if let Some(prev_freq) = self.current_freq {
            if let Some(stat) = self.stats.get_mut(&prev_freq) {
                stat.exit();
            }
        }

        // Entrer dans la nouvelle fréquence
        if let Some(stat) = self.stats.get_mut(&freq) {
            stat.enter();
            stat.add_load_sample(load);
        }

        self.current_freq = Some(freq);
    }

    fn add_load_sample(&mut self, load: f32) {
        if let Some(freq) = self.current_freq {
            if let Some(stat) = self.stats.get_mut(&freq) {
                stat.add_load_sample(load);
            }
        }
    }

    fn get_optimal_frequency(&self) -> Option<(u16, f32)> {
        self.stats
            .iter()
            .filter(|(_, s)| s.load_samples.len() >= 10) // Au moins 10 échantillons
            .max_by(|(_, a), (_, b)| a.comfort_score().partial_cmp(&b.comfort_score()).unwrap())
            .map(|(freq, stat)| (*freq, stat.comfort_score()))
    }

    fn has_sufficient_data(&self) -> bool {
        self.stats
            .values()
            .filter(|s| s.load_samples.len() >= 5)
            .count()
            >= 3
    }

    fn save_to_file(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(&self.stats)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    fn load_from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let stats: BTreeMap<u16, FrequencyStats> = serde_json::from_str(&contents)?;
        Ok(Self {
            stats,
            current_freq: None,
        })
    }

    fn print_summary(&self) {
        println!("\n=== STATISTIQUES DES FRÉQUENCES ===\n");
        println!(
            "{:<6} | {:<12} | {:<10} | {:<10} | {:<10}",
            "Freq", "Temps", "Charge moy", "Échantillons", "Confort"
        );
        println!(
            "{:-<6}-+-{:-<12}-+-{:-<10}-+-{:-<10}-+-{:-<10}",
            "", "", "", "", ""
        );

        for (freq, stat) in &self.stats {
            if stat.time_spent.as_secs() > 0 || !stat.load_samples.is_empty() {
                let time_str = format!("{:.1}s", stat.time_spent.as_secs_f32());
                let load_str = format!("{:.1}%", stat.average_load());
                let samples_str = format!("{}", stat.load_samples.len());
                let comfort_str = format!("{:.1}/100", stat.comfort_score());

                println!(
                    "{:<6} | {:<12} | {:<10} | {:<10} | {:<10}",
                    format!("{}MHz", freq),
                    time_str,
                    load_str,
                    samples_str,
                    comfort_str
                );
            }
        }

        // Trouver la fréquence la plus confortable
        if let Some((best_freq, best_score)) = self.get_optimal_frequency() {
            let best_stat = &self.stats[&best_freq];
            println!(
                "\n✓ Fréquence optimale: {} MHz (confort: {:.1}/100, charge: {:.1}%)",
                best_freq,
                best_score,
                best_stat.average_load()
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GovernorMode {
    Learning,  // Phase d'apprentissage: explore les fréquences
    Locked,    // Locked sur fréquence optimale
    Adjusting, // Ajustement temporaire si confort dégradé
}

/// Gouverneur adaptatif avec modes
struct SimpleGovernor {
    current_freq: u16,
    optimal_freq: Option<u16>,
    mode: GovernorMode,
    mode_start: Instant,
    last_change: Instant,
    last_check: Instant,
    min_change_interval: Duration,
    load_history: VecDeque<f32>,
    history_size: usize,
    discomfort_count: u32,
}

impl SimpleGovernor {
    fn new(starting_freq: u16, mode: GovernorMode) -> Self {
        Self {
            current_freq: starting_freq,
            optimal_freq: None,
            mode,
            mode_start: Instant::now(),
            last_change: Instant::now(),
            last_check: Instant::now(),
            min_change_interval: Duration::from_secs(MIN_CHANGE_INTERVAL_SECS),
            load_history: VecDeque::with_capacity(20),
            history_size: 10,
            discomfort_count: 0,
        }
    }

    fn set_optimal_freq(&mut self, freq: u16) {
        self.optimal_freq = Some(freq);
    }

    fn switch_to_locked(&mut self, optimal_freq: u16) {
        self.mode = GovernorMode::Locked;
        self.optimal_freq = Some(optimal_freq);
        self.current_freq = optimal_freq;
        self.mode_start = Instant::now();
        self.load_history.clear();
        self.discomfort_count = 0;
    }

    fn switch_to_adjusting(&mut self) {
        self.mode = GovernorMode::Adjusting;
        self.mode_start = Instant::now();
        self.discomfort_count = 0;
    }

    fn switch_to_learning(&mut self) {
        self.mode = GovernorMode::Learning;
        self.mode_start = Instant::now();
        self.discomfort_count = 0;
    }

    fn add_load_sample(&mut self, load: f32) {
        if self.load_history.len() >= self.history_size {
            self.load_history.pop_front();
        }
        self.load_history.push_back(load);
    }

    fn should_increase(&self) -> bool {
        if self.current_freq >= MAX_FREQ_MHZ {
            return false;
        }
        if self.load_history.len() < self.history_size {
            return false;
        }
        let avg = self.load_history.iter().sum::<f32>() / self.load_history.len() as f32;
        avg >= HIGH_LOAD_THRESHOLD
    }

    fn should_decrease(&self) -> bool {
        if self.current_freq <= MIN_FREQ_MHZ {
            return false;
        }
        if self.load_history.len() < self.history_size {
            return false;
        }
        let avg = self.load_history.iter().sum::<f32>() / self.load_history.len() as f32;
        avg <= LOW_LOAD_THRESHOLD
    }

    fn check_comfort(&mut self, current_load: f32) -> bool {
        // Vérifie si on est dans une situation inconfortable
        if self.last_check.elapsed() < Duration::from_secs(5) {
            return true; // Pas encore de vérification
        }
        self.last_check = Instant::now();

        // Inconfortable si charge trop haute (GPU saturé)
        if current_load > HIGH_LOAD_THRESHOLD {
            self.discomfort_count += 1;
        } else {
            self.discomfort_count = 0;
        }

        // Si inconfort persistant (3 fois de suite)
        self.discomfort_count < 3
    }

    fn try_adjust(&mut self, stats: &StatsCollector) -> Option<u16> {
        match self.mode {
            GovernorMode::Learning => {
                // Mode apprentissage: comportement normal
                if self.last_change.elapsed() < self.min_change_interval {
                    return None;
                }

                let new_freq = if self.should_increase() {
                    (self.current_freq + FREQ_STEP_MHZ).min(MAX_FREQ_MHZ)
                } else if self.should_decrease() {
                    self.current_freq
                        .saturating_sub(FREQ_STEP_MHZ)
                        .max(MIN_FREQ_MHZ)
                } else {
                    return None;
                };

                if new_freq != self.current_freq {
                    self.current_freq = new_freq;
                    self.last_change = Instant::now();
                    self.load_history.clear();
                    return Some(new_freq);
                }
            }
            GovernorMode::Locked => {
                // Mode locked: rester à la fréquence optimale
                // Sauf si on détecte un inconfort
                if let Some(optimal) = self.optimal_freq {
                    if self.current_freq != optimal {
                        self.current_freq = optimal;
                        return Some(optimal);
                    }
                }
            }
            GovernorMode::Adjusting => {
                // Mode ajustement: comme Learning mais peut retourner en Locked
                if self.last_change.elapsed() < self.min_change_interval {
                    return None;
                }

                let new_freq = if self.should_increase() {
                    (self.current_freq + FREQ_STEP_MHZ).min(MAX_FREQ_MHZ)
                } else if self.should_decrease() {
                    self.current_freq
                        .saturating_sub(FREQ_STEP_MHZ)
                        .max(MIN_FREQ_MHZ)
                } else {
                    return None;
                };

                if new_freq != self.current_freq {
                    self.current_freq = new_freq;
                    self.last_change = Instant::now();
                    self.load_history.clear();
                    return Some(new_freq);
                }
            }
        }
        None
    }

    fn current_freq(&self) -> u16 {
        self.current_freq
    }
}

fn interpolate_voltage(freq: u16) -> u16 {
    if freq <= MIN_FREQ_MHZ {
        return MIN_VOLTAGE_MV;
    }
    if freq >= MAX_FREQ_MHZ {
        return MAX_VOLTAGE_MV;
    }

    let freq_range = MAX_FREQ_MHZ - MIN_FREQ_MHZ;
    let voltage_range = MAX_VOLTAGE_MV - MIN_VOLTAGE_MV;
    let freq_offset = freq - MIN_FREQ_MHZ;

    MIN_VOLTAGE_MV + (freq_offset as u32 * voltage_range as u32 / freq_range as u32) as u16
}

fn set_gpu_frequency(pp_file: &mut File, freq: u16) -> Result<(), Box<dyn std::error::Error>> {
    let voltage = interpolate_voltage(freq);
    pp_file.write_all(format!("vc 0 {} {}\n", freq, voltage).as_bytes())?;
    pp_file.write_all(b"c\n")?;
    pp_file.flush()?;
    Ok(())
}

fn get_stats_path() -> PathBuf {
    let mut path = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    path.push("cyan-skillfish-governor");
    std::fs::create_dir_all(&path).ok();
    path.push("freq_stats.json");
    path
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Governor GPU Adaptatif Permanent ===\n");

    let location = BUS_INFO {
        domain: 0,
        bus: 1,
        dev: 0,
        func: 0,
    };

    let card = File::open(location.get_drm_render_path()?)?;
    let (dev_handle, _, _) = DeviceHandle::init(card.as_raw_fd())
        .map_err(|e| format!("Échec ouverture GPU: erreur {}", e))?;

    let sysfs_path = dev_handle
        .get_sysfs_path()
        .map_err(IoError::from_raw_os_error)?;

    let mut pp_file = OpenOptions::new()
        .write(true)
        .open(sysfs_path.join("pp_od_clk_voltage"))?;

    // Charger les stats existantes ou créer nouvelles
    let stats_path = get_stats_path();
    let mut stats = if stats_path.exists() {
        println!("📊 Chargement des statistiques existantes...");
        match StatsCollector::load_from_file(&stats_path) {
            Ok(s) => {
                println!(
                    "✓ Statistiques chargées: {} fréquences avec données\n",
                    s.stats
                        .values()
                        .filter(|st| !st.load_samples.is_empty())
                        .count()
                );
                s
            }
            Err(e) => {
                println!(
                    "⚠ Erreur chargement stats: {}, création nouvelles stats\n",
                    e
                );
                StatsCollector::new()
            }
        }
    } else {
        println!("📊 Création nouvelles statistiques...\n");
        StatsCollector::new()
    };

    // Déterminer mode de démarrage
    let (mode, starting_freq) = if let Some((optimal_freq, score)) = stats.get_optimal_frequency() {
        if stats.has_sufficient_data() && score >= MIN_COMFORT_SCORE {
            println!(
                "🔒 Mode LOCKED: Fréquence optimale détectée: {} MHz (confort: {:.1}/100)\n",
                optimal_freq, score
            );
            (GovernorMode::Locked, optimal_freq)
        } else {
            println!("📚 Mode LEARNING: Données insuffisantes ou confort trop faible\n");
            println!(
                "   Phase d'apprentissage: {} secondes ({} minutes)\n",
                LEARNING_DURATION_SECS,
                LEARNING_DURATION_SECS / 60
            );
            (GovernorMode::Learning, MIN_FREQ_MHZ)
        }
    } else {
        println!("📚 Mode LEARNING: Première exécution\n");
        println!(
            "   Phase d'apprentissage: {} secondes ({} minutes)\n",
            LEARNING_DURATION_SECS,
            LEARNING_DURATION_SECS / 60
        );
        (GovernorMode::Learning, MIN_FREQ_MHZ)
    };

    let mut load_monitor = GpuLoadMonitor::new(SAMPLE_WINDOW_SIZE);
    let mut governor = SimpleGovernor::new(starting_freq, mode);

    // Si mode locked, définir la fréquence optimale
    if matches!(mode, GovernorMode::Locked) {
        if let Some((optimal, _)) = stats.get_optimal_frequency() {
            governor.set_optimal_freq(optimal);
        }
    }

    let mut sample_count = 0u64;
    let mut last_display = Instant::now();
    let mut last_save = Instant::now();

    set_gpu_frequency(&mut pp_file, starting_freq)?;
    stats.set_frequency(starting_freq, 0.0);

    println!("🚀 Monitoring démarré... (Ctrl+C pour arrêter)\n");

    loop {
        let grbm_status = dev_handle
            .read_mm_registers(GRBM_STATUS_REG)
            .map_err(|e| format!("Échec lecture registre GPU: erreur {}", e))?;
        let is_active = (grbm_status & GUI_ACTIVE_BIT_MASK) != 0;

        load_monitor.add_sample(is_active);
        sample_count += 1;

        if load_monitor.is_full() {
            let load = load_monitor.load_percent();
            governor.add_load_sample(load);
            stats.add_load_sample(load);

            // Gestion des transitions de mode
            match governor.mode {
                GovernorMode::Learning => {
                    // Fin de la phase d'apprentissage?
                    if governor.mode_start.elapsed() >= Duration::from_secs(LEARNING_DURATION_SECS)
                        && stats.has_sufficient_data()
                    {
                        if let Some((optimal_freq, score)) = stats.get_optimal_frequency() {
                            println!("\n\n🎯 Phase d'apprentissage terminée!");
                            stats.print_summary();
                            println!("\n🔒 Passage en mode LOCKED à {} MHz\n", optimal_freq);
                            governor.switch_to_locked(optimal_freq);
                            set_gpu_frequency(&mut pp_file, optimal_freq)?;
                            stats.set_frequency(optimal_freq, load);
                        }
                    } else {
                        // Ajustement normal en mode learning
                        if let Some(new_freq) = governor.try_adjust(&stats) {
                            let direction = if new_freq > governor.current_freq() {
                                "↑"
                            } else {
                                "↓"
                            };
                            println!(
                                "\n[LEARNING] {} MHz {} {} MHz | Charge: {:.1}%",
                                governor.current_freq(),
                                direction,
                                new_freq,
                                load
                            );
                            set_gpu_frequency(&mut pp_file, new_freq)?;
                            stats.set_frequency(new_freq, load);
                        }
                    }
                }
                GovernorMode::Locked => {
                    // Vérifier le confort
                    if !governor.check_comfort(load) {
                        println!("\n⚠ Confort dégradé en mode LOCKED, passage en mode ADJUSTING\n");
                        governor.switch_to_adjusting();
                    }

                    // Réévaluation périodique?
                    if governor.mode_start.elapsed()
                        >= Duration::from_secs(REEVALUATION_INTERVAL_SECS)
                    {
                        println!("\n🔄 Réévaluation périodique, passage en mode LEARNING\n");
                        governor.switch_to_learning();
                    }
                }
                GovernorMode::Adjusting => {
                    // Ajuster la fréquence
                    if let Some(new_freq) = governor.try_adjust(&stats) {
                        let direction = if new_freq > governor.current_freq() {
                            "↑"
                        } else {
                            "↓"
                        };
                        println!(
                            "\n[ADJUSTING] {} MHz {} {} MHz | Charge: {:.1}%",
                            governor.current_freq(),
                            direction,
                            new_freq,
                            load
                        );
                        set_gpu_frequency(&mut pp_file, new_freq)?;
                        stats.set_frequency(new_freq, load);
                    }

                    // Retour en locked si confort revenu?
                    if governor.check_comfort(load)
                        && governor.mode_start.elapsed() >= Duration::from_secs(30)
                    {
                        if let Some((optimal_freq, _)) = stats.get_optimal_frequency() {
                            println!(
                                "\n✓ Confort restauré, retour en mode LOCKED à {} MHz\n",
                                optimal_freq
                            );
                            governor.switch_to_locked(optimal_freq);
                            set_gpu_frequency(&mut pp_file, optimal_freq)?;
                            stats.set_frequency(optimal_freq, load);
                        }
                    }
                }
            }
        }

        // Affichage temps réel
        if last_display.elapsed() >= Duration::from_millis(500) {
            let load = load_monitor.load_percent();
            let freq = governor.current_freq();
            let mode_str = match governor.mode {
                GovernorMode::Learning => "LEARNING",
                GovernorMode::Locked => "LOCKED  ",
                GovernorMode::Adjusting => "ADJUSTING",
            };
            eprint!(
                "\r[{}] Charge: {:5.1}% | Fréq: {:4} MHz | Échantillons: {}",
                mode_str, load, freq, sample_count
            );
            last_display = Instant::now();
        }

        // Sauvegarde périodique des stats (toutes les 60 secondes)
        if last_save.elapsed() >= Duration::from_secs(60) {
            if let Err(e) = stats.save_to_file(&stats_path) {
                eprintln!("\n⚠ Erreur sauvegarde stats: {}", e);
            }
            last_save = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}
