# Installation du Process-Aware Governor

## Description

Remplace le service `cyan-skillfish-governor` standard par le nouveau **process-aware governor** qui :

- 🎮 Apprend automatiquement la fréquence optimale pour chaque jeu
- 💾 Sauvegarde les profils dans une base de données persistante
- 🔄 Réajuste automatiquement si la config graphique change
- 🚫 Ignore les processus desktop (Steam, Discord, etc.)
- ⚡ Change automatiquement pour les processus plus gourmands

## Installation rapide

```bash
./install_process_aware.sh
```

Ce script va :

1. Compiler le governor en mode release
2. Arrêter l'ancien service `cyan-skillfish-governor`
3. Installer le nouveau binaire dans `/usr/local/bin/process-aware-governor`
4. Installer et activer le service systemd `process-aware-governor.service`

## Vérifier que ça fonctionne

```bash
# Voir le statut
sudo systemctl status process-aware-governor.service

# Voir les logs en temps réel
sudo journalctl -u process-aware-governor.service -f
```

## Commandes utiles

```bash
# Redémarrer le service
sudo systemctl restart process-aware-governor.service

# Arrêter le service
sudo systemctl stop process-aware-governor.service

# Voir les logs
sudo journalctl -u process-aware-governor.service -f

# Voir la base de données des profils
cat ~/.cache/cyan-skillfish-governor/process_profiles.json
```

## Désinstallation

Pour revenir à l'ancien service :

```bash
./uninstall_process_aware.sh
sudo systemctl enable cyan-skillfish-governor.service
sudo systemctl start cyan-skillfish-governor.service
```

## Base de données

Les profils appris sont sauvegardés dans :

```
~/.cache/cyan-skillfish-governor/process_profiles.json
```

Chaque jeu aura son entrée avec :

- Nom du jeu (détecté automatiquement depuis Steam/Proton)
- Fréquence optimale (MHz)
- Score de confort (0-100)
- Nombre d'échantillons

## Mode debug

Par défaut, les logs de debug sont désactivés dans le service.

Pour les activer temporairement :

```bash
sudo systemctl stop process-aware-governor.service
sudo DEBUG_GPU_PROCESSES=1 /usr/local/bin/process-aware-governor
```

Pour les activer en permanence, modifier `/etc/systemd/system/process-aware-governor.service` :

```ini
Environment="DEBUG_GPU_PROCESSES=1"
```

Puis recharger :

```bash
sudo systemctl daemon-reload
sudo systemctl restart process-aware-governor.service
```

## Test manuel avant installation

Pour tester sans installer le service :

```bash
# Compilation
cargo build --example process_aware_governor --release

# Lancement manuel (Ctrl+C pour arrêter)
sudo ./target/release/examples/process_aware_governor
```
