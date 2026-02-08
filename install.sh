#!/bin/bash
set -e

echo "🔨 Building project..."
cargo build --release

echo "📦 Installing binary to /usr/local/bin..."
sudo cp target/release/cyan-skillfish-governor /usr/local/bin/
sudo chmod +x /usr/local/bin/cyan-skillfish-governor

echo "📝 Installing configuration..."
sudo mkdir -p /etc/cyan-skillfish-governor
sudo cp default-config.toml /etc/cyan-skillfish-governor/config.toml

echo "🔧 Installing systemd service..."
sudo cp cyan-skillfish-governor.service /etc/systemd/system/

echo "🔄 Reloading systemd daemon..."
sudo systemctl daemon-reload

echo "📌 Enabling service..."
sudo systemctl enable cyan-skillfish-governor.service

echo "▶️  Starting service..."
sudo systemctl start cyan-skillfish-governor.service

echo "✅ Installation completed successfully!"
echo ""
echo "📊 Service status:"
sudo systemctl status cyan-skillfish-governor.service --no-pager -l
