#!/bin/bash
set -e

echo "=== Désinstallation de Cyan Skillfish Governor ==="
echo ""

# Arrêter et désactiver le service
echo "🛑 Arrêt et désactivation du service cyan-skillfish-governor..."
sudo systemctl stop cyan-skillfish-governor.service 2>/dev/null || true
sudo systemctl disable cyan-skillfish-governor.service 2>/dev/null || true

# Supprimer les fichiers
echo "🗑️  Suppression des fichiers..."
sudo rm -f /etc/systemd/system/cyan-skillfish-governor.service
sudo rm -f /usr/local/bin/cyan-skillfish-governor
sudo rm -rf /etc/cyan-skillfish-governor/

# Recharger systemd
echo "🔄 Rechargement de systemd..."
sudo systemctl daemon-reload

echo ""
echo "✅ Désinstallation de cyan-skillfish-governor terminée !"
echo ""
echo "Pour installer le nouveau process-aware governor:"
echo "  ./install_process_aware.sh"
