use crate::constants::*;
use crate::profile_db::ProcessProfile;
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// Statistiques pour une fréquence donnée
#[derive(Debug, Clone)]
pub struct FrequencyStats {
    time_spent: Duration,
    load_samples: Vec<f32>,
    last_entry: Option<Instant>,
}

impl FrequencyStats {
    pub fn new(_freq_mhz: u16) -> Self {
        Self {
            time_spent: Duration::ZERO,
            load_samples: Vec::new(),
            last_entry: None,
        }
    }

    pub fn enter(&mut self) {
        self.last_entry = Some(Instant::now());
    }

    pub fn exit(&mut self) {
        if let Some(entry_time) = self.last_entry.take() {
            self.time_spent += entry_time.elapsed();
        }
    }

    pub fn add_load_sample(&mut self, load: f32) {
        self.load_samples.push(load);
    }

    pub fn average_load(&self) -> f32 {
        if self.load_samples.is_empty() {
            return 0.0;
        }
        self.load_samples.iter().sum::<f32>() / self.load_samples.len() as f32
    }

    pub fn comfort_score(&self) -> f32 {
        let avg_load = self.average_load();
        let ideal_load = 70.0;
        let deviation = (avg_load - ideal_load).abs();
        (100.0 - deviation).max(0.0)
    }
}

/// Collecteur de statistiques temporaires pendant l'apprentissage
pub struct LearningStats {
    stats: BTreeMap<u16, FrequencyStats>,
    current_freq: Option<u16>,
}

impl LearningStats {
    pub fn new() -> Self {
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

    pub fn set_frequency(&mut self, freq: u16, load: f32) {
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

    pub fn add_load_sample(&mut self, load: f32) {
        if let Some(freq) = self.current_freq {
            if let Some(stat) = self.stats.get_mut(&freq) {
                stat.add_load_sample(load);
            }
        }
    }

    pub fn get_best_frequency(&self) -> Option<(u16, f32, usize)> {
        self.stats
            .iter()
            .filter(|(_, s)| s.load_samples.len() >= 5)
            .max_by(|(_, a), (_, b)| a.comfort_score().partial_cmp(&b.comfort_score()).unwrap())
            .map(|(freq, stat)| (*freq, stat.comfort_score(), stat.load_samples.len()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GovernorMode {
    Idle,         // Pas de process GPU actif
    Applied,      // Fréquence connue appliquée
    Learning,     // Apprentissage d'un nouveau process
    Reevaluating, // Réévaluation d'un process connu
}

/// Gouverneur adaptatif par processus
pub struct ProcessAwareGovernor {
    pub current_freq: u16,
    pub mode: GovernorMode,
    pub mode_start: Instant,
    last_change: Instant,
    pub load_history: VecDeque<f32>,
    pub learning_stats: Option<LearningStats>,
    base_freq_for_reevaluation: Option<u16>,
}

impl ProcessAwareGovernor {
    pub fn new() -> Self {
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

    pub fn start_learning(&mut self, starting_freq: u16) {
        println!("📚 Mode LEARNING: Apprentissage d'un nouveau processus");
        self.mode = GovernorMode::Learning;
        self.mode_start = Instant::now();
        self.current_freq = starting_freq;
        self.learning_stats = Some(LearningStats::new());
        self.load_history.clear();
    }

    pub fn start_reevaluation(&mut self, base_freq: u16) {
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

    pub fn apply_known_frequency(&mut self, freq: u16) {
        println!("✓ Mode APPLIED: Application fréquence connue {} MHz", freq);
        self.mode = GovernorMode::Applied;
        self.mode_start = Instant::now();
        self.current_freq = freq;
        self.learning_stats = None;
        self.load_history.clear();
    }

    pub fn enter_idle(&mut self) {
        self.mode = GovernorMode::Idle;
        self.mode_start = Instant::now();
        self.current_freq = MIN_FREQ_MHZ;
        self.learning_stats = None;
        self.load_history.clear();
    }

    pub fn add_load_sample(&mut self, load: f32) {
        if self.load_history.len() >= SATURATION_HISTORY_SIZE {
            self.load_history.pop_front();
        }
        self.load_history.push_back(load);

        if let Some(stats) = &mut self.learning_stats {
            stats.add_load_sample(load);
        }
    }

    pub fn average_load(&self) -> f32 {
        if self.load_history.is_empty() {
            return 0.0;
        }
        self.load_history.iter().sum::<f32>() / self.load_history.len() as f32
    }

    pub fn should_increase(&self) -> bool {
        let required_samples = match self.mode {
            GovernorMode::Learning | GovernorMode::Reevaluating => LEARNING_HISTORY_SIZE,
            _ => SATURATION_HISTORY_SIZE,
        };

        self.current_freq < MAX_FREQ_MHZ
            && self.load_history.len() >= required_samples
            && self.average_load() >= HIGH_LOAD_THRESHOLD
    }

    pub fn should_decrease(&self) -> bool {
        let required_samples = match self.mode {
            GovernorMode::Learning | GovernorMode::Reevaluating => LEARNING_HISTORY_SIZE,
            _ => SATURATION_HISTORY_SIZE,
        };

        self.current_freq > MIN_FREQ_MHZ
            && self.load_history.len() >= required_samples
            && self.average_load() <= LOW_LOAD_THRESHOLD
    }

    pub fn try_adjust_learning(&mut self) -> Option<u16> {
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

    pub fn finalize_learning(&mut self) -> Option<ProcessProfile> {
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

    pub fn check_saturation(&self) -> bool {
        // Si on est en mode Applied et que la charge reste haute pendant 60 secondes
        matches!(self.mode, GovernorMode::Applied)
            && self.load_history.len() >= SATURATION_HISTORY_SIZE
            && self.average_load() > HIGH_LOAD_THRESHOLD
    }

    pub fn check_underload(&self) -> bool {
        // Si on est en mode Applied et que la charge reste basse pendant 60 secondes
        matches!(self.mode, GovernorMode::Applied)
            && self.load_history.len() >= SATURATION_HISTORY_SIZE
            && self.average_load() < LOW_LOAD_THRESHOLD
    }
}

impl Default for ProcessAwareGovernor {
    fn default() -> Self {
        Self::new()
    }
}
