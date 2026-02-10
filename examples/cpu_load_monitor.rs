use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Structure pour calculer l'utilisation CPU à partir de /proc/stat
struct CpuLoadMonitor {
    prev_idle: u64,
    prev_total: u64,
}

impl CpuLoadMonitor {
    fn new() -> Self {
        Self {
            prev_idle: 0,
            prev_total: 0,
        }
    }

    /// Lit /proc/stat et calcule le pourcentage d'utilisation CPU
    fn read_cpu_usage(&mut self) -> Result<f32, std::io::Error> {
        let stat = std::fs::read_to_string("/proc/stat")?;
        let first_line = stat
            .lines()
            .next()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "Empty /proc/stat"))?;

        // Format de la ligne CPU: cpu  user nice system idle iowait irq softirq steal guest guest_nice
        // On parse tous les nombres
        let nums: Vec<u64> = first_line
            .split_whitespace()
            .skip(1) // Skip le mot "cpu"
            .filter_map(|s| s.parse::<u64>().ok())
            .collect();

        if nums.len() < 5 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid /proc/stat format",
            ));
        }

        // idle = idle + iowait (indices 3 et 4)
        let idle = nums[3] + nums[4];
        // total = somme de toutes les valeurs
        let total: u64 = nums.iter().sum();

        // Si c'est la première lecture, on initialise et retourne 0
        if self.prev_total == 0 {
            self.prev_idle = idle;
            self.prev_total = total;
            return Ok(0.0);
        }

        // Calcul des différences
        let diff_idle = idle.saturating_sub(self.prev_idle);
        let diff_total = total.saturating_sub(self.prev_total);

        // Mise à jour des valeurs précédentes
        self.prev_idle = idle;
        self.prev_total = total;

        // Calcul du pourcentage d'utilisation
        if diff_total == 0 {
            return Ok(0.0);
        }

        let usage = 100.0 * (1.0 - diff_idle as f32 / diff_total as f32);
        Ok(usage.max(0.0).min(100.0))
    }

    /// Lit les informations CPU détaillées pour affichage
    fn read_cpu_detailed(&self) -> Result<String, std::io::Error> {
        let stat = std::fs::read_to_string("/proc/stat")?;
        let first_line = stat.lines().next().unwrap_or("");
        
        let nums: Vec<u64> = first_line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse::<u64>().ok())
            .collect();

        if nums.len() >= 10 {
            Ok(format!(
                "user:{} nice:{} sys:{} idle:{} iowait:{} irq:{} softirq:{} steal:{} guest:{} guest_nice:{}",
                nums[0], nums[1], nums[2], nums[3], nums[4], 
                nums[5], nums[6], nums[7], nums[8], nums[9]
            ))
        } else if nums.len() >= 7 {
            Ok(format!(
                "user:{} nice:{} sys:{} idle:{} iowait:{} irq:{} softirq:{}",
                nums[0], nums[1], nums[2], nums[3], nums[4], nums[5], nums[6]
            ))
        } else {
            Ok(format!("user:{} nice:{} sys:{} idle:{}", 
                nums.get(0).unwrap_or(&0),
                nums.get(1).unwrap_or(&0),
                nums.get(2).unwrap_or(&0),
                nums.get(3).unwrap_or(&0)
            ))
        }
    }
}

/// Lit le nombre de CPUs logiques
fn get_cpu_count() -> usize {
    std::fs::read_to_string("/proc/stat")
        .ok()
        .map(|content| {
            content
                .lines()
                .filter(|line| line.starts_with("cpu") && !line.starts_with("cpu "))
                .count()
        })
        .unwrap_or(1)
}

/// Lit la fréquence actuelle d'un CPU (en kHz)
fn get_cpu_freq(cpu_id: usize) -> Option<u32> {
    std::fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_cur_freq",
        cpu_id
    ))
    .ok()
    .and_then(|s| s.trim().parse().ok())
}

/// Lit la fréquence min/max d'un CPU
fn get_cpu_freq_range(cpu_id: usize) -> (Option<u32>, Option<u32>) {
    let min = std::fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_min_freq",
        cpu_id
    ))
    .ok()
    .and_then(|s| s.trim().parse().ok());

    let max = std::fs::read_to_string(format!(
        "/sys/devices/system/cpu/cpu{}/cpufreq/scaling_max_freq",
        cpu_id
    ))
    .ok()
    .and_then(|s| s.trim().parse().ok());

    (min, max)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== CPU Load Monitor ===");
    println!("Monitoring CPU usage in real-time (Ctrl+C to exit)\n");

    let cpu_count = get_cpu_count();
    println!("Detected {} logical CPU(s)", cpu_count);

    // Afficher la plage de fréquences si disponible
    if let (Some(min), Some(max)) = get_cpu_freq_range(0) {
        println!("CPU0 frequency range: {} MHz - {} MHz", min / 1000, max / 1000);
    }

    println!("\n{:<20} {:<15} {:<20}", "Time", "CPU Usage (%)", "Frequency (MHz)");
    println!("{:-<55}", "");

    let mut monitor = CpuLoadMonitor::new();
    let mut sample_count = 0;
    let mut load_sum = 0.0;

    loop {
        thread::sleep(Duration::from_millis(500));

        match monitor.read_cpu_usage() {
            Ok(usage) => {
                sample_count += 1;
                load_sum += usage;
                
                // Lire la fréquence du premier CPU
                let freq_str = if let Some(freq_khz) = get_cpu_freq(0) {
                    format!("{} MHz", freq_khz / 1000)
                } else {
                    "N/A".to_string()
                };

                // Afficher la charge instantanée et moyenne
                let avg_load = load_sum / sample_count as f32;
                
                // Formater le temps
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let hours = (now / 3600) % 24;
                let minutes = (now / 60) % 60;
                let seconds = now % 60;
                let time_str = format!("{:02}:{:02}:{:02}", hours, minutes, seconds);
                
                println!(
                    "{:<20} {:<7.2} (avg: {:<4.2}) {}",
                    time_str,
                    usage,
                    avg_load,
                    freq_str
                );

                // Tous les 20 échantillons (10 secondes), afficher un résumé
                if sample_count % 20 == 0 {
                    println!("\n--- 10s Summary: avg load = {:.2}% ---\n", avg_load);
                }
            }
            Err(e) => {
                eprintln!("Error reading CPU usage: {}", e);
                break;
            }
        }
    }

    Ok(())
}
