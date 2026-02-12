#!/bin/bash
# Script d'installation pour set_gpu_freq

set -e

if [ "$EUID" -ne 0 ]; then 
    echo "Ce script nécessite les privilèges root"
    echo "Réessayez avec: sudo $0"
    exit 1
fi

echo "=== Installation de set_gpu_freq ==="

# Compilation
echo "Compilation de set_gpu_freq..."
cargo build --release --bin set_gpu_freq

# Installation du binaire
echo "Installation du binaire dans /usr/local/bin/..."
cp target/release/set_gpu_freq /usr/local/bin/
chmod +x /usr/local/bin/set_gpu_freq

# Installation du script helper
echo "Installation du script helper dans /usr/local/bin/..."
cat > /usr/local/bin/set-gpu-freq << 'EOF'
#!/bin/bash
if [ "$EUID" -ne 0 ]; then 
    echo "Ce script nécessite les privilèges root"
    echo "Réessayez avec: sudo $0 $*"
    exit 1
fi
/usr/local/bin/set_gpu_freq "$@" /etc/cyan-skillfish-governor/config.toml
EOF
chmod +x /usr/local/bin/set-gpu-freq

echo ""
echo "✓ Installation terminée!"
echo ""
echo "Usage:"
echo "  sudo set-gpu-freq <MHz>      - Définir la fréquence GPU"
echo ""
echo "Exemples:"
echo "  sudo set-gpu-freq 800        - GPU à 800 MHz"
echo "  sudo set-gpu-freq 1600       - GPU à 1600 MHz"
echo ""
echo "Note: Pour désactiver le gouverneur dynamique pendant que vous utilisez"
echo "      une fréquence statique, arrêtez le service avec:"
echo "      sudo systemctl stop cyan-skillfish-governor.service"
