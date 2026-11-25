# BlueOrb — Portage Natif Recompilé (AOT) & Remaster HD de Pokémon Saphir Alpha

**BlueOrb** est un outil de **recompilation statique AOT native** (sans aucun émulateur externe) pour **Pokémon Saphir Alpha** (*CTR-P-ECLA*, Révision 2 / `sango-2`). Il traduit les **75 219 fonctions** du jeu (`code.bin` + les **145 modules `Dll*.cro`**) en code C natif compilé directement pour votre processeur (**Apple Silicon ARM64** sur Mac, **x86_64** sur Linux et Windows) et lié à un moteur graphique moderne **Vulkan / Apple Metal** enrichi d'améliorations Remaster HD.

---

## Améliorations Graphiques & Gameplay ("Remaster HD" activé par défaut)

Toutes ces améliorations sont compilées directement dans votre exécutable natif sur **Linux, macOS et Windows** :

1. **Upscaling de textures temps-réel `4x Smooth-xBR / EPX` + Mipmaps trilinéaires** :
   - Chaque texture 3DS (Pokémon, personnages, interfaces, panneaux, icônes) est automatiquement lissée et upscalée en **4x** sur les contours lors de son envoi en VRAM (`0,00 ms` d'impact par image en jeu).
2. **Rendu 3D Interne Ultra-HD `5x` (`2000×1200`) + Anti-Aliasing `16-tap SSAA`** :
   - Calcul géométrique 3D à 5 fois la définition d'origine de la 3DS (modifiable jusqu'en **8x / 10x 4K+** avec `--res 8` ou en direct via `Échap`).
3. **Filtrage Anisotrope Matériel `16x`** :
   - Supprime le flou sur les sols, falaises et océans d'Hoenn vus en perspective rasante.
4. **Fluidité `60 FPS` par Interpolation Matricielle 3D (`frame_interpolation = true`)** :
   - Génère de vraies frames intermédiaires 3D à **60 FPS** sur le GPU **sans accélérer la vitesse du jeu** (contrairement aux patchs 60 FPS classiques qui doublent la vitesse d'horloge).
5. **Mode `No Outlines (HD)` (Sans contours noirs) + `Mode Turbo 2x` réversibles à chaud** :
   - Pré-activé sur les adresses flottantes exactes de la révision USA Rev 2 (`0x00379EB4` / `0x00379EB8`) pour un rendu moderne façon Switch. Activable/désactivable en un clic à tout moment dans **`Échap -> Cheats`**.
6. **Affichage Classique de Base (`5:3` Superposé) + Vrai Support Ultra-Wide (`16:9`, `21:9`, `32:9`)** :
   - Démarre par défaut avec l'overlay 3DS classique superposé (`5:3`). En un clic depuis le nouveau **Menu Pause Saphir (`Échap`)** ou via **`F6`**, basculez en vrai rendu 3D *Hor+* `16:9`, `21:9 Ultra-Wide` ou `32:9` **sans étirer** les modèles ni les boîtes de dialogue 2D.
7. **Séparation Écran Haut / Écran Bas Détachable (`Dual Windows`)** :
   - Détachez ou regroupez à chaud les 2 écrans en un clic depuis le menu **`Échap`** ou avec **`F7`** : l'écran tactile PokéMulti-Navi s'ouvre dans sa propre fenêtre indépendante (rendu GPU Vulkan/Metal partagé zéro-copie à 60 FPS + souris tactile), et fermer cette 2e fenêtre regroupe automatiquement les écrans.
8. **Menu Pause "Saphir Abyssal" repensé + Installateur Graphique (`GUI`)** :
   - Glissez-déposez simplement votre ROM `.3ds` dans l'**Installateur Graphique BlueOrb** (`./blueorb gui` / `BlueOrb Installer.app` / `BlueOrb-Installer.exe`) : vérification automatique de l'en-tête NCCH, recompilation AOT complète et bouton **Jouer maintenant**.

---

## Méthode 1 — Installateur Graphique (Glisser-Déposer la ROM)

Ouvrez l'installateur visuel puis glissez votre fichier `.3ds` dedans :
- **macOS** : `./blueorb gui` *(génère et ouvre aussi `dist/BlueOrb Installer.app`)*
- **Windows** : `blueorb.bat gui` *(génère et ouvre `dist\BlueOrb Installer\BlueOrb-Installer.exe`)*
- **Linux** : `./blueorb gui` *(génère et ouvre `dist/BlueOrb Installer/BlueOrb-Installer`)*

---

## Méthode 2 — Compiler en ligne de commande sur macOS, Linux et Windows

Placez votre ROM décryptée `Pokemon Alpha Sapphire (USA) (En,Ja,Fr,De,Es,It,Ko) (Rev 2).3ds` à la racine du dossier `BlueOrb`.

### 1. macOS (Apple Silicon M1/M2/M3/M4/M5 `ARM64`)

**Prérequis (une seule fois) :**
```bash
# Installer les outils de compilation Apple (fournit clang et git)
xcode-select --install

# Installer Rust (si absent)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Compiler et lancer :**
```bash
# Compile l'application native dans `dist/Pokemon Alpha Sapphire.app` :
./blueorb app

# Ou compile ET lance immédiatement le jeu :
./blueorb play
```
*(La toute première compilation traduit ~687 Mio de C natif et prend ~3 à 5 minutes ; les lancements suivants prennent moins d'une seconde grâce au cache incrémental `.o`).*

---

### 2. Windows (`x86_64`)

**Prérequis (une seule fois, dans PowerShell) :**
```powershell
# Installer Rust, Git et le compilateur C natif LLVM/Clang (ou Visual Studio Build Tools)
winget install -e --id Git.Git
winget install -e --id Rustlang.Rustup
winget install -e --id LLVM.LLVM
```
*(Note : avec Rust sous Windows MSVC, installez aussi les "Desktop development with C++" de Visual Studio Build Tools si absents, puis redémarrez PowerShell).*

**Compiler et lancer :**
```powershell
# Dans PowerShell :
.\blueorb.ps1 app
.\blueorb.ps1 play

# Ou dans l'Invite de commandes (cmd.exe) :
blueorb.bat app
blueorb.bat play
```
Votre exécutable natif Windows autonome est généré dans :
`dist\Pokemon Alpha Sapphire\Pokemon Alpha Sapphire.exe` (sans fenêtre console noire).

---

### 3. Linux (`x86_64` — Ubuntu / Debian / Arch / Fedora)

**Prérequis (une seule fois) :**
```bash
# Ubuntu / Debian
sudo apt update
sudo apt install -y build-essential clang git curl pkg-config libasound2-dev libudev-dev vulkan-loader

# Arch Linux
sudo pacman -S base-devel clang git curl pkgconf alsa-lib systemd-libs vulkan-icd-loader rust

# Fedora
sudo dnf install @development-tools clang git curl pkgconf-pkg-config alsa-lib-devel systemd-devel vulkan-loader
```
*(Si Rust n'est pas installé : `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).*

**Compiler et lancer :**
```bash
./blueorb app
./blueorb play
```
Votre exécutable natif Linux autonome + lanceur `.desktop` sont générés dans :
`dist/Pokemon Alpha Sapphire/PokemonAlphaSapphire`.

---

## Raccourcis & Contrôles en jeu

- **Changer le format d'écran (`5:3` → `16:9` → `21:9 Ultra-Wide` → `32:9 Super Ultra-Wide`)** : **`F6`**
- **Séparer / Regrouper les 2 écrans sur 2 fenêtres indépendantes** : **`F7`**
- **Changer de disposition d'écrans** : **`F9`** | **Plein écran** : **`F11`** | **Pause** : **`F1`**
- **Menu en surimpression (Graphismes, Résolution 1x-8x, Écrans, Cheats HD / Turbo, Sauvegardes)** : **`Échap`**
- **Manettes PS5 / Xbox / Switch Pro** : reconnues automatiquement
- **Clavier** : Flèches (Croix) | `I`/`J`/`K`/`L` (Stick) | `X`=A, `Z`=B, `S`=X, `A`=Y | `Q`=L, `W`=R | `Entrée`=Start | Souris=Tactile (sur la 2e fenêtre ou l'écran bas)
