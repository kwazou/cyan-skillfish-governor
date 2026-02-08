#!/bin/bash
set -e

echo "=== Désinstallation du Process-Aware Governor ==="
echo ""

# Arrêter et désactiver le service
echo "🛑 Arrêt et désactivation du service..."
sudo systemctl stop process-aware-governor.service 2>/dev/null || true
sudo systemctl disable process-aware-governor.service 2>/dev/null || true

# Supprimer les fichiers
echo "🗑️  Suppression des fichiers..."
sudo rm -f /etc/systemd/system/process-aware-governor.service
sudo rm -f /usr/local/bin/process-aware-governor

# Recharger systemd
echo "🔄 Rechargement de systemd..."
sudo systemctl daemon-reload

echo ""
echo "✅ Désinstallation terminée !"
echo ""
echo "Pour revenir à l'ancien service:"
echo "  sudo systemctl enable cyan-skillfish-governor.service"
echo "  sudo systemctl start cyan-skillfish-governor.service"
echo ""
echo "Note: La base de données des profils est conservée dans:"
echo "      ~/.cache/cyan-skillfish-governor/process_profiles.json"
