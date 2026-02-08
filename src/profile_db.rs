use std::collections::HashMap;
use std::path::PathBuf;

/// Profil d'un processus
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProcessProfile {
    pub name: String,
    pub optimal_freq: u16,
    pub comfort_score: f32,
    pub samples_count: usize,
}

impl ProcessProfile {
    pub fn new(name: String, freq: u16, comfort: f32, samples: usize) -> Self {
        Self {
            name,
            optimal_freq: freq,
            comfort_score: comfort,
            samples_count: samples,
        }
    }
}

/// Base de données de profils par processus
pub struct ProcessDatabase {
    pub profiles: HashMap<String, ProcessProfile>,
    db_path: PathBuf,
}

impl ProcessDatabase {
    pub fn new() -> Self {
        let mut path = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        path.push("cyan-skillfish-governor");
        std::fs::create_dir_all(&path).ok();
        path.push("process_profiles.json");

        let mut db = Self {
            profiles: HashMap::new(),
            db_path: path,
        };

        db.load();
        db
    }

    pub fn load(&mut self) {
        if self.db_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&self.db_path) {
                if let Ok(profiles) = serde_json::from_str(&content) {
                    self.profiles = profiles;
                    println!("📚 {} profils de processus chargés", self.profiles.len());
                }
            }
        }
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.profiles) {
            let _ = std::fs::write(&self.db_path, json);
        }
    }

    pub fn get(&self, process_name: &str) -> Option<&ProcessProfile> {
        self.profiles.get(process_name)
    }

    pub fn set(&mut self, profile: ProcessProfile) {
        println!(
            "💾 Sauvegarde profil: {} → {} MHz (confort: {:.1}/100)",
            profile.name, profile.optimal_freq, profile.comfort_score
        );
        self.profiles.insert(profile.name.clone(), profile);
        self.save();
    }

    pub fn print_summary(&self) {
        println!("=== BASE DE DONNÉES JEUX/PROCESSUS ===");
        for (name, profile) in &self.profiles {
            println!(
                "  🎮 {} → {} MHz (confort: {:.1}/100, {} échantillons)",
                name, profile.optimal_freq, profile.comfort_score, profile.samples_count
            );
        }
        println!();
    }
}

impl Default for ProcessDatabase {
    fn default() -> Self {
        Self::new()
    }
}
