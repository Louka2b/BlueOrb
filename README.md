# BlueOrb — Recompilation Statique AOT Native de Pokémon Saphir Alpha (Zéro Émulateur)

**BlueOrb** traduit statiquement l'intégralité du code machine ARM11 / Thumb / VFPv2 de **Pokémon Saphir Alpha** (`code.bin` + les **145 modules dynamiques `Dll*.cro`**, soit **75 219 fonctions** et plus de **3 millions d'instructions**) directement en **code C natif** compilé pour le processeur de votre machine (**Apple Silicon ARM64** sur Mac, **x86_64** sur Windows et Linux), lié statiquement en **un seul exécutable natif autonome** via `3dsrecomp` + le runtime HLE `Zakuro` (GPU Metal/Vulkan, Audio, OS).

Aucun émulateur externe (ni Citra, ni Lime3DS, ni Azahar) n'est utilisé.

---

## 1. Compiler et lancer l'application native

Placez votre ROM décryptée `Pokemon Alpha Sapphire (USA) (En,Ja,Fr,De,Es,It,Ko) (Rev 2).3ds` à la racine du dépôt, puis lancez :

### Sur macOS (Apple Silicon ARM64) & Linux
```bash
# Recompiler statiquement en natif (la 1re fois prend ~3-5 min) et lancer le jeu :
./blueorb play

# Ou construire uniquement l'application native autonome dans dist/ :
./blueorb app
```
Cela produit **`dist/Pokemon Alpha Sapphire.app`**, une vraie application macOS ARM64 native signée contenant :
- Les 75 219 fonctions du jeu compilées nativement en code machine Apple Silicon `arm64` (`librecomp.a` lié statiquement dans le binaire).
- `libMoltenVK.dylib` intégré pour le rendu matériel 3D direct sur le GPU **Apple Metal**.
- L'icône officielle haute définition extraite de l'en-tête SMDH de votre ROM.

### Sur Windows (`x86_64`)
```powershell
# PowerShell :
.\blueorb.ps1 play

# Invite de commandes (cmd.exe) :
blueorb.bat play
```
Cela produit **`dist\Pokemon Alpha Sapphire\Pokemon Alpha Sapphire.exe`** (exécutable natif Windows `x86_64` sans console noire).

---

## 2. Remplacer des fonctions du jeu en C natif (`overrides/`)

Vous pouvez réécrire ou modder n'importe quelle fonction de `code.bin` ou d'un module `Dll*.cro` directement en C natif dans [overrides/blueorb_overrides.c](overrides/blueorb_overrides.c) :

```c
#include "overrides.h"

// Remplace la fonction 0x0033E190 de code.bin par du C natif hôte :
RECOMP_OVERRIDE(0x0033E190) {
    uint32_t self = ctx->r[0];
    ctx->r[0] = mem_read16(ctx, self + 0x32);
    RETURN_TO(ctx->r[14]);
}

// Remplace une fonction dans un module dynamique CRO (ex: DllBattle.cro) :
// RECOMP_OVERRIDE_IN(DllBattle, 0x1A40) { ... }
```

Relancez simplement `./blueorb play` : BlueOrb détecte la modification dans `overrides/`, recompile le code C natif et met à jour l'application !

---

## 3. Contrôles en jeu (Clavier & Manettes PS5 / Xbox / Switch Pro)

Les manettes Bluetooth/USB sont reconnues nativement. Au clavier :
- **Croix directionnelle** : Flèches directionnelles
- **Stick analogique (Circle Pad)** : `I` / `J` / `K` / `L`
- **Boutons A / B / X / Y** : `X` / `Z` / `S` / `A`
- **Boutons L / R** : `Q` / `W`
- **Start / Select** : `Entrée` / `Retour arrière`
- **Écran tactile** : Clic souris
- **Plein écran** : `F11`
- **Menu intégré (Résolution, Disposition écrans, Cheats 60 FPS / No Outlines, Sauvegardes)** : `Échap`

---

## 4. Commandes d'analyse et de décompilation matching ARMv6K

BlueOrb conserve également toute sa boîte à outils d'inspection et de décompilation *matching* :
```bash
./blueorb inspect 0x00139414     # Désassembler une fonction de code.bin avec pools résolus
./blueorb cro DllBattle          # Inspecter l'un des 145 modules dynamiques CRO
./blueorb setup                  # Pipeline complet de décompilation matching ARMv6K
./blueorb diff <symbole>         # Comparaison d'instructions côte-à-côte
```
