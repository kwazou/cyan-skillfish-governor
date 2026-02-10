use cyan_skillfish_governor::gpu_sensor::GpuSensor;
use std::env;
use std::process;

fn print_usage() {
    println!("GPU Sensor Daemon - Expose la charge GPU comme sonde système");
    println!();
    println!("Usage:");
    println!("  gpu_sensor_daemon [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --path <path>       Chemin du fichier sensor (défaut: /run/gpu-sensor/load)");
    println!("  --interval <ms>     Intervalle de mise à jour en ms (défaut: 1000)");
    println!("  --help              Afficher cette aide");
    println!();
    println!("Exemples:");
    println!("  sudo gpu_sensor_daemon");
    println!("  sudo gpu_sensor_daemon --path /tmp/gpu-load --interval 500");
    println!();
    println!("Le daemon expose la charge GPU dans deux formats:");
    println!("  1. Fichier simple: <path> contient le pourcentage (ex: 45.32)");
    println!("  2. Format hwmon: /run/gpu-sensor/hwmon/ contient les fichiers compatibles");
    println!();
    println!("Pour CoolerControl, configurez une source personnalisée pointant vers:");
    println!("  - Fichier simple: /run/gpu-sensor/load");
    println!("  - Format hwmon: /run/gpu-sensor/hwmon/load1_input");
}

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut sensor_path = "/run/gpu-sensor/load".to_string();
    let mut interval_ms = 1000u64;

    // Parser les arguments
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--path" => {
                if i + 1 < args.len() {
                    sensor_path = args[i + 1].clone();
                    i += 1;
                } else {
                    eprintln!("❌ Erreur: --path requiert un argument");
                    process::exit(1);
                }
            }
            "--interval" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse() {
                        Ok(val) => interval_ms = val,
                        Err(_) => {
                            eprintln!("❌ Erreur: intervalle invalide");
                            process::exit(1);
                        }
                    }
                    i += 1;
                } else {
                    eprintln!("❌ Erreur: --interval requiert un argument");
                    process::exit(1);
                }
            }
            _ => {
                eprintln!("❌ Argument inconnu: {}", args[i]);
                eprintln!();
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    // Vérifier les permissions (nécessite généralement root pour écrire dans /run)
    if sensor_path.starts_with("/run") {
        // Note: écriture dans /run nécessite généralement les privilèges root
        println!("ℹ️  Écriture dans /run (peut nécessiter les privilèges root)");
    }

    // Créer et lancer le sensor
    let mut sensor = GpuSensor::new(&sensor_path, interval_ms);

    // Gérer Ctrl+C proprement
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        println!("\n🛑 Arrêt du daemon...");
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    })
    .expect("Erreur lors de la configuration du handler Ctrl+C");

    // Lancer le daemon
    if let Err(e) = sensor.run_daemon() {
        eprintln!("❌ Erreur fatale: {}", e);
        process::exit(1);
    }
}
