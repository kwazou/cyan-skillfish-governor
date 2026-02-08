use std::{collections::HashMap, fs, io::Error as IoError, path::Path};

/// Informations sur un processus utilisant le GPU
#[derive(Debug, Clone)]
struct GpuProcess {
    pid: u32,
    name: String,
    cmdline: String,
    drm_devices: Vec<String>,
}

/// Parse le cmdline d'un processus
fn read_process_cmdline(pid: u32) -> Result<String, IoError> {
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    let cmdline = fs::read_to_string(&cmdline_path)?;

    // Le cmdline contient des null bytes entre les arguments
    let cmdline_clean: String = cmdline
        .split('\0')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    Ok(cmdline_clean)
}

/// Parse le nom du processus (comm)
fn read_process_name(pid: u32) -> Result<String, IoError> {
    let comm_path = format!("/proc/{}/comm", pid);
    let name = fs::read_to_string(&comm_path)?.trim().to_string();
    Ok(name)
}

/// Vérifie si un lien symbolique pointe vers un device DRM/render
fn is_drm_device(link_path: &Path) -> Option<String> {
    if let Ok(target) = fs::read_link(link_path) {
        let target_str = target.to_string_lossy();
        // Chercher les devices DRM : /dev/dri/card*, /dev/dri/renderD*
        if target_str.contains("/dev/dri/") {
            return Some(target_str.to_string());
        }
    }
    None
}

/// Liste tous les processus ayant ouvert des file descriptors vers des devices DRM
fn find_gpu_processes() -> Result<Vec<GpuProcess>, IoError> {
    let mut gpu_processes = Vec::new();

    // Parcourir tous les répertoires /proc/[pid]
    let proc_entries = fs::read_dir("/proc")?;

    for entry in proc_entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // Ne garder que les répertoires avec un PID numérique
        if !path.is_dir() || !file_name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let pid: u32 = match file_name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // Vérifier les file descriptors de ce processus
        let fd_dir = path.join("fd");
        let fd_entries = match fs::read_dir(&fd_dir) {
            Ok(entries) => entries,
            Err(_) => continue, // Pas de permissions ou processus terminé
        };

        let mut drm_devices = Vec::new();

        for fd_entry in fd_entries {
            let fd_entry = match fd_entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if let Some(device) = is_drm_device(&fd_entry.path()) {
                if !drm_devices.contains(&device) {
                    drm_devices.push(device);
                }
            }
        }

        // Si ce processus a des devices DRM ouverts
        if !drm_devices.is_empty() {
            let name = read_process_name(pid).unwrap_or_else(|_| "unknown".to_string());
            let cmdline = read_process_cmdline(pid).unwrap_or_else(|_| "".to_string());

            gpu_processes.push(GpuProcess {
                pid,
                name,
                cmdline,
                drm_devices,
            });
        }
    }

    Ok(gpu_processes)
}

/// Affiche un tableau formaté des processus
fn print_process_table(processes: &[GpuProcess]) {
    if processes.is_empty() {
        println!("Aucun processus utilisant le GPU trouvé.");
        return;
    }

    println!(
        "\n{:>7} | {:>20} | {:>15} | {}",
        "PID", "Nom", "Devices", "Ligne de commande"
    );
    println!("{:-<7}-+-{:-<20}-+-{:-<15}-+-{:-<50}", "", "", "", "");

    for proc in processes {
        let devices_str = proc
            .drm_devices
            .iter()
            .map(|d| {
                // Extraire le nom court du device (ex: card1, renderD128)
                d.split('/').last().unwrap_or(d)
            })
            .collect::<Vec<_>>()
            .join(", ");

        let cmdline_short = if proc.cmdline.len() > 80 {
            format!("{}...", &proc.cmdline[..77])
        } else {
            proc.cmdline.clone()
        };

        println!(
            "{:>7} | {:>20} | {:>15} | {}",
            proc.pid,
            if proc.name.len() > 20 {
                format!("{}...", &proc.name[..17])
            } else {
                proc.name.clone()
            },
            devices_str,
            cmdline_short
        );
    }
}

/// Affiche des statistiques groupées par nom de processus
fn print_statistics(processes: &[GpuProcess]) {
    let mut stats: HashMap<String, u32> = HashMap::new();

    for proc in processes {
        *stats.entry(proc.name.clone()).or_insert(0) += 1;
    }

    if stats.is_empty() {
        return;
    }

    println!("\n📊 Statistiques par type de processus:");
    println!("{:-<40}", "");

    let mut sorted_stats: Vec<_> = stats.iter().collect();
    sorted_stats.sort_by(|a, b| b.1.cmp(a.1));

    for (name, count) in sorted_stats {
        println!("  {:>25} : {} processus", name, count);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Recherche des processus utilisant le GPU...\n");

    let processes = find_gpu_processes()?;

    println!("✅ Total: {} processus trouvé(s)", processes.len());

    print_process_table(&processes);
    print_statistics(&processes);

    println!("\n💡 Info: Cette liste inclut tous les processus ayant ouvert");
    println!("   un file descriptor vers /dev/dri/* (card* ou renderD*)");

    Ok(())
}
