use std::{
    collections::BTreeMap,
    fs::File,
    io::{Error as IoError, ErrorKind, Write},
    os::fd::AsRawFd,
};

use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use toml::Table;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse arguments: program <config> <frequency_mhz>
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <frequency_mhz> [config_file]", args[0]);
        eprintln!("  frequency_mhz: Target GPU frequency in MHz");
        eprintln!(
            "  config_file: Optional path to config.toml (default: /etc/cyan-skillfish-governor/config.toml)"
        );
        eprintln!();
        eprintln!("Example: sudo {} 1000", args[0]);
        std::process::exit(1);
    }

    let target_freq: u16 = args[1]
        .parse()
        .map_err(|_| IoError::new(ErrorKind::InvalidInput, "frequency must be a valid number"))?;

    let config_path = args
        .get(2)
        .map(|s| s.as_str())
        .unwrap_or("/etc/cyan-skillfish-governor/config.toml");

    let config = std::fs::read_to_string(config_path)
        .unwrap_or_else(|_| {
            eprintln!("Warning: Could not read config file, using conservative defaults");
            "".to_string()
        })
        .parse::<Table>()?;

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
        safe_points
    } else {
        println!("Using conservative defaults: 350 MHz @ 700 mV, 2000 MHz @ 1000 mV");
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
    let min_engine_clock = u16::try_from(info.min_engine_clock / 1000)?;
    let max_engine_clock = u16::try_from(info.max_engine_clock / 1000)?;

    let min_freq = *safe_points.first_key_value().unwrap().0;
    let max_freq = *safe_points.last_key_value().unwrap().0;

    // Check if target frequency is in valid range
    if target_freq < min_freq || target_freq > max_freq {
        eprintln!(
            "Error: Frequency {} MHz is outside valid range [{} - {}] MHz",
            target_freq, min_freq, max_freq
        );
        eprintln!(
            "Hardware range: [{} - {}] MHz",
            min_engine_clock, max_engine_clock
        );
        std::process::exit(1);
    }

    // Find appropriate voltage for target frequency
    let voltage = *safe_points
        .range(target_freq..)
        .next()
        .ok_or(IoError::other(
            "No safe voltage found for requested frequency",
        ))?
        .1;

    let mut pp_file = std::fs::OpenOptions::new().write(true).open(
        dev_handle
            .get_sysfs_path()
            .map_err(IoError::from_raw_os_error)?
            .join("pp_od_clk_voltage"),
    )?;

    // Set the frequency and voltage
    pp_file.write_all(format!("vc 0 {} {}", target_freq, voltage).as_bytes())?;
    pp_file.write_all("c".as_bytes())?;

    println!(
        "✓ GPU frequency set to {} MHz @ {} mV",
        target_freq, voltage
    );

    Ok(())
}
