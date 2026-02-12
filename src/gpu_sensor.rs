use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Error as IoError, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::thread;
use std::time::Duration;

use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};

// Registre contenant le statut GRBM pour Cyan Skillfish (gfx1013)
const GRBM_STATUS_REG: u32 = 0x2004;
// Bit 31 indique si le GPU est actif
const GUI_ACTIVE_BIT_MASK: u32 = 1 << 31;

/// Structure pour monitorer la charge GPU et l'exposer comme sonde système
pub struct GpuSensor {
    sensor_path: String,
    update_interval: Duration,
    samples: VecDeque<bool>,
    window_size: usize,
    active_count: u32,
    dev_handle: DeviceHandle,
}

impl GpuSensor {
    /// Créer un nouveau capteur GPU
    ///
    /// # Arguments
    /// * `sensor_path` - Chemin où écrire les données du capteur (ex: "/run/gpu-sensor/load")
    /// * `update_interval_ms` - Intervalle de mise à jour en millisecondes
    /// * `window_size` - Nombre d'échantillons pour la moyenne mobile (défaut: 100)
    pub fn new(
        sensor_path: &str,
        update_interval_ms: u64,
        window_size: usize,
    ) -> Result<Self, String> {
        // Location PCI du GPU Cyan Skillfish (Steam Deck)
        let location = BUS_INFO {
            domain: 0,
            bus: 1,
            dev: 0,
            func: 0,
        };

        // Vérifier que c'est bien un GPU Cyan Skillfish
        let sysfs_path = location.get_sysfs_path();
        let vendor = std::fs::read_to_string(sysfs_path.join("vendor"))
            .map_err(|e| format!("Erreur lecture vendor: {}", e))?;
        let device = std::fs::read_to_string(sysfs_path.join("device"))
            .map_err(|e| format!("Erreur lecture device: {}", e))?;

        if !((vendor == "0x1002\n") && (device == "0x13fe\n")) {
            return Err(
                "GPU Cyan Skillfish introuvable à l'emplacement PCI attendu (0000:01:00.0)"
                    .to_string(),
            );
        }

        // Ouvrir le device DRM
        let card = File::open(
            location
                .get_drm_render_path()
                .map_err(|e| format!("Erreur get_drm_render_path: {:?}", e))?,
        )
        .map_err(|e| format!("Erreur ouverture DRM: {}", e))?;

        let (dev_handle, _, _) = DeviceHandle::init(card.as_raw_fd()).map_err(|e| {
            format!(
                "Erreur init DeviceHandle: {:?}",
                IoError::from_raw_os_error(e)
            )
        })?;

        Ok(Self {
            sensor_path: sensor_path.to_string(),
            update_interval: Duration::from_millis(update_interval_ms),
            samples: VecDeque::with_capacity(window_size),
            window_size,
            active_count: 0,
            dev_handle,
        })
    }

    /// Ajouter un échantillon d'activité GPU
    fn add_sample(&mut self, is_active: bool) {
        // Si le buffer est plein, retirer l'échantillon le plus ancien
        if self.samples.len() >= self.window_size {
            if let Some(old_sample) = self.samples.pop_front() {
                if old_sample {
                    self.active_count -= 1;
                }
            }
        }

        // Ajouter le nouvel échantillon
        self.samples.push_back(is_active);
        if is_active {
            self.active_count += 1;
        }
    }

    /// Calculer la charge GPU en pourcentage
    pub fn calculate_gpu_load(&mut self) -> Result<f64, String> {
        // Échantillonner le GPU plusieurs fois pour avoir une mesure précise
        // On prend plusieurs échantillons rapprochés pour remplir la fenêtre
        let sample_interval = Duration::from_micros(2000); // 2ms entre échantillons
        let samples_per_update = 50; // 50 échantillons = 100ms d'échantillonnage

        for _ in 0..samples_per_update {
            // Lire le registre GRBM_STATUS
            let status = self
                .dev_handle
                .read_mm_registers(GRBM_STATUS_REG)
                .map_err(|e| {
                    format!(
                        "Erreur lecture registre: {:?}",
                        IoError::from_raw_os_error(e)
                    )
                })?;

            // Le bit 31 indique si le GPU est actif
            let gpu_active = (status & GUI_ACTIVE_BIT_MASK) != 0;

            // Ajouter l'échantillon
            self.add_sample(gpu_active);

            thread::sleep(sample_interval);
        }

        // Calculer le pourcentage sur la fenêtre complète
        if self.samples.is_empty() {
            return Ok(0.0);
        }

        let load_percent = (self.active_count as f64 / self.samples.len() as f64) * 100.0;
        Ok(load_percent)
    }

    /// Écrire la charge GPU dans le fichier sensor
    pub fn write_sensor_value(&self, load: f64) -> Result<(), String> {
        use std::io::Write;

        // Créer le répertoire parent si nécessaire
        if let Some(parent) = Path::new(&self.sensor_path).parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Erreur création répertoire: {}", e))?;
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
        file.flush().map_err(|e| format!("Erreur flush: {}", e))?;

        // Renommer atomiquement
        fs::rename(&temp_path, &self.sensor_path).map_err(|e| format!("Erreur rename: {}", e))?;

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
        name_file
            .write_all(b"gpu_load\n")
            .map_err(|e| format!("Erreur écriture name: {}", e))?;

        // Écrire la valeur comme input (similaire à temp1_input)
        let mut input_file = File::create(format!("{}/load1_input", hwmon_dir))
            .map_err(|e| format!("Erreur création input: {}", e))?;
        input_file
            .write_all(format!("{}\n", hwmon_value).as_bytes())
            .map_err(|e| format!("Erreur écriture input: {}", e))?;

        // Écrire un label
        let mut label_file = File::create(format!("{}/load1_label", hwmon_dir))
            .map_err(|e| format!("Erreur création label: {}", e))?;
        label_file
            .write_all(b"GPU Load\n")
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
