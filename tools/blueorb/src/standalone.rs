// Pipeline de Recompilation Statique AOT 100% Natif (ZÉRO émulateur externe)
// Traduit les 75 219 fonctions ARM11/Thumb/VFP de `code.bin` + 145 `Dll*.cro` en C natif via 3dsrecomp,
// compile `librecomp.a` avec le compilateur natif hôte (Clang ARM64 sur Mac, Clang/GCC x86_64 sur Windows/Linux),
// et lie le tout statiquement avec le runtime HLE Rust/Vulkan/Metal Zakuro en un exécutable natif autonome !

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::ProjectConfig;
use crate::icon::generate_app_icons;
use crate::rom3ds;
use crate::toolchain::{candidate_search_dirs, find_program};

const RECOMP3DS_GIT: &str = "https://github.com/fearkov/3dsrecomp.git";
const ZAKURO_GIT: &str = "https://github.com/fearkov/zakuro.git";
const MOLTENVK_URL: &str =
    "https://github.com/KhronosGroup/MoltenVK/releases/download/v1.4.2/MoltenVK-macos.tar";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetPlatform {
    MacOsArm64,
    MacOsX64,
    WindowsX64,
    LinuxX64,
}

impl TargetPlatform {
    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                Self::MacOsArm64
            } else {
                Self::MacOsX64
            }
        } else if cfg!(target_os = "windows") {
            Self::WindowsX64
        } else {
            Self::LinuxX64
        }
    }

    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "macos" | "macos-arm64" | "macos-aarch64" | "mac-arm" => Ok(Self::MacOsArm64),
            "macos-x86_64" | "macos-intel" => Ok(Self::MacOsX64),
            "windows" | "windows-x86_64" | "win64" | "win" => Ok(Self::WindowsX64),
            "linux" | "linux-x86_64" => Ok(Self::LinuxX64),
            other => Err(format!(
                "Plateforme '{}' non reconnue (attendu : macos-arm64, windows-x86_64, linux-x86_64)",
                other
            )),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::MacOsArm64 => "macOS Natif (Apple Silicon ARM64 + Metal)",
            Self::MacOsX64 => "macOS Natif (Intel x86_64)",
            Self::WindowsX64 => "Windows Natif (x86_64 + Vulkan)",
            Self::LinuxX64 => "Linux Natif (x86_64 + Vulkan)",
        }
    }
}

pub struct StandaloneOptions {
    pub target: TargetPlatform,
    pub rom_path: PathBuf,
    pub resolution_factor: u32, // Échelle fenêtre/rendu (défaut: 3)
    pub no_outlines: bool,
    pub unlock_60fps: bool,
    pub portable_copy: bool,
    pub force_recomp: bool,
    pub windowed: bool,
    pub language: u32,
}

impl StandaloneOptions {
    pub fn default_for(root: &Path, cfg: &ProjectConfig) -> Self {
        Self {
            target: TargetPlatform::host(),
            rom_path: root.join(&cfg.default_rom),
            resolution_factor: 3,
            no_outlines: true,
            unlock_60fps: false,
            portable_copy: false,
            force_recomp: false,
            windowed: false,
            language: 2,
        }
    }
}

pub fn build_standalone_app(
    project_root: &Path,
    cfg: &ProjectConfig,
    opts: &StandaloneOptions,
) -> Result<PathBuf, String> {
    let host = TargetPlatform::host();
    if opts.target != host {
        return Err(format!(
            "La recompilation AOT native traduit et lie ~75 000 fonctions C pour le CPU hôte.\n\
             Vous êtes sur {} : lancez cette même commande directement sur {} (`blueorb.bat app` sur Windows ou `./blueorb app` sur Linux/Mac).",
            host.label(),
            opts.target.label()
        ));
    }

    if !opts.rom_path.exists() {
        return Err(format!("ROM introuvable : '{}'", opts.rom_path.display()));
    }

    println!(
        "=== Recompilation Statique AOT Native — {} ===",
        opts.target.label()
    );

    let baserom_dir = project_root.join("baserom");
    let build_dir = project_root.join("build");
    let sdk_dir = build_dir.join("recomp_sdk");
    let port_dir = build_dir.join("native_port");
    let dist_dir = project_root.join("dist");

    fs::create_dir_all(&sdk_dir).map_err(|e| e.to_string())?;

    // 1. Extraire l'icône SMDH officielle si pas encore extraite
    if !baserom_dir.join("exefs").join("icon.bin").exists() {
        println!("[1/5] Extraction de l'en-tête et de l'icône SMDH de la ROM...");
        rom3ds::extract_rom(&opts.rom_path, &baserom_dir, cfg, false)?;
    } else {
        println!("[1/5] Métadonnées et icône SMDH prêtes.");
    }

    // 2. Préparer les sources du recompilateur AOT `3dsrecomp` et du runtime HLE `zakuro`
    let recomp_repo = sdk_dir.join("3dsrecomp");
    let zakuro_repo = sdk_dir.join("zakuro");
    ensure_git_repo(RECOMP3DS_GIT, &recomp_repo, "3dsrecomp")?;
    ensure_git_repo(ZAKURO_GIT, &zakuro_repo, "zakuro")?;

    let cargo_bin = find_program("cargo", &candidate_search_dirs())
        .unwrap_or_else(|| PathBuf::from("cargo"));

    let recomp_exe = recomp_binary_path(&recomp_repo);
    if !recomp_exe.exists() {
        println!("[2/5] Compilation de l'outil de recompilation statique 3dsrecomp...");
        run_checked(
            Command::new(&cargo_bin).args([
                "build",
                "--release",
                "--manifest-path",
                &recomp_repo.join("Cargo.toml").to_string_lossy(),
            ]),
            "compilation de 3dsrecomp",
        )?;
    } else {
        println!("[2/5] Recompilateur AOT 3dsrecomp prêt.");
    }

    // 3. Traduction statique AOT (ARM11 + Thumb + VFP + 145 CROs -> C natif -> librecomp.a)
    let librecomp_a = port_dir.join("code").join("librecomp.a");
    let overrides_dir = project_root.join("overrides");
    let need_recomp = opts.force_recomp
        || !librecomp_a.exists()
        || !port_dir.join("Cargo.toml").exists()
        || overrides_newer_than(&overrides_dir, &librecomp_a);

    if need_recomp {
        println!(
            "[3/5] Traduction AOT des ~75 219 fonctions (code.bin + 145 Dll*.cro) en C natif et compilation de librecomp.a..."
        );
        println!(
            "      (La première recompilation complète compile ~9,2 Mo de code machine en C natif : comptez ~2 à 5 minutes)"
        );
        let mut cmd = Command::new(&recomp_exe);
        cmd.arg("port")
            .arg(&opts.rom_path)
            .arg(&port_dir)
            .arg("--name")
            .arg("pokemon-alpha-sapphire")
            .arg("--zakuro")
            .arg(&zakuro_repo);
        if overrides_dir.is_dir() {
            cmd.arg("--overrides").arg(&overrides_dir);
        }
        // Sur Windows MSVC, configurer automatiquement clang + llvm-ar si installés
        configure_windows_cc_env(&mut cmd);
        run_checked(&mut cmd, "3dsrecomp port")?;

        // Ajouter #![cfg_attr(target_os = "windows", windows_subsystem = "windows")] au main.rs généré
        patch_generated_port_main(&port_dir.join("src").join("main.rs"))?;
    } else {
        println!(
            "[3/5] Bibliothèque native AOT déjà compilée ({}) — réutilisation instantanée !",
            librecomp_a.display()
        );
    }

    // 4. Compilation & édition de liens statique de l'exécutable natif final (librecomp.a + runtime HLE Rust)
    let native_port_bin = port_binary_path(&port_dir);
    if need_recomp || !native_port_bin.exists() {
        println!("[4/5] Liaison statique de l'exécutable natif autonome (librecomp.a + runtime HLE GPU/OS)...");
        run_checked(
            Command::new(&cargo_bin).args([
                "build",
                "--release",
                "--manifest-path",
                &port_dir.join("Cargo.toml").to_string_lossy(),
            ]),
            "compilation de l'exécutable natif pokemon-alpha-sapphire",
        )?;
    } else {
        println!("[4/5] Exécutable natif déjà à jour.");
    }

    // Compiler aussi notre micro-lanceur d'auto-boot sans arguments (`tools/launcher`)
    run_checked(
        Command::new(&cargo_bin).args([
            "build",
            "--release",
            "--manifest-path",
            &project_root
                .join("tools")
                .join("launcher")
                .join("Cargo.toml")
                .to_string_lossy(),
        ]),
        "compilation du lanceur natif",
    )?;
    let launcher_bin = launcher_binary_path(project_root);

    // 5. Assemblage du package Standalone natif dans `dist/`
    println!("[5/5] Création du package applicatif natif dans dist/...");
    fs::create_dir_all(&dist_dir).map_err(|e| e.to_string())?;
    let icon_smdh = baserom_dir.join("exefs").join("icon.bin");

    let output_bundle = match opts.target {
        TargetPlatform::MacOsArm64 | TargetPlatform::MacOsX64 => {
            let moltenvk_dylib = ensure_moltenvk_macos(&sdk_dir)?;

            // Construire et signer dans un dossier temporaire hors d'iCloud Drive pour une signature Apple Silicon 100% valide
            let tmp_stage = env::temp_dir().join(format!("blueorb_sign_{}", std::process::id()));
            if tmp_stage.exists() {
                let _ = fs::remove_dir_all(&tmp_stage);
            }
            let tmp_app = tmp_stage.join("Pokemon Alpha Sapphire.app");
            let contents = tmp_app.join("Contents");
            let macos_dir = contents.join("MacOS");
            let res_dir = contents.join("Resources");
            let fw_dir = contents.join("Frameworks");

            fs::create_dir_all(&macos_dir).map_err(|e| e.to_string())?;
            fs::create_dir_all(&res_dir).map_err(|e| e.to_string())?;
            fs::create_dir_all(&fw_dir).map_err(|e| e.to_string())?;

            generate_app_icons(&icon_smdh, Some(&res_dir.join("AppIcon.icns")), None, None)?;
            write_macos_info_plist(&contents.join("Info.plist"))?;

            let dst_boot = macos_dir.join("PokemonAlphaSapphire");
            let dst_engine = macos_dir.join("pokemon-alpha-sapphire");
            fs::copy(&launcher_bin, &dst_boot).map_err(|e| e.to_string())?;
            fs::copy(&native_port_bin, &dst_engine).map_err(|e| e.to_string())?;
            fs::copy(&moltenvk_dylib, fw_dir.join("libMoltenVK.dylib")).map_err(|e| e.to_string())?;
            make_executable(&dst_boot);
            make_executable(&dst_engine);

            // Signature ad-hoc Apple Silicon propre (dans /tmp, sans attributs iCloud)
            run_checked(
                Command::new("codesign")
                    .args(["--force", "--deep", "--sign", "-"])
                    .arg(&tmp_app),
                "signature Apple Silicon codesign",
            )?;

            let final_app = dist_dir.join("Pokemon Alpha Sapphire.app");
            if final_app.exists() {
                fs::remove_dir_all(&final_app).map_err(|e| e.to_string())?;
            }
            move_dir_cross_fs(&tmp_app, &final_app)?;
            let _ = fs::remove_dir_all(&tmp_stage);

            // Données d'assets de la ROM & sauvegardes dans `dist/game_data/`
            let game_data = dist_dir.join("game_data");
            stage_rom_assets_and_mods(&opts.rom_path, &build_dir, &game_data, cfg, opts)?;

            final_app
        }
        TargetPlatform::WindowsX64 => {
            let pkg_dir = dist_dir.join("Pokemon Alpha Sapphire");
            let game_data = pkg_dir.join("game_data");
            fs::create_dir_all(&game_data).map_err(|e| e.to_string())?;

            generate_app_icons(&icon_smdh, None, Some(&pkg_dir.join("AppIcon.ico")), None)?;
            fs::copy(&launcher_bin, pkg_dir.join("Pokemon Alpha Sapphire.exe"))
                .map_err(|e| e.to_string())?;
            fs::copy(&native_port_bin, pkg_dir.join("pokemon-alpha-sapphire-bin.exe"))
                .map_err(|e| e.to_string())?;

            stage_rom_assets_and_mods(&opts.rom_path, &build_dir, &game_data, cfg, opts)?;
            pkg_dir
        }
        TargetPlatform::LinuxX64 => {
            let pkg_dir = dist_dir.join("Pokemon Alpha Sapphire");
            let game_data = pkg_dir.join("game_data");
            fs::create_dir_all(&game_data).map_err(|e| e.to_string())?;

            generate_app_icons(&icon_smdh, None, None, Some(&pkg_dir.join("AppIcon.png")))?;
            let dst_boot = pkg_dir.join("PokemonAlphaSapphire");
            let dst_bin = pkg_dir.join("pokemon-alpha-sapphire-bin");
            fs::copy(&launcher_bin, &dst_boot).map_err(|e| e.to_string())?;
            fs::copy(&native_port_bin, &dst_bin).map_err(|e| e.to_string())?;
            make_executable(&dst_boot);
            make_executable(&dst_bin);

            stage_rom_assets_and_mods(&opts.rom_path, &build_dir, &game_data, cfg, opts)?;

            let desktop = format!(
                "[Desktop Entry]\nType=Application\nName=Pokémon Saphir Alpha (Natif)\nExec=\"{}/PokemonAlphaSapphire\"\nIcon={}/AppIcon.png\nCategories=Game;\nTerminal=false\n",
                pkg_dir.display(),
                pkg_dir.display()
            );
            fs::write(pkg_dir.join("Pokemon-Alpha-Sapphire.desktop"), desktop)
                .map_err(|e| e.to_string())?;
            pkg_dir
        }
    };

    println!(
        "\nPortage Natif AOT prêt (ZÉRO émulateur externe) : {}",
        output_bundle.display()
    );
    println!("  - Contrôles : Flèches / IJKL (Stick) | X=A, Z=B, S=X, A=Y | Entrée=Start | Souris=Tactile | F11=Plein écran | Échap=Menu");
    Ok(output_bundle)
}

pub fn launch_standalone_app(
    bundle_path: &Path,
    target: TargetPlatform,
    opts: &StandaloneOptions,
) -> Result<(), String> {
    println!("[play] Lancement du binaire natif AOT Pokémon Saphir Alpha...");
    let bin = match target {
        TargetPlatform::MacOsArm64 | TargetPlatform::MacOsX64 => bundle_path
            .join("Contents")
            .join("MacOS")
            .join("PokemonAlphaSapphire"),
        TargetPlatform::WindowsX64 => bundle_path.join("Pokemon Alpha Sapphire.exe"),
        TargetPlatform::LinuxX64 => bundle_path.join("PokemonAlphaSapphire"),
    };

    let mut cmd = Command::new(&bin);
    cmd.args(["--scale", &opts.resolution_factor.to_string()]);
    cmd.spawn()
        .map_err(|e| format!("Impossible de lancer '{}': {}", bin.display(), e))?;
    Ok(())
}

fn ensure_git_repo(url: &str, dest: &Path, name: &str) -> Result<(), String> {
    if dest.join("Cargo.toml").exists() {
        return Ok(());
    }
    println!("  Clonage de {} ({})...", name, url);
    if dest.exists() {
        let _ = fs::remove_dir_all(dest);
    }
    run_checked(
        Command::new("git")
            .args(["clone", "--depth", "1", url])
            .arg(dest),
        &format!("git clone {}", name),
    )
}

fn ensure_moltenvk_macos(sdk_dir: &Path) -> Result<PathBuf, String> {
    let cached_dylib = sdk_dir.join("libMoltenVK.dylib");
    if cached_dylib.is_file() {
        return Ok(cached_dylib);
    }
    println!("  Téléchargement de MoltenVK (pont Vulkan -> Apple Metal)...");
    let tar_path = sdk_dir.join("MoltenVK-macos.tar");
    run_checked(
        Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&tar_path)
            .arg(MOLTENVK_URL),
        "téléchargement de MoltenVK",
    )?;
    let unpack = sdk_dir.join("_mvk_unpack");
    let _ = fs::remove_dir_all(&unpack);
    fs::create_dir_all(&unpack).map_err(|e| e.to_string())?;
    run_checked(
        Command::new("tar")
            .args(["xf"])
            .arg(&tar_path)
            .arg("-C")
            .arg(&unpack)
            .arg("MoltenVK/MoltenVK/dynamic/dylib/macOS/libMoltenVK.dylib"),
        "extraction de libMoltenVK.dylib",
    )?;
    let extracted = unpack
        .join("MoltenVK")
        .join("MoltenVK")
        .join("dynamic")
        .join("dylib")
        .join("macOS")
        .join("libMoltenVK.dylib");
    fs::copy(&extracted, &cached_dylib).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&unpack);
    let _ = fs::remove_file(&tar_path);
    Ok(cached_dylib)
}

fn stage_rom_assets_and_mods(
    rom_path: &Path,
    _build_dir: &Path,
    game_data_dir: &Path,
    cfg: &ProjectConfig,
    opts: &StandaloneOptions,
) -> Result<(), String> {
    fs::create_dir_all(game_data_dir).map_err(|e| e.to_string())?;
    let dst_rom = game_data_dir.join("sango2.3ds");
    if !dst_rom.exists() {
        if opts.portable_copy || fs::hard_link(rom_path, &dst_rom).is_err() {
            println!("  Copie des assets RomFS vers {}...", dst_rom.display());
            fs::copy(rom_path, &dst_rom).map_err(|e| e.to_string())?;
        }
    }

    let userdata = game_data_dir.join("userdata");
    let cheats_dir = userdata.join("cheats");
    fs::create_dir_all(&cheats_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(userdata.join("mods").join(&cfg.title_id).join("textures"))
        .map_err(|e| e.to_string())?;

    let mut cheats_txt = String::new();
    cheats_txt.push_str("[HD - Sans Contours Noirs (No Outlines v1.4)]\n");
    if opts.no_outlines {
        cheats_txt.push_str("*citra_enabled\n");
    }
    cheats_txt.push_str("D3000000 00000000\n0037A140 00000000\n0037A144 00000000\nD2000000 00000000\n\n");
    cheats_txt.push_str("[60 FPS Unlock (v1.4)]\n");
    if opts.unlock_60fps {
        cheats_txt.push_str("*citra_enabled\n");
    }
    cheats_txt.push_str("D3000000 00000000\n08C690A0 00000002\nD2000000 00000000\n");

    fs::write(cheats_dir.join(format!("{}.txt", cfg.title_id)), cheats_txt)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn patch_generated_port_main(main_rs: &Path) -> Result<(), String> {
    if !main_rs.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(main_rs).map_err(|e| e.to_string())?;
    if !content.contains("windows_subsystem") {
        let patched = format!(
            "#![cfg_attr(target_os = \"windows\", windows_subsystem = \"windows\")]\n{}",
            content
        );
        fs::write(main_rs, patched).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn configure_windows_cc_env(cmd: &mut Command) {
    if !cfg!(target_os = "windows") {
        return;
    }
    let dirs = candidate_search_dirs();
    if env::var_os("CC").is_none() {
        if let Some(clang) = find_program("clang", &dirs) {
            cmd.env("CC", clang);
        }
    }
    if env::var_os("AR").is_none() {
        if let Some(ar) = find_program("llvm-ar", &dirs) {
            cmd.env("AR", ar);
        }
    }
}

fn overrides_newer_than(overrides_dir: &Path, archive: &Path) -> bool {
    let Ok(arc_time) = fs::metadata(archive).and_then(|m| m.modified()) else {
        return true;
    };
    let Ok(entries) = fs::read_dir(overrides_dir) else {
        return false;
    };
    for e in entries.flatten() {
        if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
            if t > arc_time {
                return true;
            }
        }
    }
    false
}

fn recomp_binary_path(recomp_repo: &Path) -> PathBuf {
    let name = if cfg!(target_os = "windows") {
        "3dsrecomp.exe"
    } else {
        "3dsrecomp"
    };
    recomp_repo.join("target").join("release").join(name)
}

fn port_binary_path(port_dir: &Path) -> PathBuf {
    let name = if cfg!(target_os = "windows") {
        "pokemon-alpha-sapphire.exe"
    } else {
        "pokemon-alpha-sapphire"
    };
    port_dir.join("target").join("release").join(name)
}

fn launcher_binary_path(project_root: &Path) -> PathBuf {
    let name = if cfg!(target_os = "windows") {
        "pokemon_alpha_sapphire_launcher.exe"
    } else {
        "pokemon_alpha_sapphire_launcher"
    };
    project_root
        .join("tools")
        .join("launcher")
        .join("target")
        .join("release")
        .join(name)
}

fn move_dir_cross_fs(src: &Path, dst: &Path) -> Result<(), String> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    // Copie récursive si /tmp et le dossier du projet sont sur des volumes différents
    copy_dir_recursive(src, dst)?;
    let _ = fs::remove_dir_all(src);
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let sp = entry.path();
        let dp = dst.join(entry.file_name());
        if sp.is_dir() {
            copy_dir_recursive(&sp, &dp)?;
        } else {
            fs::copy(&sp, &dp).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            if let Ok(meta) = fs::metadata(&sp) {
                let _ = fs::set_permissions(&dp, meta.permissions());
            }
        }
    }
    Ok(())
}

fn run_checked(cmd: &mut Command, step_desc: &str) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("Impossible de lancer {} : {}", step_desc, e))?;
    if !status.success() {
        return Err(format!("Échec lors de : {}", step_desc));
    }
    Ok(())
}

fn write_macos_info_plist(plist_path: &Path) -> Result<(), String> {
    let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>fr</string>
    <key>CFBundleDisplayName</key>
    <string>Pokémon Saphir Alpha</string>
    <key>CFBundleExecutable</key>
    <string>PokemonAlphaSapphire</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>com.blueorb.pokemon-alpha-sapphire-native</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Pokémon Saphir Alpha</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.4.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.games</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSCameraUsageDescription</key>
    <string>Utilisé par les fonctionnalités photo du jeu.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>Utilisé par les fonctionnalités micro du jeu.</string>
</dict>
</plist>
"#;
    fs::write(plist_path, plist).map_err(|e| e.to_string())
}

fn make_executable(p: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(p) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(p, perms);
        }
    }
    let _ = env::consts::OS;
}
