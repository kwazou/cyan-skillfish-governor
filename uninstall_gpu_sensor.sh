#!/bin/bash
set -e

echo "🗑️  Désinstallation du GPU Sensor Daemon"
echo "========================================"
echo ""

# Vérifier que nous sommes root
if [ "$EUID" -ne 0 ]; then 
    echo "❌ Ce script doit être exécuté avec sudo"
    exit 1
fi

# Arrêter et désactiver le service
echo "🛑 Arrêt et désactivation du service..."
systemctl stop gpu-sensor.service 2>/dev/null || true
systemctl disable gpu-sensor.service 2>/dev/null || true

# Supprimer les fichiers
echo "🗑️  Suppression des fichiers..."
rm -f /usr/local/bin/gpu_sensor_daemon
rm -f /etc/systemd/system/gpu-sensor.service
rm -f /etc/tmpfiles.d/gpu-sensor.conf
rm -rf /run/gpu-sensor

# Recharger systemd
echo "🔄 Rechargement de systemd..."
systemctl daemon-reload

echo ""
echo "✅ Désinstallation terminée!"
echo ""
