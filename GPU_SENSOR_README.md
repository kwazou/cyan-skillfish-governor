# GPU Sensor Daemon

Un daemon qui expose la charge GPU comme une sonde système, compatible avec CoolerControl et autres outils de monitoring.

## 🎯 Fonctionnalités

- **Monitoring en temps réel** de la charge GPU basé sur les cycles DRM
- **Double format de sortie** :
  - Fichier simple avec pourcentage (ex: `45.32`)
  - Format hwmon compatible avec lm-sensors
- **Faible overhead** : mesures toutes les secondes par défaut
- **Service systemd** : démarrage automatique au boot
- **Compatible** avec CoolerControl, lm-sensors, et scripts personnalisés

## 📦 Installation

### Méthode rapide

```bash
# Compiler et installer
sudo ./install_gpu_sensor.sh
```

### Installation manuelle

```bash
# 1. Compiler
cargo build --release --bin gpu_sensor_daemon

# 2. Installer le binaire
sudo cp target/release/gpu_sensor_daemon /usr/local/bin/
sudo chmod +x /usr/local/bin/gpu_sensor_daemon

# 3. Installer le service systemd
sudo cp gpu-sensor.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable gpu-sensor.service
sudo systemctl start gpu-sensor.service
```

## 🎮 Utilisation

### Démarrage manuel

```bash
# Lancer avec les paramètres par défaut
sudo gpu_sensor_daemon

# Personnaliser le chemin et l'intervalle
sudo gpu_sensor_daemon --path /tmp/gpu-load --interval 500

# Voir l'aide
gpu_sensor_daemon --help
```

### Service systemd

```bash
# Démarrer
sudo systemctl start gpu-sensor.service

# Arrêter
sudo systemctl stop gpu-sensor.service

# Redémarrer
sudo systemctl restart gpu-sensor.service

# Voir les logs
journalctl -u gpu-sensor.service -f

# Voir le statut
systemctl status gpu-sensor.service
```

## 📊 Lecture des valeurs

### Fichier simple

```bash
# Lire la charge actuelle
cat /run/gpu-sensor/load

# Monitorer en continu
watch -n 1 cat /run/gpu-sensor/load

# Utiliser dans un script
GPU_LOAD=$(cat /run/gpu-sensor/load)
echo "Charge GPU: ${GPU_LOAD}%"
```

### Format hwmon

```bash
# Lire la valeur hwmon (en millièmes)
cat /run/gpu-sensor/hwmon/load1_input

# Lire le label
cat /run/gpu-sensor/hwmon/load1_label
```

## 🔧 Intégration avec CoolerControl

### Option 1 : Source personnalisée (recommandé)

1. Ouvrir CoolerControl
2. Aller dans **Settings** → **Custom Sensors**
3. Ajouter une nouvelle source :
   - **Name**: GPU Load
   - **Type**: File
   - **Path**: `/run/gpu-sensor/load`
   - **Label**: GPU Load %
   - **Update interval**: 1000ms

### Option 2 : Hwmon (si supporté)

CoolerControl peut automatiquement détecter les sources hwmon :

- Vérifier dans la liste des capteurs disponibles
- Chercher "GPU Load" ou "load1"

## 🎛️ Configuration

### Modifier l'intervalle de mise à jour

Éditer le service systemd :

```bash
sudo systemctl edit gpu-sensor.service
```

Ajouter :

```ini
[Service]
ExecStart=
ExecStart=/usr/local/bin/gpu_sensor_daemon --interval 500
```

Puis recharger :

```bash
sudo systemctl daemon-reload
sudo systemctl restart gpu-sensor.service
```

### Changer le chemin de sortie

Modifier le fichier `/etc/systemd/system/gpu-sensor.service` :

```ini
ExecStart=/usr/local/bin/gpu_sensor_daemon --path /custom/path/gpu-load
```

Et adapter `ReadWritePaths` en conséquence.

## 📈 Format des données

### Fichier simple (`/run/gpu-sensor/load`)

```
45.32
```

Format : pourcentage avec deux décimales

### Hwmon (`/run/gpu-sensor/hwmon/`)

```
name                 → "gpu_load"
load1_input          → 45320 (valeur en millièmes)
load1_label          → "GPU Load"
```

## 🔍 Dépannage

### Le daemon ne démarre pas

```bash
# Vérifier les logs
journalctl -u gpu-sensor.service -n 50

# Vérifier les permissions
ls -la /run/gpu-sensor/

# Tester manuellement
sudo /usr/local/bin/gpu_sensor_daemon
```

### Valeurs toujours à 0

- Vérifier que votre GPU AMD est supporté
- Vérifier `/proc/*/fdinfo/` pour les entrées DRM
- Vérifier que des processus utilisent le GPU

```bash
# Test rapide
for fd in /proc/*/fdinfo/*; do
    grep -H "drm-cycles\|drm-engine" "$fd" 2>/dev/null
done
```

### Permissions insuffisantes

Pour tester sans root :

```bash
# Utiliser /tmp au lieu de /run
gpu_sensor_daemon --path /tmp/gpu-sensor/load
```

## 🗑️ Désinstallation

```bash
sudo ./uninstall_gpu_sensor.sh
```

Ou manuellement :

```bash
sudo systemctl stop gpu-sensor.service
sudo systemctl disable gpu-sensor.service
sudo rm /usr/local/bin/gpu_sensor_daemon
sudo rm /etc/systemd/system/gpu-sensor.service
sudo rm /etc/tmpfiles.d/gpu-sensor.conf
sudo rm -rf /run/gpu-sensor
sudo systemctl daemon-reload
```

## 🔬 Technique

### Calcul de la charge

Le daemon calcule la charge GPU en :

1. Scannant `/proc/*/fdinfo/*` pour tous les processus
2. Lisant les compteurs `drm-cycles-*` et `drm-engine-*`
3. Calculant le delta depuis la dernière mesure
4. Normalisant par rapport à la fréquence max du GPU

### Fréquence GPU

Par défaut, le daemon suppose une fréquence max de 2.6 GHz (RX 6700 XT). Pour ajuster :

Modifier `src/gpu_sensor.rs` ligne ~88 :

```rust
let gpu_max_freq_hz = 2_600_000_000.0; // Votre fréquence max
```

## 📝 Exemples d'intégration

### Script bash

```bash
#!/bin/bash
GPU_LOAD=$(cat /run/gpu-sensor/load)
if (( $(echo "$GPU_LOAD > 80" | bc -l) )); then
    echo "⚠️  Charge GPU élevée : ${GPU_LOAD}%"
fi
```

### Python

```python
def read_gpu_load():
    with open('/run/gpu-sensor/load', 'r') as f:
        return float(f.read().strip())

load = read_gpu_load()
print(f"GPU Load: {load:.2f}%")
```

### Prometheus exporter

```python
from prometheus_client import Gauge, start_http_server
import time

gpu_load_gauge = Gauge('gpu_load_percent', 'GPU Load Percentage')

def update_metrics():
    with open('/run/gpu-sensor/load', 'r') as f:
        load = float(f.read().strip())
    gpu_load_gauge.set(load)

if __name__ == '__main__':
    start_http_server(8000)
    while True:
        update_metrics()
        time.sleep(1)
```

## 🚀 Prochaines étapes

- [ ] Support multi-GPU
- [ ] Température GPU
- [ ] Fréquence GPU actuelle
- [ ] Consommation énergétique
- [ ] VRAM usage

## 📄 Licence

Même licence que le projet cyan-skillfish-governor (MIT).
