#!/bin/bash
set -e

echo "=== Installation du Process-Aware Governor ==="
echo ""

# Vérifier que nous sommes dans le bon dossier
if [ ! -f "Cargo.toml" ]; then
    echo "❌ Erreur: Ce script doit être exécuté depuis le dossier cyan-skillfish-governor"
    exit 1
fi

# Compilation en mode release
echo "📦 Compilation en mode release..."
cargo build --example process_aware_governor --release

# Arrêter l'ancien service s'il tourne
echo "🛑 Arrêt de l'ancien service cyan-skillfish-governor (si actif)..."
sudo systemctl stop cyan-skillfish-governor.service 2>/dev/null || true
sudo systemctl disable cyan-skillfish-governor.service 2>/dev/null || true

# Installation du binaire
echo "📥 Installation du binaire dans /usr/local/bin/..."
sudo cp target/release/examples/process_aware_governor /usr/local/bin/process-aware-governor
sudo chmod +x /usr/local/bin/process-aware-governor

# Installation du fichier service
echo "⚙️  Installation du service systemd..."
sudo cp process-aware-governor.service /etc/systemd/system/

# Recharger systemd
echo "🔄 Rechargement de systemd..."
sudo systemctl daemon-reload

# Activer et démarrer le service
echo "🚀 Activation et démarrage du service..."
sudo systemctl enable process-aware-governor.service
sudo systemctl start process-aware-governor.service

echo ""
echo "✅ Installation terminée !"
echo ""
echo "Commandes utiles:"
echo "  - Voir les logs:        sudo journalctl -u process-aware-governor.service -f"
echo "  - Voir le statut:       sudo systemctl status process-aware-governor.service"
echo "  - Arrêter le service:   sudo systemctl stop process-aware-governor.service"
echo "  - Redémarrer:           sudo systemctl restart process-aware-governor.service"
echo "  - Désactiver:           sudo systemctl disable process-aware-governor.service"
echo ""
echo "Base de données des profils: ~/.cache/cyan-skillfish-governor/process_profiles.json"
