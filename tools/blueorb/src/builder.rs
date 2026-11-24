// Moteur de compilation parallèle incrémentale (C++ & ASM -> ELF -> code.bin) + vérification SHA-256

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::config::{load_symbol_table, CppUnit, ProjectConfig, SymbolEntry};
use crate::sha256::sha256_hex;
use crate::splitter::{compute_split_plan, write_linker_script, BuildSegment};
use crate::toolchain::ResolvedToolchain;

#[derive(Clone, Debug)]
enum CompileJob {
    Asm {
        src_path: PathBuf,
        obj_path: PathBuf,
        extra_dep: Option<PathBuf>,
    },
    Cpp {
        unit: CppUnit,
        src_path: PathBuf,
        obj_path: PathBuf,
    },
}

pub fn build_project(
    project_root: &Path,
    cfg: &ProjectConfig,
    force_rebuild: bool,
) -> Result<bool, String> {
    let t0 = Instant::now();
    let tc = ResolvedToolchain::resolve(
        &cfg.clang_cxx,
        &cfg.gcc_cxx,
        &cfg.armcc_cxx,
        &cfg.assembler,
        &cfg.linker,
        &cfg.objcopy,
        &cfg.objdump,
    )?;

    let config_dir = project_root.join("config");
    let build_dir = project_root.join("build");
    let obj_dir = build_dir.join("obj");
    fs::create_dir_all(&obj_dir).map_err(|e| e.to_string())?;

    let mut symbols = load_symbol_table(&config_dir.join("symbols.txt"));
    for (addr, usym) in load_symbol_table(&config_dir.join("user_symbols.txt")) {
        symbols.insert(addr, usym);
    }

    let plan = compute_split_plan(&config_dir, cfg, &symbols)?;
    let ld_path = build_dir.join("ldscript.ld");
    write_linker_script(&ld_path, &plan, cfg, &symbols)?;

    // Collecter tous les jobs de compilation (ASM + C++)
    let mut jobs = Vec::new();
    let baserom_code = project_root.join("baserom").join("code.bin");

    for seg in &plan.text_segments {
        match seg {
            BuildSegment::AsmText {
                asm_rel_path,
                obj_rel_path,
                ..
            } => {
                let src_p = rel_to_native(project_root, asm_rel_path);
                if !src_p.exists() {
                    return Err(format!(
                        "Fichier ASM '{}' manquant : exécutez d'abord `blueorb split`.",
                        src_p.display()
                    ));
                }
                jobs.push(CompileJob::Asm {
                    src_path: src_p,
                    obj_path: rel_to_native(project_root, obj_rel_path),
                    extra_dep: None,
                });
            }
            BuildSegment::CppText { unit, obj_rel_path } => {
                let src_p = rel_to_native(project_root, &unit.source);
                if !src_p.exists() {
                    return Err(format!(
                        "Fichier source C++ '{}' introuvable (référencé dans config/splits.toml).",
                        src_p.display()
                    ));
                }
                jobs.push(CompileJob::Cpp {
                    unit: unit.clone(),
                    src_path: src_p,
                    obj_path: rel_to_native(project_root, obj_rel_path),
                });
            }
        }
    }

    for (asm_rel, obj_rel) in plan.rodata_asm_files.iter().chain(plan.data_asm_files.iter()) {
        let src_p = rel_to_native(project_root, asm_rel);
        if !src_p.exists() {
            return Err(format!(
                "Fichier ASM '{}' manquant : exécutez d'abord `blueorb split`.",
                src_p.display()
            ));
        }
        jobs.push(CompileJob::Asm {
            src_path: src_p,
            obj_path: rel_to_native(project_root, obj_rel),
            extra_dep: Some(baserom_code.clone()),
        });
    }

    // Filtrer les jobs nécessitant une recompilation incrémentale
    let include_mtime = latest_mtime_in_dir(&project_root.join("include"));
    let pending: Vec<CompileJob> = jobs
        .into_iter()
        .filter(|job| force_rebuild || needs_recompile(job, include_mtime))
        .collect();

    let compiled_count = pending.len();
    if compiled_count > 0 {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(16);
        println!(
            "[build] Compilation de {} unité(s) sur {} threads ({})...",
            compiled_count,
            threads,
            ResolvedToolchain::host_summary()
        );

        let next_idx = AtomicUsize::new(0);
        let mut errors: Vec<String> = Vec::new();

        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for _ in 0..threads.min(compiled_count) {
                let pending_ref = &pending;
                let idx_ref = &next_idx;
                let tc_ref = &tc;
                handles.push(s.spawn(move || -> Result<(), String> {
                    loop {
                        let i = idx_ref.fetch_add(1, Ordering::Relaxed);
                        if i >= pending_ref.len() {
                            break;
                        }
                        run_compile_job(&pending_ref[i], project_root, cfg, tc_ref)?;
                    }
                    Ok(())
                }));
            }
            for h in handles {
                if let Err(e) = h.join().unwrap() {
                    errors.push(e);
                }
            }
        });

        if let Some(first_err) = errors.into_iter().next() {
            return Err(first_err);
        }
    } else {
        println!("[build] Tous les fichiers objets (.o) sont à jour.");
    }

    // Édition de liens (via build/ldscript.ld pour supporter des milliers d'objets même sous Windows x86_64)
    let elf_path = build_dir.join("code.elf");
    let map_path = build_dir.join("code.map");
    let out_bin_path = build_dir.join("code.bin");

    let ld_out = Command::new(&tc.linker)
        .current_dir(project_root)
        .arg("--use-blx")
        .arg("--no-warn-rwx-segments")
        .arg("-z")
        .arg("noexecstack")
        .arg("-T")
        .arg("build/ldscript.ld")
        .arg("-Map")
        .arg(&map_path)
        .arg("-o")
        .arg(&elf_path)
        .output()
        .map_err(|e| format!("Échec d'exécution du linker '{}': {}", tc.linker.display(), e))?;

    if !ld_out.status.success() {
        return Err(format!(
            "Échec de l'édition de liens (arm-none-eabi-ld) :\n{}",
            String::from_utf8_lossy(&ld_out.stderr)
        ));
    }

    // Extraction du binaire plat `build/code.bin`
    let oc_out = Command::new(&tc.objcopy)
        .current_dir(project_root)
        .arg("-O")
        .arg("binary")
        .arg(&elf_path)
        .arg(&out_bin_path)
        .output()
        .map_err(|e| format!("Échec d'exécution de objcopy: {}", e))?;

    if !oc_out.status.success() {
        return Err(format!(
            "Échec de arm-none-eabi-objcopy :\n{}",
            String::from_utf8_lossy(&oc_out.stderr)
        ));
    }

    let elapsed = t0.elapsed().as_secs_f64();
    println!(
        "[build] Binaire généré en {:.2}s -> {}",
        elapsed,
        out_bin_path.display()
    );

    check_matching(project_root, cfg, &symbols)
}

pub fn check_matching(
    project_root: &Path,
    cfg: &ProjectConfig,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> Result<bool, String> {
    let built_path = project_root.join("build").join("code.bin");
    let base_path = project_root.join("baserom").join("code.bin");

    let built = fs::read(&built_path)
        .map_err(|e| format!("Impossible de lire '{}': {}", built_path.display(), e))?;
    let built_sha = sha256_hex(&built);

    if !cfg.code_sha256.is_empty() && built_sha == cfg.code_sha256 {
        println!(
            "[check] MATCHING 100% OK ! SHA-256 = {} (0x{:X} octets)",
            built_sha,
            built.len()
        );
        return Ok(true);
    }

    if base_path.exists() {
        let base = fs::read(&base_path).map_err(|e| e.to_string())?;
        let base_sha = sha256_hex(&base);
        if built == base {
            println!(
                "[check] MATCHING 100% OK (identique à baserom/code.bin) ! SHA-256 = {}",
                built_sha
            );
            return Ok(true);
        }

        println!("[check] DIFFÉRENCE DÉTECTÉE (NON-MATCHING) :");
        println!("  Attendu : {} (0x{:X} octets)", base_sha, base.len());
        println!("  Obtenu  : {} (0x{:X} octets)", built_sha, built.len());

        let min_len = base.len().min(built.len());
        for i in 0..min_len {
            if base[i] != built[i] {
                let vma = offset_to_vma(i as u32, cfg);
                let sym_name = symbols
                    .range(..=vma)
                    .next_back()
                    .map(|(_, s)| s.name.as_str())
                    .unwrap_or("inconnu");
                println!(
                    "  Premier écart à l'offset fichier 0x{:06X} (VMA 0x{:08X}, dans `{}`) : base=0x{:02X} != build=0x{:02X}",
                    i, vma, sym_name, base[i], built[i]
                );
                println!("  Astuce : lancez `./blueorb diff {}` pour inspecter les instructions côte à côte.", sym_name);
                break;
            }
        }
        return Ok(false);
    }

    println!(
        "[check] SHA-256 généré : {} (attendu : {})",
        built_sha, cfg.code_sha256
    );
    Ok(false)
}

fn offset_to_vma(file_off: u32, cfg: &ProjectConfig) -> u32 {
    if file_off < cfg.text_padded {
        cfg.text_addr + file_off
    } else if file_off < cfg.text_padded + cfg.rodata_padded {
        cfg.rodata_addr + (file_off - cfg.text_padded)
    } else {
        cfg.data_addr + (file_off - cfg.text_padded - cfg.rodata_padded)
    }
}

fn run_compile_job(
    job: &CompileJob,
    project_root: &Path,
    cfg: &ProjectConfig,
    tc: &ResolvedToolchain,
) -> Result<(), String> {
    match job {
        CompileJob::Asm {
            src_path, obj_path, ..
        } => {
            let out = Command::new(&tc.assembler)
                .current_dir(project_root)
                .args(&cfg.as_flags)
                .arg(src_path)
                .arg("-o")
                .arg(obj_path)
                .output()
                .map_err(|e| format!("Échec d'exécution de l'assembleur: {}", e))?;
            if !out.status.success() {
                return Err(format!(
                    "Erreur d'assemblage dans '{}' :\n{}",
                    src_path.display(),
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            Ok(())
        }
        CompileJob::Cpp {
            unit,
            src_path,
            obj_path,
        } => {
            compile_cpp_unit(unit, src_path, obj_path, project_root, cfg, tc)
        }
    }
}

pub fn compile_cpp_unit(
    unit: &CppUnit,
    src_path: &Path,
    obj_path: &Path,
    project_root: &Path,
    cfg: &ProjectConfig,
    tc: &ResolvedToolchain,
) -> Result<(), String> {
    let requested = unit
        .compiler
        .as_deref()
        .unwrap_or(&cfg.default_compiler)
        .to_lowercase();

    // Sélection intelligente avec bascule automatique Clang++ <-> GCC selon l'OS hôte
    enum Choice<'a> {
        Clang(&'a Path),
        Gcc(&'a Path),
        Armcc(&'a Path),
    }

    let choice = match requested.as_str() {
        "armcc" => {
            if let Some(ref p) = tc.armcc_cxx {
                Choice::Armcc(p)
            } else if let Some(ref p) = tc.clang_cxx {
                Choice::Clang(p)
            } else if let Some(ref p) = tc.gcc_cxx {
                Choice::Gcc(p)
            } else {
                return Err("Aucun compilateur C++ disponible".into());
            }
        }
        "gcc" => {
            if let Some(ref p) = tc.gcc_cxx {
                Choice::Gcc(p)
            } else if let Some(ref p) = tc.clang_cxx {
                Choice::Clang(p)
            } else {
                return Err("Aucun compilateur C++ disponible".into());
            }
        }
        _ => {
            if let Some(ref p) = tc.clang_cxx {
                Choice::Clang(p)
            } else if let Some(ref p) = tc.gcc_cxx {
                Choice::Gcc(p)
            } else {
                return Err("Aucun compilateur C++ disponible".into());
            }
        }
    };

    let mut cmd = match choice {
        Choice::Clang(p) => {
            let mut c = Command::new(p);
            c.args(&cfg.clang_flags);
            c.args(&unit.flags);
            c
        }
        Choice::Gcc(p) => {
            let mut c = Command::new(p);
            c.args(&cfg.gcc_flags);
            c.args(&unit.flags);
            c.args(&unit.gcc_flags);
            c
        }
        Choice::Armcc(p) => {
            let mut c = Command::new(p);
            c.args(["--cpu=MPCore", "--apcs=/interwork", "-Otime", "-Iinclude"]);
            c.args(&unit.flags);
            c
        }
    };

    let out = cmd
        .current_dir(project_root)
        .arg("-c")
        .arg(src_path)
        .arg("-o")
        .arg(obj_path)
        .output()
        .map_err(|e| format!("Échec d'exécution du compilateur C++: {}", e))?;

    if !out.status.success() {
        return Err(format!(
            "Erreur de compilation C++ dans '{}' :\n{}",
            src_path.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn rel_to_native(root: &Path, slash_rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in slash_rel.split('/') {
        if !part.is_empty() {
            p.push(part);
        }
    }
    p
}

fn needs_recompile(job: &CompileJob, include_mtime: Option<std::time::SystemTime>) -> bool {
    let (src, obj, is_cpp, extra_dep) = match job {
        CompileJob::Asm {
            src_path,
            obj_path,
            extra_dep,
        } => (src_path, obj_path, false, extra_dep.as_deref()),
        CompileJob::Cpp {
            src_path, obj_path, ..
        } => (src_path, obj_path, true, None),
    };

    let Ok(obj_meta) = fs::metadata(obj) else {
        return true;
    };
    let Ok(obj_time) = obj_meta.modified() else {
        return true;
    };
    if let Ok(src_meta) = fs::metadata(src) {
        if let Ok(src_time) = src_meta.modified() {
            if src_time > obj_time {
                return true;
            }
        }
    }
    if let Some(dep) = extra_dep {
        if let Ok(dep_meta) = fs::metadata(dep) {
            if let Ok(dep_time) = dep_meta.modified() {
                if dep_time > obj_time {
                    return true;
                }
            }
        }
    }
    if is_cpp {
        if let Some(inc_t) = include_mtime {
            if inc_t > obj_time {
                return true;
            }
        }
    }
    false
}

fn latest_mtime_in_dir(dir: &Path) -> Option<std::time::SystemTime> {
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    let mut latest: Option<std::time::SystemTime> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(sub) = latest_mtime_in_dir(&p) {
                latest = Some(latest.map_or(sub, |cur| cur.max(sub)));
            }
        } else if let Ok(m) = entry.metadata().and_then(|m| m.modified()) {
            latest = Some(latest.map_or(m, |cur| cur.max(m)));
        }
    }
    latest
}
