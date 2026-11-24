mod analyzer;
mod arm_disasm;
mod blz;
mod builder;
mod config;
mod cro;
mod differ;
mod rom3ds;
mod sha256;
mod splitter;
mod toolchain;

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::config::ProjectConfig;

fn print_help() {
    println!(
        "BlueOrb v0.1.0 — Outil de décompilation matching & recompilation Pokémon Saphir Alpha (3DS)
Compatible nativement : macOS (Apple Silicon ARM64 / Intel), Linux (x86_64 / ARM64), Windows (x86_64)

UTILISATION :
  blueorb <commande> [options]

COMMANDES PRINCIPALES :
  doctor                         Vérifie la toolchain ARM (as, ld, objcopy, clang++/g++) sur votre OS
  setup [--rom <fichier.3ds>]    Pipeline complet : extract -> analyze -> split -> build -> check
  extract [--rom <fichier.3ds>]  Extrait code.bin (BLZ), ExHeader, ExeFS et les 145 Dll*.cro (--all-romfs pour tout extraire)
  analyze                        Analyse code.bin + static.crs + 145 Dll*.cro et génère config/symbols.txt
  split                          Découpe code.bin en unités ASM GNU ARMv6K (asm/) + génère build/ldscript.ld
  build [--rebuild]              Compile en parallèle C++ (src/) + ASM (asm/), lie code.elf et vérifie le SHA-256
  check                          Vérifie que build/code.bin est 100% identique bit-à-bit à baserom/code.bin
  pack [--out <rebuilt.3ds>]     Génère le mod LayeredFS (Azahar/Citra/Luma3DS) et reconstruit une ROM .3ds

COMMANDES D'AIDE À LA DÉCOMPILATION C++ :
  progress                       Affiche les statistiques et la barre de progression de la décompilation
  inspect <sym_ou_addr> [taille] Désassemble proprement une fonction avec pools résolus et références CRO
  diff <sym_ou_addr>             Compare côte-à-côte les instructions de baserom/code.bin et build/code.bin
  new-unit <sym_ou_addr> <nom>   Crée un fichier C++ dans src/<nom>.cpp et l'enregistre dans config/splits.toml
  cro [nom_module]               Liste les 145 overlays Dll*.cro ou inspecte un module précis (ex: DllBattle)
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
