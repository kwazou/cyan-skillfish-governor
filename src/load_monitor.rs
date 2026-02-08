use std::collections::VecDeque;

/// Moniteur de charge GPU avec fenêtre glissante
pub struct GpuLoadMonitor {
    samples: VecDeque<bool>,
    capacity: usize,
}

impl GpuLoadMonitor {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn add_sample(&mut self, is_active: bool) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(is_active);
    }

    pub fn load_percent(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let active_count = self.samples.iter().filter(|&&s| s).count();
        (active_count as f32 / self.samples.len() as f32) * 100.0
    }

    pub fn is_full(&self) -> bool {
        self.samples.len() >= self.capacity
    }
}
