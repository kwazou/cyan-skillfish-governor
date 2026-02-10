use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// Structure pour monitorer la charge GPU et l'exposer comme sonde système
pub struct GpuSensor {
    sensor_path: String,
    update_interval: Duration,
    last_cycles: HashMap<i32, u64>,
    last_update: Instant,
}

impl GpuSensor {
    /// Créer un nouveau capteur GPU
    /// 
    /// # Arguments
    /// * `sensor_path` - Chemin où écrire les données du capteur (ex: "/run/gpu-sensor/load")
    /// * `update_interval_ms` - Intervalle de mise à jour en millisecondes
    pub fn new(sensor_path: &str, update_interval_ms: u64) -> Self {
        Self {
            sensor_path: sensor_path.to_string(),
            update_interval: Duration::from_millis(update_interval_ms),
            last_cycles: HashMap::new(),
            last_update: Instant::now(),
        }
    }

    /// Calculer la charge GPU en pourcentage
    pub fn calculate_gpu_load(&mut self) -> Result<f64, String> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        
        if elapsed < 0.1 {
            // Trop tôt pour avoir une mesure précise
            return Ok(0.0);
        }

        // Lire les cycles GPU actuels pour tous les processus
        let mut current_cycles = HashMap::new();

        // Scanner /proc pour trouver tous les processus avec fdinfo GPU
        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Ok(file_name) = entry.file_name().into_string() {
                    if let Ok(pid) = file_name.parse::<i32>() {
                        let fdinfo_dir = format!("/proc/{}/fdinfo", pid);
                        if let Ok(fd_entries) = fs::read_dir(&fdinfo_dir) {
                            for fd_entry in fd_entries.flatten() {
                                let fdinfo_path = fd_entry.path();
                                let cycles = crate::gpu_info::parse_fdinfo_cycles(
                                    &fdinfo_path.to_string_lossy()
                                );
                                if cycles > 0 {
                                    *current_cycles.entry(pid).or_insert(0u64) += cycles;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Calculer la différence de cycles depuis la dernière mesure
        let mut delta_cycles = 0u64;
        for (pid, cycles) in &current_cycles {
            if let Some(&last) = self.last_cycles.get(pid) {
                if cycles > &last {
                    delta_cycles += cycles - last;
                }
            }
        }

        // Mettre à jour l'état
        self.last_cycles = current_cycles;
        self.last_update = now;

        // Calculer le pourcentage de charge
        // On estime la fréquence max du GPU (en cycles/sec)
        // Pour une RX 6700 XT @ 2.6 GHz, c'est environ 2_600_000_000 cycles/sec
        let gpu_max_freq_hz = 2_600_000_000.0; // À ajuster selon votre GPU
        let max_cycles_possible = gpu_max_freq_hz * elapsed;
        
        let load_percent = if max_cycles_possible > 0.0 {
            ((delta_cycles as f64 / max_cycles_possible) * 100.0).min(100.0)
        } else {
            0.0
        };

        Ok(load_percent)
    }

    /// Écrire la charge GPU dans le fichier sensor
    pub fn write_sensor_value(&self, load: f64) -> Result<(), String> {
        use std::io::Write;
        
        // Créer le répertoire parent si nécessaire
        if let Some(parent) = Path::new(&self.sensor_path).parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Erreur création répertoire: {}", e))?;
        }

        // Écrire de manière atomique via un fichier temporaire
        let temp_path = format!("{}.tmp", self.sensor_path);
        
        // Format: nombre entier pour éviter les problèmes de parsing avec certains outils
        // CoolerControl peut avoir des problèmes avec les décimales selon la locale
        let value_int = load.round() as i32;
        let content = format!("{}\n", value_int);
        
        let mut file = File::create(&temp_path)
            .map_err(|e| format!("Erreur création fichier temporaire: {}", e))?;
        
        file.write_all(content.as_bytes())
            .map_err(|e| format!("Erreur écriture: {}", e))?;
        
        // S'assurer que les données sont écrites sur disque
        file.flush()
            .map_err(|e| format!("Erreur flush: {}", e))?;
        
        // Renommer atomiquement
        fs::rename(&temp_path, &self.sensor_path)
            .map_err(|e| format!("Erreur rename: {}", e))?;

        Ok(())
    }

    /// Écrire également au format hwmon (optionnel)
    pub fn write_hwmon_format(&self, load: f64) -> Result<(), String> {
        // Format hwmon: valeurs entières en millièmes
        // Par exemple, pour la température, 45.5°C = 45500
        // Pour un pourcentage, on peut utiliser 0-100000 (100.000 = 100%)
        let hwmon_value = (load * 1000.0) as i32;
        
        let hwmon_dir = "/run/gpu-sensor/hwmon";
        fs::create_dir_all(hwmon_dir)
            .map_err(|e| format!("Erreur création répertoire hwmon: {}", e))?;

        // Écrire le nom du capteur
        let mut name_file = File::create(format!("{}/name", hwmon_dir))
            .map_err(|e| format!("Erreur création name: {}", e))?;
        name_file.write_all(b"gpu_load\n")
            .map_err(|e| format!("Erreur écriture name: {}", e))?;

        // Écrire la valeur comme input (similaire à temp1_input)
        let mut input_file = File::create(format!("{}/load1_input", hwmon_dir))
            .map_err(|e| format!("Erreur création input: {}", e))?;
        input_file.write_all(format!("{}\n", hwmon_value).as_bytes())
            .map_err(|e| format!("Erreur écriture input: {}", e))?;

        // Écrire un label
        let mut label_file = File::create(format!("{}/load1_label", hwmon_dir))
            .map_err(|e| format!("Erreur création label: {}", e))?;
        label_file.write_all(b"GPU Load\n")
            .map_err(|e| format!("Erreur écriture label: {}", e))?;

        Ok(())
    }

    /// Boucle principale du daemon
    pub fn run_daemon(&mut self) -> Result<(), String> {
        println!("🚀 Démarrage du daemon GPU sensor");
        println!("📍 Fichier de sortie: {}", self.sensor_path);
        println!("⏱️  Intervalle: {:?}", self.update_interval);
        println!();

        // Initialiser les cycles
        let _ = self.calculate_gpu_load();
        thread::sleep(Duration::from_millis(500));

        loop {
            match self.calculate_gpu_load() {
                Ok(load) => {
                    // Écrire la valeur simple
                    if let Err(e) = self.write_sensor_value(load) {
                        eprintln!("❌ Erreur écriture sensor: {}", e);
                    }

                    // Écrire au format hwmon
                    if let Err(e) = self.write_hwmon_format(load) {
                        eprintln!("⚠️  Erreur écriture hwmon: {}", e);
                    } else {
                        println!("📊 GPU Load: {:.2}%", load);
                    }
                }
                Err(e) => {
                    eprintln!("❌ Erreur calcul charge: {}", e);
                }
            }

            thread::sleep(self.update_interval);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sensor_creation() {
        let sensor = GpuSensor::new("/tmp/test-sensor", 1000);
        assert_eq!(sensor.sensor_path, "/tmp/test-sensor");
    }
}
