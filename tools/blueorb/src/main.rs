mod analyzer;
mod arm_disasm;
mod blz;
mod builder;
mod config;
mod cro;
mod differ;
mod enhancer;
mod icon;
mod rom3ds;
mod sha256;
mod splitter;
mod standalone;
mod toolchain;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::config::ProjectConfig;
use crate::standalone::{StandaloneOptions, TargetPlatform};

fn print_help() {
    println!(
        "BlueOrb v0.2.0 — Portage Standalone PC/Mac & Décompilation Pokémon Saphir Alpha
Compatible nativement : macOS (Apple Silicon ARM64 / Intel), Linux (x86_64), Windows (x86_64)

UTILISATION :
  blueorb <commande> [options]

COMMANDES JEU STANDALONE & INSTALLATEUR GUI :
  gui, installer [--no-launch]   Compile et ouvre l'Installateur Graphique (glisser-déposer la ROM .3ds -> tout se fait seul !)
                                   - macOS   : dist/BlueOrb Installer.app
                                   - Windows : dist/BlueOrb Installer/BlueOrb-Installer.exe
                                   - Linux   : dist/BlueOrb Installer/BlueOrb-Installer
  app [options]                  Crée l'application autonome prête à jouer dans dist/ :
                                   - macOS   : dist/Pokemon Alpha Sapphire.app
                                   - Windows : dist/Pokemon Alpha Sapphire/Pokemon Alpha Sapphire.exe
                                   - Linux   : dist/Pokemon Alpha Sapphire/PokemonAlphaSapphire
  play [options]                 Met à jour l'app autonome et lance Pokémon Saphir Alpha immédiatement !
    Options de `app` / `play` :
      --ultrawide, --21:9        Active le vrai rendu 3D Ultra-Wide 21:9 sans étirement
      --super-ultrawide, --32:9  Active le rendu Super Ultra-Wide 32:9
      --widescreen, --16:9       Active le rendu Widescreen 16:9
      --5:3, --console-aspect    Conserve le format 3DS classique 5:3 (par défaut)
      --dual-windows             Sépare l'écran du haut et l'écran tactile du bas sur 2 fenêtres !
      --res <1..10>              Facteur de résolution 3D interne GPU (défaut: 5 = QHD, 8 = 4K+)
      --60fps                    Active en plus le mode Turbo 2x au démarrage
      --with-outlines            Conserve les contours noirs 3DS d'origine (désactivés en HD par défaut)
      --portable                 Copie complète de la ROM dans le bundle (pour clé USB / autre PC)
      --lang <fr|en|ja>          Langue système du jeu (défaut: fr)

COMMANDES DÉCOMPILATION & RECOMPILATION :
  doctor                         Vérifie la toolchain ARM (as, ld, objcopy, clang++/g++) sur votre OS
  setup [--rom <fichier.3ds>]    Pipeline complet : extract -> analyze -> split -> build -> check
  extract [--rom <fichier.3ds>]  Extrait code.bin (BLZ), ExHeader, ExeFS et les 145 Dll*.cro
  analyze                        Analyse code.bin + static.crs + 145 Dll*.cro -> config/symbols.txt
  split                          Découpe code.bin en unités ASM GNU ARMv6K (asm/) + build/ldscript.ld
  build [--rebuild]              Compile en parallèle C++ (src/) + ASM (asm/) -> build/code.bin
  check                          Vérifie le SHA-256 bit-à-bit de build/code.bin
  pack [--out <rebuilt.3ds>]     Génère le mod LayeredFS et reconstruit une ROM .3ds
  progress                       Affiche la progression de la décompilation C++
  inspect <sym_ou_addr> [taille] Désassemble proprement une fonction
  diff <sym_ou_addr>             Compare original vs C++ instruction par instruction
  new-unit <sym_ou_addr> <nom>   Crée un squelette C++ dans src/<nom>.cpp
  cro [nom_module]               Liste ou inspecte les 145 modules dynamiques Dll*.cro
"
    );
}

fn find_project_root() -> PathBuf {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join("blueorb.toml").exists() {
        return cwd;
    }
    if let Some(parent) = cwd.parent().and_then(|p| p.parent()) {
        if parent.join("blueorb.toml").exists() {
            return parent.to_path_buf();
        }
    }
    cwd
}

fn parse_standalone_opts(args: &[String], root: &Path, cfg: &ProjectConfig) -> Result<StandaloneOptions, String> {
    let mut opts = StandaloneOptions::default_for(root, cfg);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--rom" if i + 1 < args.len() => {
                opts.rom_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--target" if i + 1 < args.len() => {
                opts.target = TargetPlatform::from_str(&args[i + 1])?;
                i += 2;
            }
            "--res" if i + 1 < args.len() => {
                let r = args[i + 1]
                    .parse::<u32>()
                    .map_err(|_| "Facteur --res invalide (entier de 1 à 10 attendu)".to_string())?;
                opts.resolution_factor = r.clamp(1, 10);
                opts.explicit_display_flags = true;
                i += 2;
            }
            "--ultrawide" | "--21:9" => {
                opts.top_aspect = "wide21x9".into();
                opts.explicit_display_flags = true;
                i += 1;
            }
            "--super-ultrawide" | "--32:9" => {
                opts.top_aspect = "wide32x9".into();
                opts.explicit_display_flags = true;
                i += 1;
            }
            "--widescreen" | "--16:9" => {
                opts.top_aspect = "wide16x9".into();
                opts.explicit_display_flags = true;
                i += 1;
            }
            "--console-aspect" | "--5:3" => {
                opts.top_aspect = "console".into();
                opts.explicit_display_flags = true;
                i += 1;
            }
            "--dual-windows" | "--two-windows" => {
                opts.dual_windows = true;
                opts.explicit_display_flags = true;
                i += 1;
            }
            "--single-window" | "--stacked" => {
                opts.dual_windows = false;
                opts.explicit_display_flags = true;
                i += 1;
            }
            "--60fps" => {
                opts.unlock_60fps = true;
                i += 1;
            }
            "--with-outlines" | "--outlines" => {
                opts.no_outlines = false;
                i += 1;
            }
            "--portable" => {
                opts.portable_copy = true;
                i += 1;
            }
            "--rebuild" | "-f" => {
                opts.force_recomp = true;
                i += 1;
            }
            "--windowed" | "-w" => {
                opts.windowed = true;
                i += 1;
            }
            "--lang" if i + 1 < args.len() => {
                opts.language = match args[i + 1].to_lowercase().as_str() {
                    "ja" | "jp" => 0,
                    "en" => 1,
                    "fr" => 2,
                    "de" => 3,
                    "it" => 4,
                    "es" => 5,
                    _ => 2,
                };
                i += 2;
            }
            _ => i += 1,
        }
    }
    Ok(opts)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" || args[0] == "help" {
        print_help();
        return Ok(());
    }

    let root = find_project_root();
    let cfg = ProjectConfig::load(&root.join("blueorb.toml"))?;
    let baserom_dir = root.join("baserom");
    let config_dir = root.join("config");
    let asm_dir = root.join("asm");
    let build_dir = root.join("build");

    match args[0].as_str() {
        "gui" | "installer" => {
            let launch = !args.iter().any(|a| a == "--no-launch" || a == "--build-only");
            standalone::build_and_launch_gui_installer(&root, launch)?;
        }
        "app" | "standalone" => {
            let opts = parse_standalone_opts(&args, &root, &cfg)?;
            standalone::build_standalone_app(&root, &cfg, &opts)?;
        }
        "play" | "run" => {
            let opts = parse_standalone_opts(&args, &root, &cfg)?;
            let bundle = standalone::build_standalone_app(&root, &cfg, &opts)?;
            if opts.target != TargetPlatform::host() {
                return Err(format!(
                    "Le package {} a été généré, mais ne peut être lancé que sur {}.",
                    bundle.display(),
                    opts.target.label()
                ));
            }
            standalone::launch_standalone_app(&bundle, opts.target, &opts)?;
        }
        "doctor" => {
            toolchain::run_doctor(
                &cfg.clang_cxx,
                &cfg.gcc_cxx,
                &cfg.armcc_cxx,
                &cfg.assembler,
                &cfg.linker,
                &cfg.objcopy,
                &cfg.objdump,
            )?;
        }
        "extract" => {
            let mut rom_path = root.join(&cfg.default_rom);
            let mut all_romfs = false;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--rom" if i + 1 < args.len() => {
                        rom_path = PathBuf::from(&args[i + 1]);
                        i += 2;
                    }
                    "--all-romfs" => {
                        all_romfs = true;
                        i += 1;
                    }
                    other if !other.starts_with('-') => {
                        rom_path = PathBuf::from(other);
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            rom3ds::extract_rom(&rom_path, &baserom_dir, &cfg, all_romfs)?;
        }
        "analyze" => {
            analyzer::analyze_baserom(&baserom_dir, &config_dir, &cfg)?;
        }
        "split" => {
            if !config_dir.join("symbols.txt").exists() {
                analyzer::analyze_baserom(&baserom_dir, &config_dir, &cfg)?;
            }
            splitter::split_baserom(&baserom_dir, &config_dir, &asm_dir, &build_dir, &cfg)?;
        }
        "build" => {
            let force = args.iter().any(|a| a == "--rebuild" || a == "-f");
            let matched = builder::build_project(&root, &cfg, force)?;
            if !matched && args.iter().any(|a| a == "--strict") {
                return Err("Le binaire généré ne correspond pas à 100% à la ROM originale.".into());
            }
        }
        "check" => {
            let syms = differ::merged_symbols(&root);
            let matched = builder::check_matching(&root, &cfg, &syms)?;
            if !matched {
                return Err("Échec de la vérification SHA-256 (NON-MATCHING).".into());
            }
        }
        "setup" => {
            let mut rom_path = root.join(&cfg.default_rom);
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--rom" && i + 1 < args.len() {
                    rom_path = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            println!("=== [1/4] Extraction de la ROM 3DS ===");
            rom3ds::extract_rom(&rom_path, &baserom_dir, &cfg, false)?;
            println!("\n=== [2/4] Analyse des symboles & des 145 modules CRO ===");
            analyzer::analyze_baserom(&baserom_dir, &config_dir, &cfg)?;
            println!("\n=== [3/4] Découpage ASM GNU ARMv6K & Liaison C++ ===");
            splitter::split_baserom(&baserom_dir, &config_dir, &asm_dir, &build_dir, &cfg)?;
            println!("\n=== [4/4] Compilation parallèle & Vérification SHA-256 ===");
            let matched = builder::build_project(&root, &cfg, true)?;
            if !matched {
                return Err("Le pipeline initial n'a pas produit un binaire 100% identique.".into());
            }
        }
        "pack" => {
            let mut orig_rom = root.join(&cfg.default_rom);
            let mut out_rom: Option<PathBuf> = Some(build_dir.join("Pokemon_Alpha_Sapphire_rebuilt.3ds"));
            let mut compress = true;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--rom" if i + 1 < args.len() => {
                        orig_rom = PathBuf::from(&args[i + 1]);
                        i += 2;
                    }
                    "--out" if i + 1 < args.len() => {
                        out_rom = Some(PathBuf::from(&args[i + 1]));
                        i += 2;
                    }
                    "--layeredfs-only" => {
                        out_rom = None;
                        i += 1;
                    }
                    "--uncompressed" => {
                        compress = false;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            let code_bin_path = build_dir.join("code.bin");
            if !code_bin_path.exists() {
                builder::build_project(&root, &cfg, false)?;
            }
            rom3ds::pack_rom(
                &code_bin_path,
                &baserom_dir,
                &orig_rom,
                out_rom.as_deref(),
                &build_dir,
                &cfg,
                compress,
            )?;
        }
        "inspect" => {
            if args.len() < 2 {
                return Err("Usage : blueorb inspect <nom_symbole_ou_adresse> [taille_octets]".into());
            }
            let len_opt = args.get(2).and_then(|s| config::parse_u32(s));
            differ::inspect_function(&root, &cfg, &args[1], len_opt)?;
        }
        "diff" => {
            if args.len() < 2 {
                return Err("Usage : blueorb diff <nom_symbole_ou_adresse>".into());
            }
            differ::diff_function(&root, &cfg, &args[1])?;
        }
        "new-unit" => {
            if args.len() < 3 {
                return Err("Usage : blueorb new-unit <nom_symbole_ou_adresse> <nom_fichier_cpp>".into());
            }
            differ::create_new_cpp_unit(&root, &cfg, &args[1], &args[2])?;
        }
        "progress" => {
            differ::show_progress(&root, &cfg)?;
        }
        "cro" | "cro-info" | "cro-list" => {
            differ::inspect_cro(&root, args.get(1).map(|s| s.as_str()))?;
        }
        other => {
            return Err(format!(
                "Commande inconnue '{}'. Lancez `blueorb --help` pour voir les commandes.",
                other
            ));
        }
    }

    Ok(())
}

fn main() -> ExitCode {
    let _ = Path::new(".");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("\n[erreur] {}", msg);
            ExitCode::FAILURE
        }
    }
}
