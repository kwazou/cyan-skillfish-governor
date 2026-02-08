use crate::gpu_info::{is_drm_device, parse_fdinfo_cycles};
use std::io::Error as IoError;

/// Liste de processus à exclure (desktop, utilitaires, etc.)
pub const EXCLUDED_PROCESSES: &[&str] = &[
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

/// Informations sur un processus utilisant le GPU
#[derive(Debug, Clone)]
pub struct GpuProcess {
    pub _pid: u32,
    pub name: String,
    pub total_cycles: u64,
}

/// Vérifie si un chemin/nom de processus correspond à un processus exclu
pub fn is_excluded_process(name: &str) -> bool {
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

/// Extrait le nom du jeu depuis un chemin Steam
/// Cherche "steamapps" dans le path et retourne le dossier qui suit "common"
pub fn extract_steam_game_name(path: &str) -> Option<String> {
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
pub fn read_process_name(pid: u32) -> Result<String, IoError> {
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

/// Collecte les statistiques GPU pour tous les processus
pub fn collect_gpu_processes() -> Vec<GpuProcess> {
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
