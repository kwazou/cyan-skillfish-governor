#!/bin/bash

echo "🧪 Test du GPU Sensor Daemon (sans root)"
echo "========================================"
echo ""
echo "Ce script teste le daemon en écrivant dans /tmp au lieu de /run"
echo ""

# Nettoyer d'éventuels fichiers précédents
rm -rf /tmp/gpu-sensor-test

# Compiler si nécessaire
if [ ! -f "target/debug/gpu_sensor_daemon" ]; then
    echo "📦 Compilation du daemon..."
    cargo build --bin gpu_sensor_daemon
fi

echo "🚀 Démarrage du daemon en arrière-plan..."
echo ""

# Lancer le daemon en arrière-plan
./target/debug/gpu_sensor_daemon --path /tmp/gpu-sensor-test/load --interval 1000 &
DAEMON_PID=$!

echo "✅ Daemon lancé avec PID: $DAEMON_PID"
echo ""

# Fonction de nettoyage
cleanup() {
    echo ""
    echo "🛑 Arrêt du daemon..."
    kill $DAEMON_PID 2>/dev/null || true
    wait $DAEMON_PID 2>/dev/null || true
    echo "✅ Nettoyage terminé"
    exit 0
}

# Capturer Ctrl+C
trap cleanup SIGINT SIGTERM

echo "⏳ Attente de la première mesure (2 secondes)..."
sleep 2

echo "📊 Monitoring de la charge GPU (Ctrl+C pour arrêter):"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Vérifier que les fichiers existent
if [ ! -f "/tmp/gpu-sensor-test/load" ]; then
    echo "❌ Erreur: le fichier /tmp/gpu-sensor-test/load n'a pas été créé"
    echo "   Vérifiez les logs du daemon ci-dessus"
    cleanup
fi

# Boucle d'affichage
COUNT=0
while true; do
    # Lire les valeurs
    if [ -f "/tmp/gpu-sensor-test/load" ]; then
        SIMPLE_LOAD=$(cat /tmp/gpu-sensor-test/load 2>/dev/null || echo "N/A")
    else
        SIMPLE_LOAD="N/A"
    fi
    
    if [ -f "/tmp/gpu-sensor-test/hwmon/load1_input" ]; then
        HWMON_VALUE=$(cat /tmp/gpu-sensor-test/hwmon/load1_input 2>/dev/null || echo "N/A")
        # Convertir de millièmes à pourcentage
        if [ "$HWMON_VALUE" != "N/A" ]; then
            HWMON_LOAD=$(echo "scale=2; $HWMON_VALUE / 1000" | bc)
        else
            HWMON_LOAD="N/A"
        fi
    else
        HWMON_LOAD="N/A"
    fi
    
    # Afficher
    TIMESTAMP=$(date '+%H:%M:%S')
    printf "[%s] GPU Load: %8s%%  |  Hwmon: %8s%%\n" "$TIMESTAMP" "$SIMPLE_LOAD" "$HWMON_LOAD"
    
    # Afficher une ligne d'info toutes les 10 mesures
    COUNT=$((COUNT + 1))
    if [ $((COUNT % 10)) -eq 0 ]; then
        echo "           (Les fichiers sont dans /tmp/gpu-sensor-test/)"
    fi
    
    sleep 1
done
