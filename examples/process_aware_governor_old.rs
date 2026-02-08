use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Error as IoError, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
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

// Durée phase d'apprentissage pour un nouveau process (2 minutes)
const LEARNING_DURATION_SECS: u64 = 120;
// Durée minimum avant de considérer un process comme "stable"
const PROCESS_STABILITY_SECS: u64 = 10;
// Taille de l'historique pour ajustements rapides en apprentissage (200 échantillons = 2s)
const LEARNING_HISTORY_SIZE: usize = 200;
// Taille de l'historique pour détection de saturation (60s à 10ms = 6000 échantillons)
const SATURATION_HISTORY_SIZE: usize = 6000;
// Intervalle minimum entre les mises à jour du monitoring de processus (en secondes)
// Nécessaire pour avoir des calculs de % GPU stables
const PROCESS_UPDATE_INTERVAL_SECS: f64 = 1.0;
// Seuil minimum d'utilisation GPU pour considérer un processus actif (en pourcentage)
// Les jeux utilisent typiquement > 5%, les apps desktop < 1%
const MIN_GPU_USAGE_PERCENT: f64 = 5.0;
// Ratio minimum pour forcer changement vers un autre process (ex: 2.0 = 2x plus gourmand)
const PROCESS_SWITCH_RATIO: f64 = 2.0;

/// Liste de processus à exclure (desktop, utilitaires, etc.)
const EXCLUDED_PROCESSES: &[&str] = &[
    "kwin_wayland",
    "kwin",
    "Xwayland",
    "ksmserver",
    "plasmashell",
    "kaccess",
    "plasma",
    "steam",
    "steamwebhelper",
    "Discord",
    "code",
    "electron",
    "chrome",
    "firefox",
    "chromium",
    "gnome-shell",
    "mutter",
    "xfwm4",
    "marco",
    "coolercontrol",
    "systemsettings",
];

/// Vérifie si un chemin/nom de processus correspond à un processus exclu
fn is_excluded_process(name: &str) -> bool {
    // Extraire le nom du fichier si c'est un chemin
    let basename = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);

    // Vérifier uniquement l'égalité exacte sur le basename
    // (ne pas utiliser contains sur le chemin complet pour éviter les faux positifs)
    EXCLUDED_PROCESSES
        .iter()
        .any(|&excluded| basename == excluded)
}

/// Informations sur un processus utilisant le GPU
#[derive(Debug, Clone)]
struct GpuProcess {
    _pid: u32,
    name: String,
    total_cycles: u64,
}

/// Extrait le nom du jeu depuis un chemin Steam
/// Cherche "steamapps" dans le path et retourne le dossier qui suit "common"
fn extract_steam_game_name(path: &str) -> Option<String> {
    let path_obj = std::path::Path::new(path);
    let mut found_common = false;

    for component in path_obj.components() {
        if let Some(name) = component.as_os_str().to_str() {
            if found_common && name != "." && name != ".." {
                return Some(name.to_string());
            }
            if name == "common" {
                found_common = true;
            }
        }
    }
    None
}

/// Parse le nom d'un processus de manière intelligente
/// Pour les jeux Wine/Proton, essaie d'extraire le nom du jeu depuis cmdline ou cwd
/// Sinon utilise le chemin complet de l'exécutable
fn read_process_name(pid: u32) -> Result<String, IoError> {
    // D'abord essayer de lire cmdline pour les jeux Wine/Proton
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    if let Ok(cmdline_bytes) = std::fs::read(&cmdline_path) {
        let cmdline = String::from_utf8_lossy(&cmdline_bytes);
        // Les arguments sont séparés par des null bytes
        let args: Vec<&str> = cmdline.split('\0').filter(|s| !s.is_empty()).collect();

        // Chercher un .exe dans les arguments (typique pour Wine/Proton)
        for arg in &args {
            if arg.ends_with(".exe") {
                // Essayer d'extraire le nom du jeu depuis le chemin Steam
                if let Some(game_name) = extract_steam_game_name(arg) {
                    let exe_path = std::path::Path::new(arg);
                    if let Some(exe_name) = exe_path.file_stem() {
                        return Ok(format!("{}/{}", game_name, exe_name.to_string_lossy()));
                    }
                    return Ok(game_name);
                }

                // Sinon extraire le nom du exe et son dossier parent
                let exe_path = std::path::Path::new(arg);
                if let Some(parent) = exe_path.parent() {
                    if let Some(game_dir) = parent.file_name() {
                        if let Some(exe_name) = exe_path.file_stem() {
                            return Ok(format!(
                                "{}/{}",
                                game_dir.to_string_lossy(),
                                exe_name.to_string_lossy()
                            ));
                        }
                    }
                }
                // Sinon juste le nom du exe
                if let Some(exe_name) = exe_path.file_stem() {
                    return Ok(exe_name.to_string_lossy().to_string());
                }
            }
        }
    }

    // Si pas de .exe trouvé, essayer le répertoire de travail (cwd)
    let cwd_path = format!("/proc/{}/cwd", pid);
    if let Ok(cwd_link) = std::fs::read_link(&cwd_path) {
        let cwd_str = cwd_link.to_string_lossy();

        // Essayer d'extraire le nom du jeu Steam depuis le cwd
        if let Some(game_name) = extract_steam_game_name(&cwd_str) {
            let exe_path_str = format!("/proc/{}/exe", pid);
            if let Ok(exe_link) = std::fs::read_link(&exe_path_str) {
                if let Some(exe_name) = exe_link.file_name() {
                    let exe_name_str = exe_name.to_string_lossy();
                    if exe_name_str.contains("wine") || exe_name_str.contains("proton") {
                        return Ok(format!("{}/{}", game_name, exe_name_str));
                    }
                }
            }
            return Ok(game_name);
        }

        // Sinon utiliser juste le dernier dossier du cwd
        let exe_path_str = format!("/proc/{}/exe", pid);
        if let Ok(exe_link) = std::fs::read_link(&exe_path_str) {
            let exe_name = exe_link
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            if exe_name.contains("wine") || exe_name.contains("proton") {
                if let Some(game_dir) = cwd_link.file_name() {
                    return Ok(format!("{}/{}", game_dir.to_string_lossy(), exe_name));
                }
            }
        }
    }

    // Fallback: chemin complet de l'exécutable
    let exe_path = format!("/proc/{}/exe", pid);
    if let Ok(exe_link) = std::fs::read_link(&exe_path) {
        let path_str = exe_link.to_string_lossy().to_string();
        let clean_path = path_str.split(" (").next().unwrap_or(&path_str).to_string();
        if !clean_path.is_empty() {
            return Ok(clean_path);
        }
    }

    // Dernier fallback: /proc/{pid}/comm
    let comm_path = format!("/proc/{}/comm", pid);
    let name = std::fs::read_to_string(&comm_path)?.trim().to_string();
    Ok(name)
}

/// Vérifie si un lien symbolique pointe vers un device DRM
fn is_drm_device(link_path: &Path) -> bool {
    if let Ok(target) = std::fs::read_link(link_path) {
        let target_str = target.to_string_lossy();
        return target_str.contains("/dev/dri/");
    }
    false
}

/// Parse les cycles GPU depuis fdinfo
fn parse_fdinfo_cycles(fdinfo_path: &str) -> u64 {
    let Ok(content) = std::fs::read_to_string(fdinfo_path) else {
        return 0;
    };

    let mut total = 0u64;
    for line in content.lines() {
        if line.starts_with("drm-engine-") || line.starts_with("drm-cycles-") {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 {
                let value_str = parts[1].trim().split_whitespace().next().unwrap_or("0");
                if let Ok(value) = value_str.parse::<u64>() {
                    total += value;
                }
            }
        }
    }
    total
}

/// Collecte les statistiques GPU pour tous les processus
fn collect_gpu_processes() -> Vec<GpuProcess> {
    let mut processes = Vec::new();

    let Ok(proc_entries) = std::fs::read_dir("/proc") else {
        return processes;
    };

    for entry in proc_entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        if !path.is_dir() || !file_name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let Ok(pid) = file_name_str.parse::<u32>() else {
            continue;
        };

        let fd_dir = path.join("fd");
        let Ok(fd_entries) = std::fs::read_dir(&fd_dir) else {
            continue;
        };

        let mut total_cycles = 0u64;
        let mut has_drm = false;

        for fd_entry in fd_entries.flatten() {
            let fd_path = fd_entry.path();

            if !is_drm_device(&fd_path) {
                continue;
            }

            has_drm = true;
            let fd_num = fd_entry.file_name().to_string_lossy().to_string();
            let fdinfo_path = format!("/proc/{}/fdinfo/{}", pid, fd_num);
            total_cycles += parse_fdinfo_cycles(&fdinfo_path);
        }

        if has_drm && total_cycles > 0 {
            if let Ok(name) = read_process_name(pid) {
                processes.push(GpuProcess {
                    _pid: pid,
                    name,
                    total_cycles,
                });
            }
        }
    }

    processes
}

/// Moniteur de processus GPU
struct ProcessMonitor {
    current_process: Option<String>,
    process_start: Option<Instant>,
    last_cycles: HashMap<String, u64>,
    last_update: Instant,
    debug_mode: bool,
    current_process_usage_percent: f64, // Pourcentage GPU actuel du processus en cours
}

impl ProcessMonitor {
    fn new() -> Self {
        Self {
            current_process: None,
            process_start: None,
            last_cycles: HashMap::new(),
            last_update: Instant::now(),
            debug_mode: false,
            current_process_usage_percent: 0.0,
        }
    }

    fn update(&mut self) -> Option<String> {
        let elapsed_since_last = self.last_update.elapsed();

        // Ne mettre à jour que si suffisamment de temps s'est écoulé
        if elapsed_since_last.as_secs_f64() < PROCESS_UPDATE_INTERVAL_SECS {
            return self.current_process.clone();
        }

        let processes = collect_gpu_processes();
        self.last_update = Instant::now();

        if processes.is_empty() {
            self.current_process = None;
            self.process_start = None;
            return None;
        }

        // Calculer le delta de cycles pour chaque process
        let mut deltas: Vec<(String, u64, f64)> = Vec::new();

        for proc in &processes {
            let last = self.last_cycles.get(&proc.name).copied().unwrap_or(0);
            let delta = proc.total_cycles.saturating_sub(last);

            // Calculer le pourcentage d'utilisation GPU
            let elapsed_ns = elapsed_since_last.as_nanos() as f64;
            let usage_percent = if elapsed_ns > 0.0 {
                (delta as f64 / elapsed_ns) * 100.0
            } else {
                0.0
            };

            deltas.push((proc.name.clone(), delta, usage_percent));
            self.last_cycles
                .insert(proc.name.clone(), proc.total_cycles);
        }

        // Filtrer les processus avec utilisation GPU significative ET non exclus
        let active_processes: Vec<_> = deltas
            .iter()
            .filter(|(name, _, usage_percent)| {
                *usage_percent >= MIN_GPU_USAGE_PERCENT && !is_excluded_process(name)
            })
            .collect();

        // Si aucun processus actif, rester sur MIN_FREQ
        if active_processes.is_empty() {
            if self.current_process.is_some() {
                println!(
                    "\n💤 Aucun processus avec utilisation GPU > {:.1}%",
                    MIN_GPU_USAGE_PERCENT
                );
                self.current_process = None;
                self.process_start = None;
                self.current_process_usage_percent = 0.0;
            }
            return None;
        }

        // Trouver le process avec l'utilisation GPU la plus élevée parmi les actifs
        if let Some((dominant_process, _, dominant_usage)) = active_processes
            .iter()
            .max_by(|(_, _, usage_a), (_, _, usage_b)| usage_a.partial_cmp(usage_b).unwrap())
        {
            // Vérifier si on doit changer de processus
            let should_change = if let Some(current) = &self.current_process {
                // Cas 1: Le processus dominant est différent
                if *current != **dominant_process {
                    // Si le nouveau process est significativement plus gourmand, changer
                    let current_usage = deltas
                        .iter()
                        .find(|(name, _, _)| name == current)
                        .map(|(_, _, usage)| *usage)
                        .unwrap_or(0.0);

                    if self.debug_mode {
                        println!(
                            "[DEBUG] Comparaison: {} ({:.2}% GPU) vs {} ({:.2}% GPU), ratio: {:.2}x",
                            current,
                            current_usage,
                            dominant_process,
                            dominant_usage,
                            if current_usage > 0.0 {
                                dominant_usage / current_usage
                            } else {
                                999.0
                            }
                        );
                    }

                    // Changer si le nouveau est PROCESS_SWITCH_RATIO fois plus actif
                    current_usage == 0.0
                        || (dominant_usage / current_usage.max(0.1)) >= PROCESS_SWITCH_RATIO
                } else {
                    false
                }
            } else {
                // Pas de processus actuel, prendre le dominant
                true
            };

            if should_change {
                self.current_process = Some((*dominant_process).clone());
                self.process_start = Some(Instant::now());
                self.current_process_usage_percent = *dominant_usage;
                return Some((*dominant_process).clone());
            } else {
                // Pas de changement, mais mettre à jour l'utilisation du processus actuel
                self.current_process_usage_percent = *dominant_usage;
            }
        }

        self.current_process.clone()
    }

    fn is_process_stable(&self) -> bool {
        self.process_start.map_or(false, |start| {
            start.elapsed() >= Duration::from_secs(PROCESS_STABILITY_SECS)
        })
    }
}

/// Profil d'un processus
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ProcessProfile {
    name: String,
    optimal_freq: u16,
    comfort_score: f32,
    samples_count: usize,
}

impl ProcessProfile {
    fn new(name: String, freq: u16, comfort: f32, samples: usize) -> Self {
        Self {
            name,
            optimal_freq: freq,
            comfort_score: comfort,
            samples_count: samples,
        }
    }
}

/// Base de données de profils par processus
struct ProcessDatabase {
    profiles: HashMap<String, ProcessProfile>,
    db_path: PathBuf,
}

impl ProcessDatabase {
    fn new() -> Self {
        let mut path = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        path.push("cyan-skillfish-governor");
        std::fs::create_dir_all(&path).ok();
        path.push("process_profiles.json");

        let mut db = Self {
            profiles: HashMap::new(),
            db_path: path,
        };

        db.load();
        db
    }

    fn load(&mut self) {
        if self.db_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&self.db_path) {
                if let Ok(profiles) = serde_json::from_str(&content) {
                    self.profiles = profiles;
                    println!("📚 {} profils de processus chargés", self.profiles.len());
                }
            }
        }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.profiles) {
            let _ = std::fs::write(&self.db_path, json);
        }
    }

    fn get(&self, process_name: &str) -> Option<&ProcessProfile> {
        self.profiles.get(process_name)
    }

    fn set(&mut self, profile: ProcessProfile) {
        println!(
            "💾 Sauvegarde profil: {} → {} MHz (confort: {:.1}/100)",
            profile.name, profile.optimal_freq, profile.comfort_score
        );
        self.profiles.insert(profile.name.clone(), profile);
        self.save();
    }

    fn print_summary(&self) {
        println!("=== BASE DE DONNÉES JEUX/PROCESSUS ===");
        for (name, profile) in &self.profiles {
            println!(
                "  🎮 {} → {} MHz (confort: {:.1}/100, {} échantillons)",
                name, profile.optimal_freq, profile.comfort_score, profile.samples_count
            );
        }
        println!();
    }
}

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
#[derive(Debug, Clone)]
struct FrequencyStats {
    time_spent: Duration,
    load_samples: Vec<f32>,
    last_entry: Option<Instant>,
}

impl FrequencyStats {
    fn new(_freq_mhz: u16) -> Self {
        Self {
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
        let avg_load = self.average_load();
        let ideal_load = 70.0;
        let deviation = (avg_load - ideal_load).abs();
        (100.0 - deviation).max(0.0)
    }
}

/// Collecteur de statistiques temporaires pendant l'apprentissage
struct LearningStats {
    stats: BTreeMap<u16, FrequencyStats>,
    current_freq: Option<u16>,
}

impl LearningStats {
    fn new() -> Self {
        let mut stats = BTreeMap::new();
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
        if let Some(prev_freq) = self.current_freq {
            if let Some(stat) = self.stats.get_mut(&prev_freq) {
                stat.exit();
            }
        }

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

    fn get_best_frequency(&self) -> Option<(u16, f32, usize)> {
        self.stats
            .iter()
            .filter(|(_, s)| s.load_samples.len() >= 5)
            .max_by(|(_, a), (_, b)| a.comfort_score().partial_cmp(&b.comfort_score()).unwrap())
            .map(|(freq, stat)| (*freq, stat.comfort_score(), stat.load_samples.len()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GovernorMode {
    Idle,         // Pas de process GPU actif
    Applied,      // Fréquence connue appliquée
    Learning,     // Apprentissage d'un nouveau process
    Reevaluating, // Réévaluation d'un process connu
}

/// Gouverneur adaptatif par processus
struct ProcessAwareGovernor {
    current_freq: u16,
    mode: GovernorMode,
    mode_start: Instant,
    last_change: Instant,
    load_history: VecDeque<f32>,
    learning_stats: Option<LearningStats>,
    base_freq_for_reevaluation: Option<u16>,
}

impl ProcessAwareGovernor {
    fn new() -> Self {
        Self {
            current_freq: MIN_FREQ_MHZ,
            mode: GovernorMode::Idle,
            mode_start: Instant::now(),
            last_change: Instant::now(),
            load_history: VecDeque::with_capacity(SATURATION_HISTORY_SIZE),
            learning_stats: None,
            base_freq_for_reevaluation: None,
        }
    }

    fn start_learning(&mut self, starting_freq: u16) {
        println!("📚 Mode LEARNING: Apprentissage d'un nouveau processus");
        self.mode = GovernorMode::Learning;
        self.mode_start = Instant::now();
        self.current_freq = starting_freq;
        self.learning_stats = Some(LearningStats::new());
        self.load_history.clear();
    }

    fn start_reevaluation(&mut self, base_freq: u16) {
        println!(
            "🔄 Mode RÉEVALUATION: Redémarrage depuis {} MHz (référence connue)",
            base_freq
        );
        println!(
            "   Ajustement par palier de {} MHz selon la charge",
            FREQ_STEP_MHZ
        );
        self.mode = GovernorMode::Reevaluating;
        self.mode_start = Instant::now();
        self.current_freq = base_freq;
        self.base_freq_for_reevaluation = Some(base_freq);
        self.learning_stats = Some(LearningStats::new());
        self.load_history.clear();
    }

    fn apply_known_frequency(&mut self, freq: u16) {
        println!("✓ Mode APPLIED: Application fréquence connue {} MHz", freq);
        self.mode = GovernorMode::Applied;
        self.mode_start = Instant::now();
        self.current_freq = freq;
        self.learning_stats = None;
        self.load_history.clear();
    }

    fn enter_idle(&mut self) {
        self.mode = GovernorMode::Idle;
        self.mode_start = Instant::now();
        self.current_freq = MIN_FREQ_MHZ;
        self.learning_stats = None;
        self.load_history.clear();
    }

    fn add_load_sample(&mut self, load: f32) {
        if self.load_history.len() >= SATURATION_HISTORY_SIZE {
            self.load_history.pop_front();
        }
        self.load_history.push_back(load);

        if let Some(stats) = &mut self.learning_stats {
            stats.add_load_sample(load);
        }
    }

    fn average_load(&self) -> f32 {
        if self.load_history.is_empty() {
            return 0.0;
        }
        self.load_history.iter().sum::<f32>() / self.load_history.len() as f32
    }

    fn should_increase(&self) -> bool {
        let required_samples = match self.mode {
            GovernorMode::Learning | GovernorMode::Reevaluating => LEARNING_HISTORY_SIZE,
            _ => SATURATION_HISTORY_SIZE,
        };

        self.current_freq < MAX_FREQ_MHZ
            && self.load_history.len() >= required_samples
            && self.average_load() >= HIGH_LOAD_THRESHOLD
    }

    fn should_decrease(&self) -> bool {
        let required_samples = match self.mode {
            GovernorMode::Learning | GovernorMode::Reevaluating => LEARNING_HISTORY_SIZE,
            _ => SATURATION_HISTORY_SIZE,
        };

        self.current_freq > MIN_FREQ_MHZ
            && self.load_history.len() >= required_samples
            && self.average_load() <= LOW_LOAD_THRESHOLD
    }

    fn try_adjust_learning(&mut self) -> Option<u16> {
        if self.last_change.elapsed() < Duration::from_secs(MIN_CHANGE_INTERVAL_SECS) {
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
            Some(new_freq)
        } else {
            None
        }
    }

    fn finalize_learning(&mut self) -> Option<ProcessProfile> {
        let stats = self.learning_stats.as_ref()?;
        let (best_freq, comfort, samples) = stats.get_best_frequency()?;

        println!(
            "\n✓ Apprentissage terminé: {} MHz (confort: {:.1}/100, {} échantillons)",
            best_freq, comfort, samples
        );

        Some(ProcessProfile::new(
            String::new(), // Le nom sera rempli par l'appelant
            best_freq,
            comfort,
            samples,
        ))
    }

    fn check_saturation(&self) -> bool {
        // Si on est en mode Applied et que la charge reste haute pendant 60 secondes
        matches!(self.mode, GovernorMode::Applied)
            && self.load_history.len() >= SATURATION_HISTORY_SIZE
            && self.average_load() > HIGH_LOAD_THRESHOLD
    }

    fn check_underload(&self) -> bool {
        // Si on est en mode Applied et que la charge reste basse pendant 60 secondes
        matches!(self.mode, GovernorMode::Applied)
            && self.load_history.len() >= SATURATION_HISTORY_SIZE
            && self.average_load() < LOW_LOAD_THRESHOLD
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Governor GPU par Processus (Base de données par Jeu) ===\n");
    println!("🎮 Chaque jeu aura sa fréquence optimale apprise et sauvegardée");
    println!("🔄 Réévaluations automatiques si config graphique change");
    println!(
        "💤 Processus desktop/inactifs ignorés (seuil: {:.1}% GPU)",
        MIN_GPU_USAGE_PERCENT
    );
    println!(
        "🚫 {} processus exclus automatiquement (steam, Discord, desktop, etc.)",
        EXCLUDED_PROCESSES.len()
    );
    println!(
        "⚡ Changement auto vers process {}x plus gourmand",
        PROCESS_SWITCH_RATIO
    );
    println!(
        "🕒 Mise à jour monitoring: chaque {:.1}s\n",
        PROCESS_UPDATE_INTERVAL_SECS
    );

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

    let mut db = ProcessDatabase::new();
    if !db.profiles.is_empty() {
        println!("💾 Base de données chargée:");
        db.print_summary();
    } else {
        println!("🆕 Aucun profil existant, création nouvelle base de données\n");
    }

    let mut process_monitor = ProcessMonitor::new();
    // Activer le debug par défaut pour voir ce qui se passe
    process_monitor.debug_mode = true;
    // Possibilité de désactiver avec DEBUG_GPU_PROCESSES=0
    if std::env::var("DEBUG_GPU_PROCESSES").as_deref() == Ok("0") {
        process_monitor.debug_mode = false;
    }
    if process_monitor.debug_mode {
        println!("🔍 Mode debug activé (désactiver avec DEBUG_GPU_PROCESSES=0)\n");
    }
    let mut load_monitor = GpuLoadMonitor::new(SAMPLE_WINDOW_SIZE);
    let mut governor = ProcessAwareGovernor::new();

    let mut last_display = Instant::now();
    let mut sample_count = 0u64;
    let mut current_tracked_process: Option<String> = None;
    let mut previous_tracked_process: Option<String> = None;
    let mut process_start_time: Option<Instant> = None;

    set_gpu_frequency(&mut pp_file, MIN_FREQ_MHZ)?;

    println!("🚀 Monitoring démarré... (Ctrl+C pour arrêter)\n");

    loop {
        // Lecture de l'activité GPU
        let grbm_status = dev_handle
            .read_mm_registers(GRBM_STATUS_REG)
            .map_err(|e| format!("Échec lecture registre GPU: erreur {}", e))?;
        let is_active = (grbm_status & GUI_ACTIVE_BIT_MASK) != 0;

        load_monitor.add_sample(is_active);
        sample_count += 1;

        // Détection du processus principal
        let detected_process = process_monitor.update();

        // Détection de changement de processus
        if detected_process.as_deref() != current_tracked_process.as_deref() {
            if let Some(ref new_process) = detected_process {
                let usage_percent = process_monitor.current_process_usage_percent;
                println!(
                    "\n🔄 Détection nouveau processus GPU: '{}' ({:.2}% GPU)",
                    new_process, usage_percent
                );

                // Sauvegarder le profil du processus précédent si en apprentissage
                if matches!(
                    governor.mode,
                    GovernorMode::Learning | GovernorMode::Reevaluating
                ) {
                    if let Some(ref old_process) = current_tracked_process {
                        println!(
                            "   Sauvegarde profil de '{}' (apprentissage interrompu)",
                            old_process
                        );
                        if let Some(mut profile) = governor.finalize_learning() {
                            profile.name = old_process.clone();
                            db.set(profile);
                        }
                    }
                }

                // Charger ou démarrer apprentissage pour le nouveau processus
                if let Some(profile) = db.get(new_process) {
                    println!(
                        "   ✓ Profil connu trouvé: {} MHz (confort: {:.1}/100, {} échantillons)",
                        profile.optimal_freq, profile.comfort_score, profile.samples_count
                    );
                    println!("   Application de la fréquence optimale connue");
                    governor.apply_known_frequency(profile.optimal_freq);
                    set_gpu_frequency(&mut pp_file, profile.optimal_freq)?;
                } else {
                    println!(
                        "   ⚠ Processus inconnu, lancement apprentissage ({} secondes)",
                        LEARNING_DURATION_SECS
                    );
                    governor.start_learning(MIN_FREQ_MHZ);
                    set_gpu_frequency(&mut pp_file, MIN_FREQ_MHZ)?;
                }

                current_tracked_process = Some(new_process.clone());
            } else {
                // Plus de processus GPU actif (ou seulement des processus inactifs)
                if current_tracked_process.is_some() {
                    println!("\n💤 Aucune activité GPU significative (processus desktop ignorés)");
                    governor.enter_idle();
                    set_gpu_frequency(&mut pp_file, MIN_FREQ_MHZ)?;
                    previous_tracked_process = current_tracked_process.clone();
                    current_tracked_process = None;
                    process_start_time = None;
                }
            }
        }

        if load_monitor.is_full() {
            let load = load_monitor.load_percent();
            governor.add_load_sample(load);

            // Mise à jour des stats d'apprentissage
            if let Some(stats) = &mut governor.learning_stats {
                stats.set_frequency(governor.current_freq, load);
            }

            match governor.mode {
                GovernorMode::Idle => {
                    // Rien à faire
                }
                GovernorMode::Applied => {
                    // Vérifier si saturation
                    if governor.check_saturation() && process_monitor.is_process_stable() {
                        if let Some(ref process_name) = current_tracked_process {
                            if let Some(profile) = db.get(process_name) {
                                println!(
                                    "\n⚠ SURCHARGE DÉTECTÉE: Charge > {:.0}% pendant 60s (moyenne: {:.1}%)",
                                    HIGH_LOAD_THRESHOLD,
                                    governor.average_load()
                                );
                                println!(
                                    "   La config graphique a peut-être changé, augmentation par palier de {} MHz",
                                    FREQ_STEP_MHZ
                                );
                                governor.start_reevaluation(profile.optimal_freq);
                            }
                        }
                    }
                    // Vérifier si sous-charge
                    else if governor.check_underload() && process_monitor.is_process_stable() {
                        if let Some(ref process_name) = current_tracked_process {
                            if let Some(profile) = db.get(process_name) {
                                println!(
                                    "\n🔻 SOUS-CHARGE DÉTECTÉE: Charge < {:.0}% pendant 60s (moyenne: {:.1}%)",
                                    LOW_LOAD_THRESHOLD,
                                    governor.average_load()
                                );
                                println!(
                                    "   La config graphique a peut-être changé, réduction par palier de {} MHz",
                                    FREQ_STEP_MHZ
                                );
                                governor.start_reevaluation(profile.optimal_freq);
                            }
                        }
                    }
                }
                GovernorMode::Learning | GovernorMode::Reevaluating => {
                    // Ajustement dynamique pendant l'apprentissage
                    let old_freq = governor.current_freq;
                    if let Some(new_freq) = governor.try_adjust_learning() {
                        set_gpu_frequency(&mut pp_file, new_freq)?;
                        let direction = if new_freq > old_freq { "↑" } else { "↓" };
                        println!(
                            "   [{}] {} MHz {} {} MHz (charge: {:.1}%, palier: ±{} MHz)",
                            if matches!(governor.mode, GovernorMode::Learning) {
                                "LEARNING"
                            } else {
                                "REEVALUAT"
                            },
                            old_freq,
                            direction,
                            new_freq,
                            load,
                            FREQ_STEP_MHZ
                        );
                    }

                    // Vérifier si apprentissage terminé
                    let learning_done = governor.mode_start.elapsed()
                        >= Duration::from_secs(LEARNING_DURATION_SECS);

                    if learning_done && process_monitor.is_process_stable() {
                        if let Some(ref process_name) = current_tracked_process {
                            if let Some(mut profile) = governor.finalize_learning() {
                                profile.name = process_name.clone();
                                println!(
                                    "\n✓ Apprentissage terminé pour '{}': {} MHz optimal",
                                    process_name, profile.optimal_freq
                                );
                                db.set(profile.clone());

                                // Appliquer la fréquence optimale trouvée
                                governor.apply_known_frequency(profile.optimal_freq);
                                set_gpu_frequency(&mut pp_file, profile.optimal_freq)?;
                            }
                        }
                    }
                }
            }
        }

        // Affichage temps réel
        if last_display.elapsed() >= Duration::from_millis(500) {
            let load = load_monitor.load_percent();
            let mode_str = match governor.mode {
                GovernorMode::Idle => "IDLE      ",
                GovernorMode::Applied => "APPLIED   ",
                GovernorMode::Learning => "LEARNING  ",
                GovernorMode::Reevaluating => "REEVALUATE",
            };
            let process_str = current_tracked_process
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("none");
            let prev_str = previous_tracked_process
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("-");
            let age_str = if let Some(start) = process_start_time {
                let secs = start.elapsed().as_secs();
                format!("{}s", secs)
            } else {
                "-".to_string()
            };
            eprint!(
                "\r[{}] {} | Charge: {:5.1}% | Fréq: {:4} MHz | Process: {} (âge: {}) | Prev: {}",
                mode_str, sample_count, load, governor.current_freq, process_str, age_str, prev_str
            );
            last_display = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}
