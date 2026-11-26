# BlueOrb — Native AOT Port & HD / Ultrawide Runtime

**BlueOrb** compiles your decrypted Nintendo 3DS `.3ds` copy of **Pokémon Alpha Sapphire** (*CTR-P-ECLA*, Rev 2) into a **native desktop executable** (**macOS Apple Silicon `ARM64`**, **Windows `x86_64`**, **Linux `x86_64`**) with zero external emulator and zero terminal setup required.

> **100% Legal Distribution**: This repository and its **Releases** contain **zero** ROM bytes, game code, sprites, music, or copyrighted Nintendo / Pokémon / Game Freak icons. You supply your own legally dumped, decrypted `.3ds` ROM on first launch.

---

## Quick Start (Zero Terminal)

1. Download the build for your OS from **[GitHub Releases](../../releases)**:
   - **Windows (`x64`)**: `BlueOrb-Windows-x64.exe` — download and run **`BlueOrb-Windows-x64.exe`** directly (portable single-file executable, no extraction needed).
   - **macOS (`ARM64` M1–M5)**: `BlueOrb-macOS-AppleSilicon.dmg` — open the DMG and **drag `BlueOrb` to `Applications` first**. On first launch from `Applications`, click **Done** (*Terminé* — do **not** click *Move to Trash*), then open **System Settings → Privacy & Security**, scroll down, and click **Open Anyway** (*Ouvrir quand même*).
   - **Linux (`x64` / Steam Deck)**: `BlueOrb-Linux-x86_64.tar.gz` — extract and run **`./BlueOrb`**.
2. **First Launch**:
   - Click **Select ROM…** (or drop your decrypted `.3ds` / `.cci` file onto the launcher window).
   - BlueOrb statically recompiles the game for your CPU (automatically fetching a portable `zig cc` compiler if needed), updates the app icon to your ROM's official icon once compiled, and launches straight into the game.
3. **Future Launches**:
   - Open **`BlueOrb`** to boot directly into your game.
4. **Cleaning Residual Files**:
   - Click **Clean Cache** in the launcher bar (or **System & Cheats → Clean Residual Build Files…** inside `Esc`) anytime to remove temporary build artifacts while keeping all save files intact.

---

## Key Features

- **Native HD 3D Rendering**: Internal 3D scaling up to `4K+` (`5x / 1440p` default), `4x Smooth-xBR` texture upscaling, `16x` anisotropic filtering, and optional **No Outlines (HD)** toggle.
- **Flicker-Free 60 Hz Display**: Safe default presentation with UV sprite-atlas guards in the optional frame interpolator.
- **Hor+ Ultrawide (`5:3`, `16:9`, `21:9`, `32:9`)**: Starts in native `5:3` stacked layout; press **`F6`** or open `Esc → Display` for true unstretched widescreen/ultrawide.
- **Detachable Dual Windows**: Press **`F7`** to pop the bottom touch screen into a second independent window (zero-copy shared GPU surface + mouse touch input).
- **Co-op Campaign & Multiplayer (`F4`)**: Host or join rooms via **LAN auto-discovery** or **IP:Port** to see friends live in Hoenn's 3D overworld, designate a **Chef (Leader)** whose zone changes automatically **teleport** party members beside them, **wait cleanly on story cutscenes** (with a clear on-screen wait notice until all players finish before syncing), **upgrade standard encounters into 2v2 Double Battles** while **leaving solo, tutorial, trainer 1v1, Sky, Multi, and scripted boss battles intact**, keep each player's **real save file & Pokémon** 100% untouched, and unlock campaign co-op automatically right after the tutorial (picking your starter). Also supports native 3DS PSS/local wireless battles & trades.
- **English by Default**: Clean native desktop UI, Xbox / PlayStation / Switch Pro controller auto-mapping, save states (`F5`/`F8`), and custom texture/cheat support.

---

## In-Game Hotkeys

| Key | Action |
| :--- | :--- |
| **`Esc`** | Toggle Pause / Settings overlay |
| **`F4`** | Toggle Multiplayer & Live Overworld Co-op window |
| **`F5` / `F8`** | Quick Save State / Quick Load State |
| **`F6`** | Cycle Aspect Ratio (`5:3` → `16:9` → `21:9` → `32:9`) |
| **`F7`** | Detach / Re-attach Dual Screen Window |
| **`F9`** | Cycle Screen Layout |
| **`F11`** | Toggle Fullscreen |

---

## Building from Source (Optional — Developers)

```bash
# Build the 100% legal standalone BlueOrb GUI app (no ROM required) in dist/:
./blueorb release          # macOS / Linux
.\blueorb.ps1 release      # Windows PowerShell

# Build a pre-linked app directly from a local ROM:
./blueorb app              # macOS / Linux
.\blueorb.ps1 app          # Windows PowerShell
```
