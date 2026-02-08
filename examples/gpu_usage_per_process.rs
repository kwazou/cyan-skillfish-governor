use std::{
    collections::HashMap,
    fs,
    io::Error as IoError,
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// Informations sur l'utilisation GPU d'un processus
#[derive(Debug, Clone)]
struct ProcessGpuUsage {
    pid: u32,
    name: String,
    cmdline: String,
    // Cycles GPU utilisés par moteur
    _engine_cycles: HashMap<String, u64>,
    // Total des cycles depuis le dernier échantillon
    total_cycles: u64,
}

/// Parse le cmdline d'un processus
fn read_process_cmdline(pid: u32) -> Result<String, IoError> {
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    let cmdline = fs::read_to_string(&cmdline_path)?;

    let cmdline_clean: String = cmdline
        .split('\0')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(cmdline_clean)
}

/// Parse le nom du processus
fn read_process_name(pid: u32) -> Result<String, IoError> {
    let comm_path = format!("/proc/{}/comm", pid);
    let name = fs::read_to_string(&comm_path)?.trim().to_string();
    Ok(name)
}

/// Vérifie si un lien symbolique pointe vers un device DRM
fn is_drm_device(link_path: &Path) -> bool {
    if let Ok(target) = fs::read_link(link_path) {
        let target_str = target.to_string_lossy();
        return target_str.contains("/dev/dri/");
    }
    false
}

/// Parse les informations fdinfo pour extraire les cycles GPU
fn parse_fdinfo(fdinfo_path: &str) -> Result<HashMap<String, u64>, IoError> {
    let mut cycles = HashMap::new();
    let content = fs::read_to_string(fdinfo_path)?;

    for line in content.lines() {
        // Chercher les lignes du type "drm-engine-<engine>: <cycles> ns"
        if line.starts_with("drm-engine-") || line.starts_with("drm-cycles-") {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 {
                let engine = parts[0].trim().to_string();
                let value_str = parts[1].trim().split_whitespace().next().unwrap_or("0");
                if let Ok(value) = value_str.parse::<u64>() {
                    cycles.insert(engine, value);
                }
            }
        }
    }

    Ok(cycles)
}

/// Collecte les statistiques GPU pour tous les processus
fn collect_gpu_stats() -> Result<Vec<ProcessGpuUsage>, IoError> {
    let mut processes = Vec::new();

    let proc_entries = fs::read_dir("/proc")?;

    for entry in proc_entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        if !path.is_dir() || !file_name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let pid: u32 = match file_name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let fd_dir = path.join("fd");
        let fd_entries = match fs::read_dir(&fd_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        let mut all_engine_cycles = HashMap::new();
        let mut has_drm = false;

        for fd_entry in fd_entries {
            let fd_entry = match fd_entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let fd_path = fd_entry.path();

            if !is_drm_device(&fd_path) {
                continue;
            }

            has_drm = true;

            // Lire le fdinfo correspondant
            let fd_num = fd_entry.file_name().to_string_lossy().to_string();
            let fdinfo_path = format!("/proc/{}/fdinfo/{}", pid, fd_num);

            if let Ok(cycles) = parse_fdinfo(&fdinfo_path) {
                for (engine, value) in cycles {
                    *all_engine_cycles.entry(engine).or_insert(0) += value;
                }
            }
        }

        if has_drm {
            let total_cycles: u64 = all_engine_cycles.values().sum();

            let name = read_process_name(pid).unwrap_or_else(|_| "unknown".to_string());
            let cmdline = read_process_cmdline(pid).unwrap_or_else(|_| "".to_string());

            processes.push(ProcessGpuUsage {
                pid,
                name,
                cmdline,
                _engine_cycles: all_engine_cycles,
                total_cycles,
            });
        }
    }

    Ok(processes)
}

/// Calcule le delta d'utilisation entre deux échantillons
fn calculate_usage_delta(
    prev: &HashMap<u32, ProcessGpuUsage>,
    current: &HashMap<u32, ProcessGpuUsage>,
    elapsed: Duration,
) -> Vec<(ProcessGpuUsage, f64)> {
    let mut deltas = Vec::new();

    for (pid, curr_stats) in current {
        if let Some(prev_stats) = prev.get(pid) {
            let cycle_delta = curr_stats
                .total_cycles
                .saturating_sub(prev_stats.total_cycles);

            // Convertir les cycles (en nanosecondes) en pourcentage d'utilisation
            let elapsed_ns = elapsed.as_nanos() as f64;
            let usage_percent = if elapsed_ns > 0.0 {
                (cycle_delta as f64 / elapsed_ns) * 100.0
            } else {
                0.0
            };

            deltas.push((curr_stats.clone(), usage_percent));
        } else {
            // Nouveau processus
            deltas.push((curr_stats.clone(), 0.0));
        }
    }

    // Trier par utilisation décroissante
    deltas.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    deltas
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Surveillance de l'utilisation GPU par processus...\n");
    println!("Collecte des statistiques initiales...");

    let mut prev_stats: HashMap<u32, ProcessGpuUsage> = HashMap::new();
    let mut prev_time = Instant::now();

    // Premier échantillon
    for proc in collect_gpu_stats()? {
        prev_stats.insert(proc.pid, proc);
    }

    thread::sleep(Duration::from_secs(1));

    println!(
        "\n{:>7} | {:>20} | {:>10} | {}",
        "PID", "Nom", "GPU %", "Ligne de commande"
    );
    println!("{:-<7}-+-{:-<20}-+-{:-<10}-+-{:-<50}", "", "", "", "");

    loop {
        let mut current_stats: HashMap<u32, ProcessGpuUsage> = HashMap::new();
        let current_time = Instant::now();
        let elapsed = current_time.duration_since(prev_time);

        for proc in collect_gpu_stats()? {
            current_stats.insert(proc.pid, proc);
        }

        let usage_deltas = calculate_usage_delta(&prev_stats, &current_stats, elapsed);

        // Effacer l'écran (ANSI escape code)
        print!("\x1B[2J\x1B[H");

        println!(
            "🎮 Utilisation GPU par processus - Actualisation: {:.1}s\n",
            elapsed.as_secs_f32()
        );

        println!(
            "{:>7} | {:>20} | {:>10} | {}",
            "PID", "Nom", "GPU %", "Ligne de commande"
        );
        println!("{:-<7}-+-{:-<20}-+-{:-<10}-+-{:-<50}", "", "", "", "");

        let mut total_usage = 0.0;
        let mut displayed = 0;

        for (proc, usage) in &usage_deltas {
            // N'afficher que les processus avec une utilisation significative
            if *usage > 0.1 || displayed < 10 {
                let cmdline_short = if proc.cmdline.len() > 60 {
                    format!("{}...", &proc.cmdline[..57])
                } else {
                    proc.cmdline.clone()
                };

                let name_short = if proc.name.len() > 20 {
                    format!("{}...", &proc.name[..17])
                } else {
                    proc.name.clone()
                };

                // Créer une barre de progression
                let bar_width = 10;
                let filled = ((usage / 100.0).min(1.0) * bar_width as f64) as usize;
                let bar = "█".repeat(filled) + &"░".repeat(bar_width - filled);

                println!(
                    "{:>7} | {:>20} | {:>9.2}% | {} {}",
                    proc.pid, name_short, usage, bar, cmdline_short
                );

                total_usage += usage;
                displayed += 1;
            }
        }

        if displayed == 0 {
            println!("Aucune activité GPU détectée");
        }

        println!("\n📊 Utilisation totale (somme): {:.2}%", total_usage);
        println!("💡 Note: La somme peut dépasser 100% (multi-threading GPU)");
        println!("\nAppuyez sur Ctrl+C pour quitter...");

        prev_stats = current_stats;
        prev_time = current_time;

        thread::sleep(Duration::from_secs(1));
    }
}
