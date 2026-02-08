use std::path::Path;

/// Vérifie si un lien symbolique pointe vers un device DRM
pub fn is_drm_device(link_path: &Path) -> bool {
    if let Ok(target) = std::fs::read_link(link_path) {
        let target_str = target.to_string_lossy();
        return target_str.contains("/dev/dri/");
    }
    false
}

/// Parse les cycles GPU depuis fdinfo
pub fn parse_fdinfo_cycles(fdinfo_path: &str) -> u64 {
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
