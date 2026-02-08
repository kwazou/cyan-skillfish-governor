use crate::constants::{MIN_GPU_USAGE_PERCENT, PROCESS_STABILITY_SECS, PROCESS_SWITCH_RATIO, PROCESS_UPDATE_INTERVAL_SECS};
use crate::process_detection::{collect_gpu_processes, is_excluded_process};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Moniteur de processus GPU
pub struct ProcessMonitor {
    current_process: Option<String>,
    process_start: Option<Instant>,
    last_cycles: HashMap<String, u64>,
    last_update: Instant,
    pub debug_mode: bool,
    pub current_process_usage_percent: f64, // Pourcentage GPU actuel du processus en cours
}

impl ProcessMonitor {
    pub fn new() -> Self {
        Self {
            current_process: None,
            process_start: None,
            last_cycles: HashMap::new(),
            last_update: Instant::now(),
            debug_mode: false,
            current_process_usage_percent: 0.0,
        }
    }

    pub fn update(&mut self) -> Option<String> {
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

    pub fn is_process_stable(&self) -> bool {
        self.process_start.map_or(false, |start| {
            start.elapsed() >= Duration::from_secs(PROCESS_STABILITY_SECS)
        })
    }
}

impl Default for ProcessMonitor {
    fn default() -> Self {
        Self::new()
    }
}
