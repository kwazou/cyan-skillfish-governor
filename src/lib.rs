// Public modules
pub mod governor;
pub mod gpu_info;
pub mod load_monitor;
pub mod process_detection;
pub mod process_monitor;
pub mod profile_db;

// Re-export constants commonly used
pub mod constants {
    pub const MIN_FREQ_MHZ: u16 = 350;
    pub const MAX_FREQ_MHZ: u16 = 2000;
    pub const FREQ_STEP_MHZ: u16 = 50;

    pub const MIN_VOLTAGE_MV: u16 = 700;
    pub const MAX_VOLTAGE_MV: u16 = 1000;

    pub const HIGH_LOAD_THRESHOLD: f32 = 80.0;
    pub const LOW_LOAD_THRESHOLD: f32 = 40.0;
    pub const SAMPLE_WINDOW_SIZE: usize = 100;
    pub const MIN_CHANGE_INTERVAL_SECS: u64 = 2;

    pub const LEARNING_DURATION_SECS: u64 = 120;
    pub const PROCESS_STABILITY_SECS: u64 = 10;
    pub const LEARNING_HISTORY_SIZE: usize = 200;
    pub const SATURATION_HISTORY_SIZE: usize = 6000;
    pub const PROCESS_UPDATE_INTERVAL_SECS: f64 = 1.0;
    pub const MIN_GPU_USAGE_PERCENT: f64 = 5.0;
    pub const PROCESS_SWITCH_RATIO: f64 = 2.0;
}
