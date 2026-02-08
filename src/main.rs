use std::{
    collections::{BTreeMap, VecDeque},
    fs::File,
    io::{Error as IoError, ErrorKind, Write},
    os::fd::AsRawFd,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use toml::Table;

// cyan_skillfish.gfx1013.mmGRBM_STATUS
const GRBM_STATUS_REG: u32 = 0x2004;
// cyan_skillfish.gfx1013.mmGRBM_STATUS.GUI_ACTIVE (bit 31)
const GUI_ACTIVE_BIT_MASK: u32 = 1 << 31;

/// Structure to calculate GPU statistics with moving average
struct GpuStats {
    samples: VecDeque<bool>,
    window_size: usize,
    active_count: u32,
}

impl GpuStats {
    fn new(window_size: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(window_size),
            window_size,
            active_count: 0,
        }
    }

    fn add_sample(&mut self, is_active: bool) {
        // If buffer is full, remove the oldest sample
        if self.samples.len() >= self.window_size {
            if let Some(old_sample) = self.samples.pop_front() {
                if old_sample {
                    self.active_count -= 1;
                }
            }
        }

        // Add the new sample
        self.samples.push_back(is_active);
        if is_active {
            self.active_count += 1;
        }
    }

    fn gpu_percent(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        (self.active_count as f32 / self.samples.len() as f32) * 100.0
    }
}

/// Structure to manage logging rate limiting (max 1 log per second)
struct LogThrottle {
    last_log: Instant,
    min_interval: Duration,
}

impl LogThrottle {
    fn new(min_interval_secs: u64) -> Self {
        Self {
            last_log: Instant::now() - Duration::from_secs(min_interval_secs),
            min_interval: Duration::from_secs(min_interval_secs),
        }
    }

    fn should_log(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_log) >= self.min_interval {
            self.last_log = now;
            true
        } else {
            false
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = std::env::args()
        .nth(1)
        .map(std::fs::read_to_string)
        .unwrap_or(Ok("".to_string()))?
        .parse::<Table>()?;

    let timing = config.get("timing").and_then(|t| t.as_table());
    let intervals = timing
        .and_then(|t| t.get("intervals"))
        .and_then(|t| t.as_table());
    // us
    let sampling_interval: u16 = intervals
        .and_then(|t| t.get("sample"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u16::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u16::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!("timing.intervals.sample {s}, replaced with the default value of 2 ms");
            2000
        });
    // us
    let adjustment_interval = intervals
        .and_then(|t| t.get("adjust"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            (v >= i64::from(sampling_interval))
                .then_some(v)
                .ok_or("must be at least as high as timing.intervals.sample")
        })
        .and_then(|v| {
            u64::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u64::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.intervals.adjust {s}, replaced with the default of \
                10 * timing.intervals.sample"
            );
            10 * u64::from(sampling_interval)
        });
    // us
    let finetune_interval = intervals
        .and_then(|t| t.get("finetune"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            (v >= i64::from(sampling_interval))
                .then_some(v)
                .ok_or("must be at least as high as timing.intervals.sample")
        })
        .and_then(|v| {
            u64::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u64::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.intervals.finetune {s}, replaced with the default of \
                50_000 * timing.intervals.adjust"
            );
            50_000 * u64::from(sampling_interval)
        });
    // seconds
    let log_interval: u64 = intervals
        .and_then(|t| t.get("log"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| {
            (!v.is_negative())
                .then_some(v)
                .ok_or("must not be negative")
        })
        .and_then(|v| {
            u64::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u64::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!("timing.intervals.log {s}, replaced with the default of 60 second");
            60
        });
    // us - optimization interval
    let optimize_interval: u64 = intervals
        .and_then(|t| t.get("optimize"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| {
            (!v.is_negative())
                .then_some(v)
                .ok_or("must not be negative")
        })
        .and_then(|v| {
            u64::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u64::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.intervals.optimize {s}, replaced with the default of 30 seconds (0 = disabled)"
            );
            30_000_000
        });
    let optimize_enabled = optimize_interval > 0;
    // samples - window size for GPU load moving average
    let window_samples: usize = timing
        .and_then(|t| t.get("window-samples"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            usize::try_from(v)
                .map_err(|_| &*format!("cannot be greater than {}", usize::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!("timing.window-samples {s}, replaced with the default of 100 samples");
            100
        });

    // samples
    let burst_mask = match timing
        .and_then(|t| t.get("burst-samples"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| {
            (!v.is_negative())
                .then_some(v)
                .ok_or("must not be negative")
        }) {
        Err(s) => {
            println!(
                "timing.burst-samples {s}, replaced with the default of \
            48"
            );
            Some(48)
        }
        Ok(0) => None, // 0 = burst disabled
        Ok(v @ 1..64) => Some(!(u64::MAX << v)),
        Ok(64) => Some(u64::MAX),
        Ok(65..) => {
            println!("timing.burst-samples can be at most 64, clamping");
            Some(64)
        }
        Ok(i64::MIN..0) => unreachable!(),
    };

    let ramp_rates = timing
        .and_then(|t| t.get("ramp-rates"))
        .and_then(|t| t.as_table());
    // MHz/ms
    let ramp_rate = ramp_rates
        .and_then(|t| t.get("normal"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            v.is_sign_positive()
                .then_some(v)
                .ok_or("must have positive sign")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "timing.ramp-rates.normal {s}, replaced with the default value of \
                1 MHz/ms"
            );
            1.0
        });
    // MHz/ms
    let ramp_rate_burst = ramp_rates
        .and_then(|t| t.get("burst"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            v.is_sign_positive()
                .then_some(v)
                .ok_or("must have positive sign")
        })
        .map(|v| v as f32)
        .and_then(|v| {
            (v > ramp_rate || burst_mask.is_none()).then_some(v).ok_or(
                "must, if bursting is active, be greater than timing.ramp-rates.normal \
                (if you want to turn bursting off, set timing.burst-samples = 0)",
            )
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.ramp-rates.burst {s}, replaced with the default value of \
                50 * timing.ramp-rates.normal"
            );
            50.0 * ramp_rate
        });

    let freq_threshs = config
        .get("frequency-thresholds")
        .and_then(|t| t.as_table());
    // MHz
    let small_change = freq_threshs
        .and_then(|t| t.get("finetune"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u16::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u16::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "frequency-thresholds.finetune {s}, replaced with the default of \
                10 MHz"
            );
            10
        });
    // MHz
    let significant_change = freq_threshs
        .and_then(|t| t.get("adjust"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u16::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u16::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "frequency-thresholds.adjust {s}, replaced with the default of \
                10 * frequency-thresholds.finetune"
            );
            10 * small_change
        });

    let load_threshs = config.get("load-target").and_then(|t| t.as_table());
    // percentage (0-100)
    let up_thresh = load_threshs
        .and_then(|t| t.get("upper"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            (0.0..=100.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be between 0 and 100")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "load-target.upper {s}, replaced with the default value of \
                90%"
            );
            90.0
        });
    // percentage (0-100)
    let down_thresh = load_threshs
        .and_then(|t| t.get("lower"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            (0.0..=100.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be between 0 and 100")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "load-target.lower {s}, replaced with the default value of \
                upper - 10%"
            );
            (up_thresh - 10.0).max(0.0)
        });
    let down_thresh = if down_thresh > up_thresh {
        println!("load-target.lower can't be greater than load-target.upper, clamping");
        up_thresh
    } else {
        down_thresh
    };

    // MHz, mV
    let safe_points: BTreeMap<u16, u16> = if let Some(array) = config.get("safe-points") {
        let array = array.as_array().ok_or(IoError::new(
            ErrorKind::InvalidInput,
            "safe-points must be an array",
        ))?;
        if array.is_empty() {
            Err(IoError::new(
                ErrorKind::InvalidInput,
                "safe-points must not be empty",
            ))?;
        }
        let mut safe_points = BTreeMap::new();
        for (i, t) in array.iter().enumerate() {
            let t = t.as_table().ok_or_else(|| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!("safe-points[{i}] must be a table"),
                )
            })?;

            // MHz
            let frequency = t
                .get("frequency")
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].frequency must exist"),
                    )
                })?
                .as_integer()
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].frequency must be an integer"),
                    )
                })?;
            let frequency = u16::try_from(frequency).map_err(|_| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "safe-points[{i}].frequency must be between 0 and {} inclusive",
                        u16::MAX
                    ),
                )
            })?;

            // mV
            let voltage = t
                .get("voltage")
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].voltage must exist"),
                    )
                })?
                .as_integer()
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].voltage must be an integer"),
                    )
                })?;
            let voltage = u16::try_from(voltage).map_err(|_| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "safe-points[{i}].voltage must be between 0 and {} inclusive",
                        u16::MAX
                    ),
                )
            })?;

            if safe_points.insert(frequency, voltage).is_some() {
                Err(IoError::new(
                    ErrorKind::InvalidInput,
                    format!("multiple supposedly safe voltages for {frequency} MHz"),
                ))?;
            }
        }
        let mut highest_pair = (0, 0);
        for (frequency, voltage) in &safe_points {
            let pair = (*voltage, *frequency);
            if pair < highest_pair {
                Err(IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "supposedly safe voltage {} mV for {} MHz is higher than \
                        {voltage} mV for {frequency} MHz",
                        highest_pair.0, highest_pair.1,
                    ),
                ))?;
            } else {
                highest_pair = pair;
            }
        }
        safe_points
    } else {
        println!(
            "safe-points undefined, using conservative defaults:\n\
            * 350 MHz @ 700 mV\n\
            * 2000 MHz @ 1000 mV"
        );
        BTreeMap::from([(350, 700), (2000, 1000)])
    };

    let location = BUS_INFO {
        domain: 0,
        bus: 1,
        dev: 0,
        func: 0,
    };
    let sysfs_path = location.get_sysfs_path();
    let vendor = std::fs::read_to_string(sysfs_path.join("vendor"))?;
    let device = std::fs::read_to_string(sysfs_path.join("device"))?;
    if !((vendor == "0x1002\n") && (device == "0x13fe\n")) {
        Err(IoError::other(
            "Cyan Skillfish GPU not found at expected PCI bus location",
        ))?;
    }
    let card = File::open(location.get_drm_render_path()?)?;
    let (dev_handle, _, _) =
        DeviceHandle::init(card.as_raw_fd()).map_err(IoError::from_raw_os_error)?;

    let info = dev_handle
        .device_info()
        .map_err(IoError::from_raw_os_error)?;
    // given in kHz, we need MHz
    let min_engine_clock = info.min_engine_clock / 1000;
    let max_engine_clock = info.max_engine_clock / 1000;
    let mut min_freq = *safe_points.first_key_value().unwrap().0;
    if u64::from(min_freq) < min_engine_clock {
        eprintln!("GPU minimum frequency higher than lowest safe frequency, clamping");
        min_freq = u16::try_from(min_engine_clock)?;
    }
    let mut max_freq = *safe_points.last_key_value().unwrap().0;
    if u64::from(max_freq) > max_engine_clock {
        eprintln!("GPU maximum frequency lower than highest safe frequency, clamping");
        max_freq = u16::try_from(max_engine_clock)?;
    }
    let (min_freq, max_freq) = (min_freq, max_freq);

    let mut pp_file = std::fs::OpenOptions::new().write(true).open(
        dev_handle
            .get_sysfs_path()
            .map_err(IoError::from_raw_os_error)?
            .join("pp_od_clk_voltage"),
    )?;
    let (send, mut recv) = watch::channel(min_freq);

    // Capture variables for thread
    let optimize_enabled_capture = optimize_enabled;
    let optimize_interval_capture = optimize_interval;

    let jh_gov: JoinHandle<Result<(), IoError>> = std::thread::spawn(move || {
        let mut curr_freq = min_freq;
        let mut target_freq = f32::from(min_freq);
        let mut samples: u64 = 0;
        let mut stats = GpuStats::new(window_samples);
        let mut last_adjustment = Instant::now();
        let mut last_finetune = Instant::now();
        let mut last_freq_change = Instant::now();
        let mut log_throttle = LogThrottle::new(log_interval);

        // Stability zone: avoids oscillations between thresholds
        // Between lower and upper, do nothing (target zone)
        // Except if optimization mode enabled: slowly decrease to optimize
        loop {
            let res = dev_handle
                .read_mm_registers(GRBM_STATUS_REG)
                .map_err(IoError::from_raw_os_error)?;
            let gui_busy = (res & GUI_ACTIVE_BIT_MASK) != 0;

            // Radeontop method: counting for percentage
            stats.add_sample(gui_busy);

            // Buffer for burst detection (keeps old method)
            samples <<= 1;
            if gui_busy {
                samples |= 1;
            }

            // GPU percentage calculation (0-100)
            let gpu_percent = stats.gpu_percent();
            let burst = burst_mask
                .map(|mask| samples & mask == mask)
                .unwrap_or(false);

            // Apply frequency changes
            let in_stable_zone = gpu_percent >= down_thresh && gpu_percent <= up_thresh;
            let stable_duration = last_freq_change.elapsed();
            let can_optimize = optimize_enabled_capture
                && in_stable_zone
                && stable_duration >= Duration::from_micros(optimize_interval_capture)
                && gpu_percent < (up_thresh - 2.0); // 2% margin: if already close to target, do nothing

            if burst {
                // Burst: fast ramp up
                target_freq += ramp_rate_burst * f32::from(sampling_interval) / 1000.0;
            } else if gpu_percent > up_thresh {
                // Above upper threshold: ramp up
                target_freq += ramp_rate * f32::from(sampling_interval) / 1000.0;
            } else if gpu_percent < down_thresh {
                // Below lower threshold: ramp down
                target_freq -= ramp_rate * f32::from(sampling_interval) / 1000.0;
            } else if can_optimize {
                // Stable zone AND stable for a long time: optimization
                // Slow decrease (10% of normal speed) to increase load
                target_freq -= ramp_rate * 0.1 * f32::from(sampling_interval) / 1000.0;
            }
            // Otherwise: between down_thresh and up_thresh, do nothing

            target_freq = target_freq.clamp(f32::from(min_freq), f32::from(max_freq));

            let adj_now = last_adjustment.elapsed() >= Duration::from_micros(adjustment_interval);
            if adj_now || burst {
                let target_freq = target_freq as u16;
                let hit_bounds = target_freq != curr_freq
                    && (target_freq == min_freq || target_freq == max_freq);
                let big_change = curr_freq.abs_diff(target_freq) >= significant_change;
                let finetune = (last_finetune.elapsed()
                    >= Duration::from_micros(finetune_interval))
                    && curr_freq.abs_diff(target_freq) >= small_change;
                let burst_up = burst && curr_freq != target_freq;
                if hit_bounds || big_change || finetune || burst_up {
                    // Frequency change logging (rate limited to 1/sec)
                    if log_throttle.should_log() {
                        let direction = if target_freq > curr_freq {
                            "↑"
                        } else if target_freq < curr_freq {
                            "↓"
                        } else {
                            "="
                        };
                        let mut reasons = Vec::new();
                        if burst_up {
                            reasons.push("activity burst detected");
                        }
                        if hit_bounds {
                            if target_freq == min_freq {
                                reasons.push("min limit reached");
                            } else {
                                reasons.push("max limit reached");
                            }
                        }
                        if big_change {
                            reasons.push("significant change");
                        }
                        if finetune {
                            reasons.push("fine adjustment");
                        }
                        if can_optimize && !burst_up && !big_change && !finetune && !hit_bounds {
                            reasons.push("energy optimization");
                        }

                        let reason_str = reasons.join(", ");
                        println!(
                            "[FREQ] {} MHz {} {} MHz | GPU Load: {:.1}% | Reasons: {}",
                            curr_freq, direction, target_freq, gpu_percent, reason_str
                        );
                    }

                    send.send(target_freq);
                    curr_freq = target_freq;
                    last_finetune = Instant::now();
                    last_freq_change = Instant::now();
                }
                last_adjustment = Instant::now();
            }

            std::thread::sleep(Duration::from_micros(u64::from(sampling_interval)));
        }
    });
    let jh_set: JoinHandle<Result<(), IoError>> = std::thread::spawn(move || {
        loop {
            let freq = recv.wait();
            let vol = *safe_points
                .range(freq..)
                .next()
                .ok_or(IoError::other(
                    "tried to set a frequency beyond max safe point",
                ))?
                .1;
            pp_file.write_all(format!("vc 0 {freq} {vol}").as_bytes())?;
            pp_file.write_all("c".as_bytes())?;
        }
    });

    let () = jh_set.join().unwrap()?;
    let () = jh_gov.join().unwrap()?;
    Ok(())
}
