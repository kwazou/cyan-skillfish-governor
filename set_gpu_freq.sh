#!/bin/bash
# Script pour définir statiquement la fréquence GPU
# Usage: ./set_gpu_freq.sh <frequency_mhz>

if [ "$EUID" -ne 0 ]; then 
    echo "Ce script nécessite les privilèges root"
    echo "Réessayez avec: sudo $0 $*"
    exit 1
fi

BINARY="/usr/local/bin/set_gpu_freq"
CONFIG="/etc/cyan-skillfish-governor/config.toml"

# Si le binaire n'est pas installé, utilisez la version dans target/release
if [ ! -f "$BINARY" ]; then
    BINARY="./target/release/set_gpu_freq"
    if [ ! -f "$BINARY" ]; then
        echo "Erreur: Le binaire set_gpu_freq n'est pas installé"
        echo "Compilez d'abord avec: cargo build --release"
        exit 1
    fi
fi

# Exécuter la commande
$BINARY "$@" "$CONFIG"
