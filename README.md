# BlueOrb — Recompilateur Natif & Remaster HD / Ultra-Wide (Sans Terminal)

**BlueOrb** transforme votre copie `.3ds` décryptée de **Pokémon Saphir Alpha** (*CTR-P-ECLA*, Révision 2) en un **jeu PC/Mac natif recompilé** (**macOS Apple Silicon `ARM64`**, **Windows `x86_64`**, **Linux `x86_64`**), sans émulateur externe et sans aucune commande à taper dans un terminal.

> **100 % Légal** : Ce dépôt et les versions publiées dans **Releases** ne contiennent **aucun** fichier ROM, code, icône ou asset de Pokémon / Nintendo / Game Freak. Vous fournissez uniquement votre propre fichier `.3ds` décrypté au premier lancement.

---

## Installation Simple (Sans Terminal — Recommandé pour tout le monde)

1. Rendez-vous dans l'onglet **[Releases](../../releases)** de GitHub et téléchargez le fichier correspondant à votre ordinateur :
   - **Windows** : `BlueOrb-Windows-x64.zip` → Décompressez et double-cliquez sur **`BlueOrb.exe`**
   - **macOS (M1 / M2 / M3 / M4 / M5)** : `BlueOrb-macOS-AppleSilicon.zip` → Double-cliquez sur **`BlueOrb.app`** *(si macOS bloque la 1re ouverture : Réglages Système → Confidentialité et sécurité → Ouvrir quand même)*
   - **Linux / Steam Deck** : `BlueOrb-Linux-x86_64.tar.gz` → Extrayez et lancez **`BlueOrb`**
2. **Au tout premier lancement** :
   - Cliquez sur **« Choisir mon fichier .3ds… »** (ou glissez votre fichier `.3ds` décrypté directement dans la fenêtre).
   - BlueOrb recompile automatiquement le jeu en code machine natif avec une barre de progression *(aucun outil à installer : si votre PC n'a pas de compilateur C, un compilateur portable est téléchargé tout seul)*, puis **lance le jeu automatiquement**.
3. **Toutes les fois suivantes** :
   - Ouvrez simplement **`BlueOrb`** : le jeu démarre **immédiatement** !

---

## Nouveautés & Améliorations Incluses

- **Qualité Haute Définition (`1440p` par défaut, jusqu'à `4K+`)** : rendu géométrique 3D haute résolution, lissage des textures `4x Smooth-xBR`, filtrage anisotrope `16x` et option **Sans contours noirs (HD)** activée par défaut.
- **60 Hz / 60 FPS stable sans clignotement** : synchronisation fluide sans artefacts d'interpolation d'atlas UV (et fluidité expérimentale désactivable/activable dans le menu).
- **Vrai Support Ultra-Wide (`5:3`, `16:9`, `21:9`, `32:9`)** : démarre avec l'affichage classique `5:3` superposé et passe en plein écran large *Hor+* sans étirer les personnages ni l'interface (`F6` ou menu `Échap`).
- **Deux Fenêtres Détachables (`Dual Screen`)** : détachez l'écran tactile du bas (PokéMulti-Navi, sac, combats) dans sa propre fenêtre indépendante en un clic (`F7` ou menu `Échap`), puis fermez la 2e fenêtre pour regrouper.
- **Menu en français pensé pour tous (`Échap`)** : réglages clairs sans jargon technique (*Jeu & Sauvegardes*, *Écrans & Ultra-Wide*, *Qualité & Son*, *Touches & Manette*, *Multijoueur & Bonus*), manettes Xbox / PlayStation / Switch Pro reconnues automatiquement.

---

## Raccourcis Pratiques en Jeu

| Touche | Action |
| :--- | :--- |
| **`Échap`** | Ouvrir / fermer le menu des réglages et sauvegardes rapides |
| **`F6`** | Changer le format d'écran (`5:3` → `16:9` → `21:9 Ultra-Wide` → `32:9`) |
| **`F7`** | Détacher ou regrouper les deux écrans sur **2 fenêtres** |
| **`F9`** | Changer la disposition des écrans |
| **`F11`** | Activer / quitter le plein écran |

---

## Compiler soi-même depuis le code source (Optionnel — Développeurs)

Si vous souhaitez compiler vous-même depuis le dépôt avec Rust installé :

```bash
# Générer l'application autonome 100 % légale (Zéro ROM requise) dans dist/BlueOrb* :
./blueorb release          # macOS / Linux
.\blueorb.ps1 release      # Windows PowerShell

# Ou compiler un bundle statique pré-lié avec votre ROM locale :
./blueorb app              # macOS / Linux
.\blueorb.ps1 app          # Windows PowerShell
```
