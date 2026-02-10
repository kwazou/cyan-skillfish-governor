# Intégration avec CoolerControl

Ce guide explique comment intégrer le GPU Sensor avec CoolerControl pour monitorer la charge GPU.

## 📋 Prérequis

1. CoolerControl installé : https://gitlab.com/coolercontrol/coolercontrol
2. GPU Sensor Daemon installé et en cours d'exécution :
   ```bash
   sudo ./install_gpu_sensor.sh
   systemctl status gpu-sensor.service
   ```

## 🎯 Méthode 1 : Source de fichier personnalisée (Recommandée)

### Étape 1 : Vérifier que le daemon fonctionne

```bash
# Vérifier que le fichier existe et contient des données
cat /run/gpu-sensor/load
# Devrait afficher un nombre comme : 45.32
```

### Étape 2 : Configurer CoolerControl

CoolerControl supporte plusieurs méthodes selon la version :

#### Option A : Interface graphique (si disponible)

1. Ouvrir CoolerControl
2. Aller dans **Settings** ou **Configuration**
3. Chercher **Custom Sensors** ou **File Sources**
4. Ajouter une nouvelle source :
   - **Name**: `GPU Load`
   - **Type**: `File` ou `Custom`
   - **Path**: `/run/gpu-sensor/load`
   - **Unit**: `%`
   - **Update Interval**: `1000` ms

#### Option B : Fichier de configuration

Si CoolerControl utilise un fichier TOML pour la configuration :

```bash
# Localiser le fichier de config (peut varier selon l'installation)
# Généralement dans ~/.config/coolercontrol/ ou /etc/coolercontrol/
```

Exemple de configuration à ajouter :

```toml
[[sensors.custom]]
name = "GPU Load"
type = "file"
path = "/run/gpu-sensor/load"
unit = "%"
interval = 1000
min_value = 0.0
max_value = 100.0
```

## 🎯 Méthode 2 : Format hwmon

Si CoolerControl scanne automatiquement les sources hwmon :

### Étape 1 : Vérifier les fichiers hwmon

```bash
ls -la /run/gpu-sensor/hwmon/
# Devrait montrer:
# - name
# - load1_input
# - load1_label
```

### Étape 2 : Lire les valeurs

```bash
cat /run/gpu-sensor/hwmon/name          # → gpu_load
cat /run/gpu-sensor/hwmon/load1_label   # → GPU Load
cat /run/gpu-sensor/hwmon/load1_input   # → 45320 (45.32% en millièmes)
```

### Étape 3 : Configuration CoolerControl

Selon la version de CoolerControl, il peut détecter automatiquement les sources hwmon dans `/run/`.

**Note** : Par défaut, la plupart des outils hwmon scannent `/sys/class/hwmon/`. Notre daemon écrit dans `/run/gpu-sensor/hwmon/` qui nécessite une configuration manuelle.

## 🔧 Configuration avancée

### Créer un lien symbolique vers /sys (Expérimental)

⚠️ **Attention** : Cette méthode nécessite des privilèges root et peut ne pas fonctionner sur tous les systèmes.

```bash
# Créer un répertoire hwmon dans /sys/devices/virtual
sudo mkdir -p /sys/devices/virtual/gpu-sensor
sudo ln -s /run/gpu-sensor/hwmon /sys/devices/virtual/gpu-sensor/hwmon0

# Vérifier
ls -la /sys/devices/virtual/gpu-sensor/hwmon0/
```

Puis éditer le service pour créer ce lien au démarrage :

```bash
sudo systemctl edit gpu-sensor.service
```

Ajouter :

```ini
[Service]
ExecStartPost=/bin/bash -c 'mkdir -p /sys/devices/virtual/gpu-sensor; ln -sf /run/gpu-sensor/hwmon /sys/devices/virtual/gpu-sensor/hwmon0'
ExecStopPost=/bin/bash -c 'rm -rf /sys/devices/virtual/gpu-sensor'
```

## 📊 Exemples de graphiques

### Graphique de charge simple

Dans CoolerControl, créer un graphique avec :

- **Source** : GPU Load (custom sensor)
- **Type** : Line ou Area
- **Range** : 0-100%
- **Color** : Orange ou Rouge

### Alert sur charge élevée

Configurer une alerte :

- **Condition** : GPU Load > 80%
- **Action** : Notification ou ajustement des ventilateurs

## 🐛 Dépannage

### CoolerControl ne voit pas le sensor

1. **Vérifier que le daemon tourne** :

   ```bash
   systemctl status gpu-sensor.service
   ```

2. **Vérifier les permissions** :

   ```bash
   ls -la /run/gpu-sensor/
   # Devrait être lisible (755)
   ```

3. **Vérifier les logs CoolerControl** :
   ```bash
   journalctl -u coolercontrol -f
   ```

### Valeurs incorrectes

1. **Vérifier manuellement** :

   ```bash
   watch -n 1 cat /run/gpu-sensor/load
   ```

2. **Comparer avec d'autres outils** :

   ```bash
   # AMD GPU
   watch -n 1 cat /sys/class/drm/card0/device/gpu_busy_percent

   # radeontop
   radeontop -d - -l 1
   ```

### CoolerControl ne démarre plus

Si vous avez modifié la configuration et que CoolerControl ne démarre plus :

1. **Sauvegarder la config** :

   ```bash
   cp ~/.config/coolercontrol/config.toml ~/.config/coolercontrol/config.toml.bak
   ```

2. **Retirer la configuration GPU Sensor** et redémarrer

## 🔄 Alternative : Script d'intégration

Si CoolerControl n'a pas de support direct pour les fichiers personnalisés, vous pouvez créer un script wrapper :

```bash
#!/bin/bash
# /usr/local/bin/coolercontrol-gpu-load-plugin

# Lire la charge GPU
GPU_LOAD=$(cat /run/gpu-sensor/load 2>/dev/null || echo "0.00")

# Retourner au format attendu par CoolerControl
# (à adapter selon l'API de votre version)
echo "{\"name\": \"GPU Load\", \"value\": $GPU_LOAD, \"unit\": \"%\"}"
```

## 📞 Support

Pour des problèmes spécifiques à CoolerControl :

- Issues CoolerControl : https://gitlab.com/coolercontrol/coolercontrol/-/issues
- Documentation : https://gitlab.com/coolercontrol/coolercontrol/-/wikis/home

Pour des problèmes avec GPU Sensor :

- Vérifier [GPU_SENSOR_README.md](GPU_SENSOR_README.md)
- Vérifier les logs : `journalctl -u gpu-sensor.service -f`

## 🎨 Captures d'écran

### Exemple de configuration

```
┌─────────────────────────────────────────┐
│ CoolerControl - Custom Sensors         │
├─────────────────────────────────────────┤
│ Name:           GPU Load                │
│ Type:           File                    │
│ Path:           /run/gpu-sensor/load    │
│ Unit:           %                       │
│ Min:            0                       │
│ Max:            100                     │
│ Update (ms):    1000                    │
│                                         │
│ ┌─────────┐  ┌──────┐                 │
│ │   Save  │  │ Cancel│                 │
│ └─────────┘  └──────┘                  │
└─────────────────────────────────────────┘
```

### Exemple de graphique

```
GPU Load %
100 ┤                           ╭╮
 90 ┤                          ╭╯╰╮
 80 ┤                      ╭───╯  ╰─╮
 70 ┤                  ╭───╯        ╰─╮
 60 ┤              ╭───╯              ╰─╮
 50 ┤          ╭───╯                    ╰─╮
 40 ┤      ╭───╯                          ╰─
 30 ┤  ╭───╯
 20 ┤──╯
 10 ┤
  0 ┼─────────────────────────────────────
    0s         30s         60s         90s
```

## ✅ Checklist d'installation

- [ ] GPU Sensor Daemon installé et actif
- [ ] Fichier `/run/gpu-sensor/load` créé et mis à jour
- [ ] CoolerControl installé et en cours d'exécution
- [ ] Source personnalisée ajoutée dans CoolerControl
- [ ] Graphique configuré et affichant des données
- [ ] (Optionnel) Alerts configurées pour charge élevée
