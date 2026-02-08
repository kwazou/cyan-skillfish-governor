#!/bin/bash
set -e

echo "🔨 Building project..."
cargo build --release

echo "🛑 Stopping service..."
sudo systemctl stop cyan-skillfish-governor.service

echo "📦 Copying binary to /usr/local/bin..."
sudo cp target/release/cyan-skillfish-governor /usr/local/bin/

echo "📝 Copying configuration..."
sudo mkdir -p /etc/cyan-skillfish-governor
sudo cp default-config.toml /etc/cyan-skillfish-governor/config.toml

echo "🔄 Restarting service..."
sudo systemctl start cyan-skillfish-governor.service

echo "✅ Service reloaded successfully!"
echo ""
echo "📊 Service status:"
sudo systemctl status cyan-skillfish-governor.service --no-pager -l

sudo journalctl -u cyan-skillfish-governor.service -f
