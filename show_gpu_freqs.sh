#!/bin/bash
# Script pour afficher les fréquences GPU disponibles

CONFIG="/etc/cyan-skillfish-governor/config.toml"

if [ ! -f "$CONFIG" ]; then
    CONFIG="./default-config.toml"
    if [ ! -f "$CONFIG" ]; then
        echo "Erreur: Fichier de configuration introuvable"
        exit 1
    fi
fi

echo "=== Fréquences GPU disponibles ==="
echo ""
echo "Points sûrs définis dans $CONFIG :"
echo ""

# Parser le fichier TOML pour extraire les safe-points
awk '
BEGIN { in_safe_points = 0; max_freq = 0; min_freq = 999999; }
/^\[\[safe-points\]\]/ { in_safe_points = 1; freq = 0; volt = 0; next; }
/^\[\[/ { if (in_safe_points) in_safe_points = 0; }
in_safe_points && /^frequency/ { 
    gsub(/[^0-9]/, "", $3); 
    freq = $3; 
    if (freq > max_freq) max_freq = freq;
    if (freq < min_freq) min_freq = freq;
}
in_safe_points && /^voltage/ { 
    gsub(/[^0-9]/, "", $3); 
    volt = $3; 
    if (freq > 0 && volt > 0) {
        printf "  %4d MHz @ %4d mV\n", freq, volt;
    }
}
END {
    print "";
    print "Plage recommandée:";
    printf "  Minimum: %d MHz\n", min_freq;
    printf "  Maximum: %d MHz\n", max_freq;
    print "";
    print "Usage:";
    print "  sudo set-gpu-freq <fréquence>";
    print "";
    print "Exemple:";
    printf "  sudo set-gpu-freq %d\n", int((min_freq + max_freq) / 2);
}
' "$CONFIG"

# Afficher aussi l'état actuel si possible
if [ -f /sys/class/drm/card1/device/pp_dpm_sclk ]; then
    echo ""
    echo "Fréquence actuelle:"
    grep '\*' /sys/class/drm/card1/device/pp_dpm_sclk 2>/dev/null | sed 's/^/  /' || echo "  Non disponible"
fi
