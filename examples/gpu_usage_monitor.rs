use std::{
    collections::VecDeque,
    fs::File,
    io::Error as IoError,
    os::fd::AsRawFd,
    thread,
    time::Duration,
};

use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};

// Registre contenant le statut GRBM pour Cyan Skillfish (gfx1013)
const GRBM_STATUS_REG: u32 = 0x2004;
// Bit 31 indique si le GPU est actif
const GUI_ACTIVE_BIT_MASK: u32 = 1 << 31;

/// Structure pour calculer les statistiques GPU avec moyenne mobile
struct GpuUsageCalculator {
    samples: VecDeque<bool>,
    window_size: usize,
    active_count: u32,
}

impl GpuUsageCalculator {
    fn new(window_size: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(window_size),
            window_size,
            active_count: 0,
        }
    }

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

    fn usage_percent(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        (self.active_count as f32 / self.samples.len() as f32) * 100.0
    }

    fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Démarrage du moniteur d'utilisation GPU...\n");

    // Location PCI du GPU Cyan Skillfish (Steam Deck)
    let location = BUS_INFO {
        domain: 0,
        bus: 1,
        dev: 0,
        func: 0,
    };

    // Vérifier que c'est bien un GPU Cyan Skillfish
    let sysfs_path = location.get_sysfs_path();
    let vendor = std::fs::read_to_string(sysfs_path.join("vendor"))?;
    let device = std::fs::read_to_string(sysfs_path.join("device"))?;
    
    if !((vendor == "0x1002\n") && (device == "0x13fe\n")) {
        return Err(IoError::other(
            "GPU Cyan Skillfish introuvable à l'emplacement PCI attendu (0000:01:00.0)"
        ))?;
    }

    println!("✅ GPU Cyan Skillfish détecté");

    // Ouvrir le device DRM
    let card = File::open(location.get_drm_render_path()?)?;
    let (dev_handle, _, _) =
        DeviceHandle::init(card.as_raw_fd()).map_err(IoError::from_raw_os_error)?;

    let info = dev_handle
        .device_info()
        .map_err(IoError::from_raw_os_error)?;
    
    println!("📊 Infos GPU:");
    println!("   - Fréquence min: {} MHz", info.min_engine_clock / 1000);
    println!("   - Fréquence max: {} MHz", info.max_engine_clock / 1000);
    println!();

    // Paramètres de monitoring
    let sampling_interval = Duration::from_micros(2000); // 2ms entre chaque échantillon
    let window_size = 100; // Fenêtre de 100 échantillons pour la moyenne (= 200ms)
    let mut usage_calc = GpuUsageCalculator::new(window_size);

    println!("🔍 Lecture du statut GPU en temps réel (Ctrl+C pour arrêter)");
    println!("   Intervalle d'échantillonnage: {:?}", sampling_interval);
    println!("   Taille de fenêtre: {} échantillons\n", window_size);
    println!("{:>10} | {:>8} | {:>10}", "Temps (s)", "GPU %", "Échantillons");
    println!("{:-<10}-+-{:-<8}-+-{:-<10}", "", "", "");

    let start = std::time::Instant::now();
    let mut sample_counter = 0u64;

    loop {
        // Lire le registre GRBM_STATUS
        let status = dev_handle
            .read_mm_registers(GRBM_STATUS_REG)
            .map_err(IoError::from_raw_os_error)?;
        
        // Le bit 31 indique si le GPU est actif
        let gpu_active = (status & GUI_ACTIVE_BIT_MASK) != 0;
        
        // Ajouter l'échantillon au calculateur
        usage_calc.add_sample(gpu_active);
        sample_counter += 1;

        // Afficher l'utilisation toutes les 100ms (environ 50 échantillons)
        if sample_counter % 50 == 0 {
            let elapsed = start.elapsed().as_secs_f32();
            let usage = usage_calc.usage_percent();
            
            // Créer une barre de progression visuelle
            let bar_width = 20;
            let filled = ((usage / 100.0) * bar_width as f32) as usize;
            let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);
            
            print!("\r{:>10.1} │ {:>7.2}% │ {:>10} │ {} │",
                elapsed,
                usage,
                usage_calc.sample_count(),
                bar
            );
            std::io::Write::flush(&mut std::io::stdout())?;
        }

        // Attendre avant le prochain échantillon
        thread::sleep(sampling_interval);
    }
}
