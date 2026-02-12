# Définir statiquement la fréquence GPU

Cet outil permet de définir manuellement et de manière statique la fréquence du GPU AMD Cyan Skillfish (Steam Deck).

## Installation

```bash
sudo ./install_set_gpu_freq.sh
```

Ce script va :

1. Compiler le binaire `set_gpu_freq`
2. Installer le binaire dans `/usr/local/bin/`
3. Créer une commande helper `set-gpu-freq`

## Usage

```bash
sudo set-gpu-freq <fréquence_MHz>
```

### Exemples

```bash
# Définir la fréquence à 800 MHz
sudo set-gpu-freq 800

# Définir la fréquence à 1200 MHz
sudo set-gpu-freq 1200

# Définir la fréquence maximale (selon votre config)
sudo set-gpu-freq 1600
```

## Plages de fréquence

La plage de fréquences valide dépend de votre configuration dans `/etc/cyan-skillfish-governor/config.toml`.

Par défaut avec les points sûrs conservateurs :

- **Minimum** : 350 MHz
- **Maximum** : 2000 MHz

## Important : Interaction avec le gouverneur dynamique

⚠️ **Le gouverneur dynamique et la fréquence statique ne peuvent pas fonctionner en même temps.**

### Option 1 : Arrêter le gouverneur temporairement

```bash
# Arrêter le gouverneur
sudo systemctl stop cyan-skillfish-governor.service

# Définir votre fréquence statique
sudo set-gpu-freq 1000

# Redémarrer le gouverneur plus tard
sudo systemctl start cyan-skillfish-governor.service
```

### Option 2 : Désactiver le gouverneur de façon permanente

```bash
# Désactiver le gouverneur
sudo systemctl disable --now cyan-skillfish-governor.service

# Maintenant vous pouvez utiliser set-gpu-freq librement
sudo set-gpu-freq 1200
```

**Note :** Avec une fréquence statique, vous devez la changer manuellement selon vos besoins. Le gouverneur dynamique, lui, ajuste automatiquement la fréquence en fonction de la charge GPU.

## Cas d'usage

### Tests et benchmarks

Fixer la fréquence permet d'obtenir des résultats reproductibles :

```bash
sudo systemctl stop cyan-skillfish-governor.service
sudo set-gpu-freq 1600
# Lancer vos benchmarks...
sudo systemctl start cyan-skillfish-governor.service
```

### Économie d'énergie maximale

Forcer une fréquence basse pour maximiser la durée de batterie :

```bash
sudo set-gpu-freq 400
```

### Performance maximale

Forcer la fréquence la plus haute disponible :

```bash
sudo set-gpu-freq 1600
```

## Tensions et sécurité

L'outil utilise automatiquement les tensions sûres définies dans votre fichier de configuration. Par exemple, avec la configuration par défaut :

- 800 MHz → utilise automatiquement la tension sûre appropriée
- 1600 MHz → utilise automatiquement la tension sûre appropriée

Les paires fréquence/tension sont définies dans la section `safe-points` du fichier de configuration.

## Vérifier la fréquence actuelle

Pour vérifier la fréquence GPU actuelle :

```bash
# Via sysfs
cat /sys/class/drm/card*/device/pp_dpm_sclk

# Ou avec radeontop
sudo radeontop
```

## Dépannage

### Erreur : "Cyan Skillfish GPU not found"

Cet outil est spécifique au Steam Deck (AMD Cyan Skillfish APU). Il ne fonctionnera pas sur d'autres GPU.

### Erreur : "Frequency outside valid range"

La fréquence demandée n'est pas dans la plage définie par vos `safe-points`. Vérifiez votre configuration ou ajustez les points sûrs.

### La fréquence change automatiquement

Le gouverneur dynamique est probablement toujours actif. Arrêtez-le avec :

```bash
sudo systemctl stop cyan-skillfish-governor.service
```

## Désinstallation

```bash
sudo rm /usr/local/bin/set_gpu_freq
sudo rm /usr/local/bin/set-gpu-freq
```
