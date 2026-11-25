#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn locate_paths(exe_path: &Path) -> Option<(PathBuf, PathBuf)> {
    let exe_dir = exe_path.parent()?;

    // Cas 1 : Bundle macOS `dist/Pokemon Alpha Sapphire.app/Contents/MacOS/PokemonAlphaSapphire`
    if exe_dir.file_name().and_then(|s| s.to_str()) == Some("MacOS") {
        let native_bin = exe_dir.join("pokemon-alpha-sapphire");
        let app_bundle = exe_dir.parent()?.parent()?;
        let dist_dir = app_bundle.parent()?;
        // Chercher game_data à côté du .app ou dans Contents/Resources/game_data
        let external_data = dist_dir.join("game_data");
        let internal_data = exe_dir.parent()?.join("Resources").join("game_data");
        let data_root = if external_data.is_dir() {
            external_data
        } else {
            internal_data
        };
        return Some((native_bin, data_root));
    }

    // Cas 2 : Windows x86_64 / Linux x86_64 (`dist/Pokemon Alpha Sapphire/`)
    let native_bin = if cfg!(target_os = "windows") {
        exe_dir.join("pokemon-alpha-sapphire-bin.exe")
    } else {
        exe_dir.join("pokemon-alpha-sapphire-bin")
    };
    let data_root = exe_dir.join("game_data");
    Some((native_bin, data_root))
}

fn find_rom_asset(game_data_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(game_data_dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            if ext.eq_ignore_ascii_case("3ds")
                || ext.eq_ignore_ascii_case("cci")
                || ext.eq_ignore_ascii_case("cxi")
            {
                return Some(p);
            }
        }
    }
    None
}

fn main() {
    let Ok(exe) = env::current_exe() else {
        std::process::exit(1);
    };

    let Some((native_bin, game_data_dir)) = locate_paths(&exe) else {
        eprintln!("Structure du bundle invalide.");
        std::process::exit(1);
    };

    if !native_bin.is_file() {
        eprintln!(
            "Binaire natif recompilé introuvable : '{}'. Lancez `./blueorb app`.",
            native_bin.display()
        );
        std::process::exit(1);
    }

    let Some(rom_asset) = find_rom_asset(&game_data_dir) else {
        eprintln!(
            "Données de la ROM introuvables dans '{}'.",
            game_data_dir.display()
        );
        std::process::exit(1);
    };

    let user_save_dir = game_data_dir.join("userdata");
    let _ = fs::create_dir_all(&user_save_dir);
    let settings_file = user_save_dir.join("settings.toml");

    let mut cmd = Command::new(&native_bin);
    cmd.current_dir(&game_data_dir);
    cmd.env("ZAKURO_SETTINGS_PATH", &settings_file);
    cmd.arg(&rom_asset);
    cmd.arg("--new3ds");
    cmd.arg("--data").arg(&user_save_dir);

    let user_args: Vec<String> = env::args()
        .skip(1)
        .filter(|a| !a.starts_with("-psn_"))
        .collect();

    for arg in &user_args {
        cmd.arg(arg);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        eprintln!("Échec exec de '{}': {}", native_bin.display(), err);
        std::process::exit(1);
    }

    #[cfg(not(unix))]
    {
        match cmd.status() {
            Ok(st) => std::process::exit(st.code().unwrap_or(0)),
            Err(err) => {
                eprintln!("Échec lancement de '{}': {}", native_bin.display(), err);
                std::process::exit(1);
            }
        }
    }
}
