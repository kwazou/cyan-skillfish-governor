# Cyan Skillfish GPU Governor

A dynamic GPU frequency governor for the AMD Cyan Skillfish APU (Steam Deck).

## Features

- **Dynamic frequency scaling**: Continuously monitors GPU load and adjusts frequency to maintain target utilization
- **Burst detection**: Rapid frequency ramp-up when sustained GPU activity is detected
- **Energy optimization**: Automatically reduces frequency during stable low-load periods to save power
- **Radeontop-style load calculation**: Accurate GPU utilization percentage using moving average
- **Configurable thresholds**: Fine-tune behavior with adjustable load targets and ramp rates
- **Safe voltage/frequency management**: Operates within user-defined safe points
- **Static frequency control**: Manual frequency setting tool for testing or fixed performance needs

## Installation

### Prerequisites

- Rust toolchain (for building)
- AMD Cyan Skillfish APU (Steam Deck)
- Root access (for systemd service installation)
- Kernel with custom voltage control support (for frequencies outside default range)

### Quick Install

```bash
./install.sh
```

This script will:

1. Build the project in release mode
2. Install the binary to `/usr/local/bin/`
3. Install the configuration to `/etc/cyan-skillfish-governor/`
4. Install and enable the systemd service
5. Start the service

### Manual Installation

```bash
# Build the project
cargo build --release

# Install binary
sudo cp target/release/cyan-skillfish-governor /usr/local/bin/
sudo chmod +x /usr/local/bin/cyan-skillfish-governor

# Install configuration
sudo mkdir -p /etc/cyan-skillfish-governor
sudo cp default-config.toml /etc/cyan-skillfish-governor/config.toml

# Install systemd service
sudo cp cyan-skillfish-governor.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable cyan-skillfish-governor.service
sudo systemctl start cyan-skillfish-governor.service
```

## Configuration

The governor takes a TOML configuration file path as its only argument (default: `/etc/cyan-skillfish-governor/config.toml`).

### Configuration Keys

#### `timing.intervals` (in microseconds)

- `sample`: How often to sample GPU activity (default: 2000 µs = 2ms, max: 65535)
- `adjust`: Interval for considering frequency adjustments (default: 10 × sample)
- `finetune`: Minimum time since last adjustment before fine-tuning (default: 50000 × sample)
- `log`: Log display interval in seconds (default: 1, set to 0 to disable)
- `optimize`: Stability duration before enabling energy optimization (default: 30000000 µs = 30s, set to 0 to disable)

#### `timing`

- `burst-samples`: Number of consecutive active samples required to trigger burst mode (default: 48, set to 0 to disable)
- `window-samples`: Number of samples for GPU load moving average (default: 100)

#### `timing.ramp-rates` (in MHz/ms)

- `normal`: Frequency ramp rate for normal adjustments (default: 1.0 MHz/ms)
- `burst`: Frequency ramp rate in burst mode (default: 50 × normal)

#### `frequency-thresholds` (in MHz)

- `adjust`: Minimum frequency change for normal adjustments (default: 10 MHz)
- `finetune`: Minimum frequency change for fine-tuning adjustments (default: 100 MHz)

#### `load-target` (percentage 0-100)

- `upper`: GPU load above which frequency increases (default: 90.0%)
- `lower`: GPU load below which frequency decreases (default: 80.0% = upper - 10%)

Between `lower` and `upper` is a stable zone where frequency remains constant (prevents oscillations).

#### `safe-points`

Array of known safe/stable power points. Each entry contains:

- `frequency`: GPU frequency in MHz
- `voltage`: GPU supply voltage in mV

**Note**: Frequencies outside the default range (350-1600 MHz) require a patched kernel with custom voltage control support.

### Example Configuration

See `default-config.toml` for a complete example.

## Usage

### Manual Run

```bash
# Run with default config
sudo cyan-skillfish-governor /etc/cyan-skillfish-governor/config.toml

# Run with custom config
sudo cyan-skillfish-governor /path/to/config.toml
```

### Service Management

```bash
# Check service status
sudo systemctl status cyan-skillfish-governor.service

# View logs
sudo journalctl -u cyan-skillfish-governor.service -f

# Stop service
sudo systemctl stop cyan-skillfish-governor.service

# Restart service
sudo systemctl restart cyan-skillfish-governor.service
```

### Development

```bash
# Build and reload service
./rebuild_and_reload.sh
```

This script rebuilds the project and hot-reloads the systemd service.

## How It Works

1. **GPU Load Monitoring**: Uses the `GRBM_STATUS.GUI_ACTIVE` bit to sample GPU activity at regular intervals
2. **Moving Average**: Calculates GPU utilization percentage over a sliding window of samples
3. **Frequency Adjustment**:
   - **Above upper threshold**: Ramps frequency up at normal rate
   - **Below lower threshold**: Ramps frequency down at normal rate
   - **Burst mode**: Rapid frequency increase when sustained activity detected
   - **Stable zone optimization**: Slow frequency reduction during prolonged stable periods to improve efficiency
4. **Voltage/Frequency Pairing**: Automatically selects safe voltage for the target frequency from configured safe points

## Logging

The governor outputs frequency changes with detailed information:

```
[FREQ] 800 MHz ↑ 1000 MHz | GPU Load: 92.5% | Reasons: significant change
[FREQ] 1000 MHz ↓ 900 MHz | GPU Load: 65.3% | Reasons: energy optimization
[FREQ] 900 MHz ↑ 1800 MHz | GPU Load: 98.1% | Reasons: activity burst detected
```

Log output is rate-limited to the configured interval to avoid spam.

## Static Frequency Control

For testing, benchmarking, or when you need fixed GPU performance, use the `set-gpu-freq` tool:

### Installation

```bash
sudo ./install_set_gpu_freq.sh
```

### Usage

```bash
# Set GPU to 800 MHz
sudo set-gpu-freq 800

# Set GPU to maximum frequency
sudo set-gpu-freq 1600

# Show available frequencies
./show_gpu_freqs.sh
```

**⚠️ Important:** The dynamic governor and static frequency cannot run simultaneously. Stop the governor service first:

```bash
sudo systemctl stop cyan-skillfish-governor.service
sudo set-gpu-freq 1200
```

See [SET_GPU_FREQ_README.md](SET_GPU_FREQ_README.md) for detailed documentation.

## License

See LICENSE file for details.

## Contributing

Contributions are welcome! Please open an issue or pull request on GitHub.
