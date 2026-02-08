use cyan_skillfish_governor::constants::*;
use cyan_skillfish_governor::governor::{GovernorMode, ProcessAwareGovernor};
use cyan_skillfish_governor::load_monitor::GpuLoadMonitor;
use cyan_skillfish_governor::process_detection::EXCLUDED_PROCESSES;
use cyan_skillfish_governor::process_monitor::ProcessMonitor;
use cyan_skillfish_governor::profile_db::ProcessDatabase;

use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

const GRBM_STATUS_REG: u32 = 0x2004;
const GUI_ACTIVE_BIT_MASK: u32 = 1 << 31;

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
        .map_err(std::io::Error::from_raw_os_error)?;

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

                previous_tracked_process = current_tracked_process.clone();
                current_tracked_process = Some(new_process.clone());
                process_start_time = Some(Instant::now());
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
