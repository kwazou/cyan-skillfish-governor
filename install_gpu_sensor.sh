#!/bin/bash
set -e

echo "🚀 Installation du GPU Sensor Daemon"
echo "===================================="
echo ""

# Vérifier que nous sommes root
if [ "$EUID" -ne 0 ]; then 
    echo "❌ Ce script doit être exécuté avec sudo"
    exit 1
fi

# Compiler le daemon en mode release
echo "📦 Compilation du daemon..."
cargo build --release --bin gpu_sensor_daemon

# Arrêter le service s'il tourne
echo "🛑 Arrêt du service existant (si présent)..."
systemctl stop gpu-sensor.service 2>/dev/null || true

# Copier le binaire
echo "📋 Installation du binaire..."
cp target/release/gpu_sensor_daemon /usr/local/bin/
chmod +x /usr/local/bin/gpu_sensor_daemon

# Copier le fichier service
echo "📋 Installation du service systemd..."
cp gpu-sensor.service /etc/systemd/system/

# Créer le répertoire pour les sensors
echo "📁 Création du répertoire /run/gpu-sensor..."
mkdir -p /run/gpu-sensor
chmod 755 /run/gpu-sensor

# Créer un tmpfiles.d pour recréer le répertoire au boot
echo "📋 Configuration tmpfiles.d..."
cat > /etc/tmpfiles.d/gpu-sensor.conf << 'EOF'
# GPU Sensor daemon directory
d /run/gpu-sensor 0755 root root -
EOF

# Recharger systemd
echo "🔄 Rechargement de systemd..."
systemctl daemon-reload

# Activer et démarrer le service
echo "✅ Activation du service..."
systemctl enable gpu-sensor.service
systemctl start gpu-sensor.service

# Vérifier le statut
echo ""
echo "📊 Statut du service:"
systemctl status gpu-sensor.service --no-pager || true

echo ""
echo "✅ Installation terminée!"
echo ""
echo "Commandes utiles:"
echo "  - Voir les logs: journalctl -u gpu-sensor.service -f"
echo "  - Arrêter: sudo systemctl stop gpu-sensor.service"
echo "  - Redémarrer: sudo systemctl restart gpu-sensor.service"
echo "  - Désactiver: sudo systemctl disable gpu-sensor.service"
echo ""
echo "Fichiers de sortie:"
echo "  - Simple: /run/gpu-sensor/load"
echo "  - Hwmon: /run/gpu-sensor/hwmon/load1_input"
echo ""
echo "Pour tester:"
echo "  watch -n 1 cat /run/gpu-sensor/load"
echo ""
