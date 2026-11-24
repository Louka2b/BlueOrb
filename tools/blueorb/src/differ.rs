// Outils interactifs de décompilation : inspect, diff côte-à-côte, new-unit, cro-info et progress

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::arm_disasm::{decode_arm, decode_thumb};
use crate::config::{load_splits, load_symbol_table, parse_u32, ProjectConfig, SymMode, SymbolEntry};
use crate::cro::parse_cro_file;
use crate::sha256::sha256_hex;

#[inline]
fn u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

#[inline]
fn u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}

pub fn merged_symbols(project_root: &Path) -> BTreeMap<u32, SymbolEntry> {
    let config_dir = project_root.join("config");
    let mut syms = load_symbol_table(&config_dir.join("symbols.txt"));
    for (addr, usym) in load_symbol_table(&config_dir.join("user_symbols.txt")) {
        match syms.get_mut(&addr) {
            Some(existing) => {
                existing.name = usym.name;
                existing.mode = usym.mode;
            }
            None => {
                syms.insert(addr, usym);
            }
        }
    }
    syms
}

pub fn resolve_target(
    query: &str,
    symbols: &BTreeMap<u32, SymbolEntry>,
    cfg: &ProjectConfig,
) -> Result<(u32, u32, SymMode, String), String> {
    let addr_opt = parse_u32(query).or_else(|| {
        // Recherche exacte puis partielle sur le nom du symbole
        symbols
            .values()
            .find(|s| s.name == query)
            .or_else(|| symbols.values().find(|s| s.name.contains(query)))
            .map(|s| s.addr)
    });

    let Some(raw_addr) = addr_opt else {
        return Err(format!(
            "Symbole ou adresse '{}' introuvable dans config/symbols.txt",
            query
        ));
    };

    let addr = raw_addr & !1;
    let text_end = cfg.text_addr + cfg.text_size;
    if addr < cfg.text_addr || addr >= text_end {
        return Err(format!(
            "Adresse 0x{:08X} hors de la section .text (0x{:08X}..0x{:08X})",
            addr, cfg.text_addr, text_end
        ));
    }

    let (mode, name) = if let Some(sym) = symbols.get(&addr) {
        (sym.mode, sym.name.clone())
    } else if (raw_addr & 1) != 0 {
        (SymMode::Thumb, format!("sub_{:08X}", addr))
    } else {
        (SymMode::Arm, format!("sub_{:08X}", addr))
    };

    let next_addr = symbols
        .range((addr + 2)..)
        .next()
        .map(|(&a, _)| a)
        .unwrap_or((addr + 0x100).min(text_end));
    let end = next_addr.min(addr + 0x800).min(text_end);

    Ok((addr, end, mode, name))
}

pub fn inspect_function(
    project_root: &Path,
    cfg: &ProjectConfig,
    query: &str,
    override_len: Option<u32>,
) -> Result<(), String> {
    let code = fs::read(project_root.join("baserom").join("code.bin"))
        .map_err(|e| format!("Impossible de lire baserom/code.bin: {}", e))?;
    let symbols = merged_symbols(project_root);
    let (start, auto_end, mode, name) = resolve_target(query, &symbols, cfg)?;
    let end = override_len
        .map(|l| (start + l).min(cfg.text_addr + cfg.text_size))
        .unwrap_or(auto_end);

    println!("================================================================");
    println!(
        " Fonction : {}  (0x{:08X} .. 0x{:08X}, {} octets, mode {})",
        name,
        start,
        end,
        end - start,
        mode.as_str().to_uppercase()
    );
    if let Some(sym) = symbols.get(&start) {
        if let Some(ref dem) = sym.demangled {
            println!(" C++      : {}", dem);
        }
        if !sym.cro_users.is_empty() {
            println!(" Importé  : {}", sym.cro_users.join(", "));
        }
    }
    println!("================================================================");

    let rendered = render_clean_disasm(&code, start, end, mode, cfg, &symbols);
    print!("{}", rendered);
    Ok(())
}

pub fn render_clean_disasm(
    code: &[u8],
    start: u32,
    end: u32,
    mode: SymMode,
    cfg: &ProjectConfig,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> String {
    // Pré-passe pour collecter les étiquettes locales de saut
    let mut local_labels = BTreeSet::new();
    let noop_lookup = |_: u32| -> Option<String> { None };
    let mut pc = start;
    while pc < end {
        let off = (pc - cfg.text_addr) as usize;
        let dec = if mode == SymMode::Thumb {
            let hw1 = u16_le(code, off);
            let hw2 = if pc + 4 <= end {
                Some(u16_le(code, off + 2))
            } else {
                None
            };
            decode_thumb(pc, hw1, hw2, noop_lookup)
        } else {
            if pc + 4 > end {
                break;
            }
            decode_arm(pc, u32_le(code, off), noop_lookup)
        };
        if let Some((target, _)) = dec.branch_target {
            if target >= start && target < end && !symbols.contains_key(&target) {
                local_labels.insert(target);
            }
        }
        pc += dec.size;
    }

    let sym_lookup = |target: u32| -> Option<String> {
        if let Some(s) = symbols.get(&target) {
            Some(s.name.clone())
        } else if local_labels.contains(&target) {
            Some(format!(".L_{:08X}", target))
        } else {
            None
        }
    };

    let mut out = String::new();
    pc = start;
    while pc < end {
        if local_labels.contains(&pc) {
            let _ = writeln!(out, ".L_{:08X}:", pc);
        }
        let off = (pc - cfg.text_addr) as usize;
        if mode == SymMode::Thumb {
            let hw1 = u16_le(code, off);
            let hw2 = if pc + 4 <= end && !local_labels.contains(&(pc + 2)) {
                Some(u16_le(code, off + 2))
            } else {
                None
            };
            let dec = decode_thumb(pc, hw1, hw2, sym_lookup);
            let extra = describe_pool_target(dec.pool_load_addr, code, cfg, symbols);
            if dec.size == 4 {
                let _ = writeln!(
                    out,
                    "  {:08X}: {:04X} {:04X}    {}{}",
                    pc,
                    hw1,
                    hw2.unwrap(),
                    dec.text,
                    extra
                );
            } else {
                let _ = writeln!(out, "  {:08X}: {:04X}         {}{}", pc, hw1, dec.text, extra);
            }
            pc += dec.size;
        } else {
            if pc + 4 > end {
                break;
            }
            let w = u32_le(code, off);
            let dec = decode_arm(pc, w, sym_lookup);
            let extra = describe_pool_target(dec.pool_load_addr, code, cfg, symbols);
            let _ = writeln!(out, "  {:08X}: {:08X}     {}{}", pc, w, dec.text, extra);
            pc += 4;
        }
    }
    out
}

fn describe_pool_target(
    pool_addr: Option<u32>,
    code: &[u8],
    cfg: &ProjectConfig,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> String {
    let Some(pa) = pool_addr else {
        return String::new();
    };
    if pa < cfg.text_addr || pa + 4 > cfg.text_addr + cfg.text_size {
        return String::new();
    }
    let val = u32_le(code, (pa - cfg.text_addr) as usize);
    if let Some(sym) = symbols.get(&val).or_else(|| symbols.get(&(val & !1))) {
        return format!("  ; =0x{:08X} ({})", val, sym.name);
    }
    // Vérifier si `val` pointe vers une chaîne ASCII dans .rodata
    if val >= cfg.rodata_addr && val < cfg.rodata_addr + cfg.rodata_size {
        let ro_off = (cfg.text_padded + (val - cfg.rodata_addr)) as usize;
        if ro_off < code.len() {
            let preview = extract_ascii_preview(&code[ro_off..], 32);
            if preview.len() >= 3 {
                return format!("  ; =0x{:08X} (\"{}\")", val, preview);
            }
        }
    }
    format!("  ; =0x{:08X}", val)
}

fn extract_ascii_preview(slice: &[u8], max_len: usize) -> String {
    let mut s = String::new();
    for &b in slice.iter().take(max_len) {
        if b == 0 {
            break;
        }
        if (0x20..=0x7E).contains(&b) {
            s.push(b as char);
        } else {
            return String::new();
        }
    }
    s
}

pub fn diff_function(
    project_root: &Path,
    cfg: &ProjectConfig,
    query: &str,
) -> Result<(), String> {
    let base = fs::read(project_root.join("baserom").join("code.bin"))
        .map_err(|e| format!("Impossible de lire baserom/code.bin: {}", e))?;
    let built = fs::read(project_root.join("build").join("code.bin")).map_err(|e| {
        format!(
            "Impossible de lire build/code.bin (lancez d'abord `blueorb build`): {}",
            e
        )
    })?;

    let symbols = merged_symbols(project_root);
    let (start, end, mode, name) = resolve_target(query, &symbols, cfg)?;

    println!(
        "=== Comparaison ASM : {} (0x{:08X}..0x{:08X}) ===",
        name, start, end
    );
    println!(
        "{:10} | {:36} | {:36}",
        "Adresse", "ORIGINAL (baserom/code.bin)", "RECOMPILÉ (build/code.bin)"
    );
    println!("{}", "-".repeat(90));

    let sym_lookup = |addr: u32| -> Option<String> { symbols.get(&addr).map(|s| s.name.clone()) };

    let mut pc = start;
    let mut diffs = 0usize;
    while pc < end {
        let off = (pc - cfg.text_addr) as usize;
        if mode == SymMode::Thumb {
            let b_hw1 = u16_le(&base, off);
            let c_hw1 = u16_le(&built, off);
            let b_dec = decode_thumb(pc, b_hw1, (pc + 4 <= end).then(|| u16_le(&base, off + 2)), sym_lookup);
            let c_dec = decode_thumb(pc, c_hw1, (pc + 4 <= end).then(|| u16_le(&built, off + 2)), sym_lookup);
            let step = b_dec.size.min(c_dec.size);
            let same = base[off..off + step as usize] == built[off..off + step as usize];
            if !same {
                diffs += 1;
            }
            let mark = if same { " " } else { "!" };
            println!(
                "{} {:08X} | {:04X} {:31} | {:04X} {:31}",
                mark, pc, b_hw1, b_dec.text, c_hw1, c_dec.text
            );
            pc += step;
        } else {
            let bw = u32_le(&base, off);
            let cw = u32_le(&built, off);
            let b_dec = decode_arm(pc, bw, sym_lookup);
            let c_dec = decode_arm(pc, cw, sym_lookup);
            let same = bw == cw;
            if !same {
                diffs += 1;
            }
            let mark = if same { " " } else { "!" };
            println!(
                "{} {:08X} | {:08X} {:27} | {:08X} {:27}",
                mark, pc, bw, b_dec.text, cw, c_dec.text
            );
            pc += 4;
        }
    }

    println!("{}", "-".repeat(90));
    if diffs == 0 {
        println!("Résultat : 100% MATCHING sur `{}` !", name);
    } else {
        println!("Résultat : {} instruction(s) différente(s) sur `{}`.", diffs, name);
    }
    Ok(())
}

pub fn create_new_cpp_unit(
    project_root: &Path,
    cfg: &ProjectConfig,
    query: &str,
    unit_stem: &str,
) -> Result<(), String> {
    let code = fs::read(project_root.join("baserom").join("code.bin"))
        .map_err(|e| format!("Impossible de lire baserom/code.bin: {}", e))?;
    let symbols = merged_symbols(project_root);
    let (raw_start, raw_end, mode, name) = resolve_target(query, &symbols, cfg)?;

    let start = raw_start & !3;
    let end = (raw_end + 3) & !3;

    let clean_stem = unit_stem
        .trim_end_matches(".cpp")
        .trim_start_matches("src/");
    let rel_source = format!("src/{}.cpp", clean_stem);
    let full_source = project_root.join("src").join(format!("{}.cpp", clean_stem));

    if full_source.exists() {
        return Err(format!("Le fichier '{}' existe déjà.", full_source.display()));
    }

    let existing_units = load_splits(&project_root.join("config").join("splits.toml"))?;
    for u in &existing_units {
        if start < u.end && end > u.start {
            return Err(format!(
                "La plage 0x{:08X}..0x{:08X} chevauche déjà '{}' (0x{:08X}..0x{:08X})",
                start, end, u.source, u.start, u.end
            ));
        }
    }

    let disasm = render_clean_disasm(&code, start, end, mode, cfg, &symbols);
    let mode_flag = if mode == SymMode::Thumb {
        "-mthumb"
    } else {
        "-marm"
    };

    let mut cpp = String::new();
    cpp.push_str("#include \"types.hpp\"\n\n");
    let _ = writeln!(
        cpp,
        "// Unité générée par `blueorb new-unit` — Plage: 0x{:08X} .. 0x{:08X} ({} octets)",
        start,
        end,
        end - start
    );
    cpp.push_str("/*\nAssembleur de référence :\n");
    cpp.push_str(&disasm);
    cpp.push_str("*/\n\n");
    let _ = writeln!(cpp, "BLUEORB_EXTERN_C void {}(void) {{", name);
    cpp.push_str("    // TODO: Implémenter en C/C++ et vérifier avec `./blueorb diff`\n");
    cpp.push_str("}\n");

    fs::create_dir_all(project_root.join("src")).map_err(|e| e.to_string())?;
    fs::write(&full_source, cpp).map_err(|e| e.to_string())?;

    let splits_path = project_root.join("config").join("splits.toml");
    let mut splits_txt = fs::read_to_string(&splits_path).unwrap_or_default();
    if !splits_txt.ends_with('\n') && !splits_txt.is_empty() {
        splits_txt.push('\n');
    }
    let _ = writeln!(
        splits_txt,
        "\n[[unit]]\nstart  = 0x{:08X}\nend    = 0x{:08X}\nsource = \"{}\"\nflags  = [\"{}\", \"-O2\"]",
        start, end, rel_source, mode_flag
    );
    fs::write(&splits_path, splits_txt).map_err(|e| e.to_string())?;

    println!(
        "[new-unit] Squelette C++ créé : {} (0x{:08X}..0x{:08X})",
        full_source.display(),
        start,
        end
    );
    println!(
        "[new-unit] Ajouté à config/splits.toml. Lancez ensuite : `./blueorb split && ./blueorb build`"
    );
    Ok(())
}

pub fn show_progress(project_root: &Path, cfg: &ProjectConfig) -> Result<(), String> {
    let symbols = merged_symbols(project_root);
    let units = load_splits(&project_root.join("config").join("splits.toml"))?;

    let total_text = cfg.text_size as u64;
    let cpp_bytes: u64 = units.iter().map(|u| (u.end - u.start) as u64).sum();
    let pct = (cpp_bytes as f64) * 100.0 / (total_text as f64);

    let arm_funcs = symbols.values().filter(|s| s.mode == SymMode::Arm).count();
    let thumb_funcs = symbols.values().filter(|s| s.mode == SymMode::Thumb).count();
    let cro_linked = symbols
        .values()
        .filter(|s| !s.cro_users.is_empty() && s.mode != SymMode::Data)
        .count();

    let built_path = project_root.join("build").join("code.bin");
    let matching_status = if built_path.exists() {
        let sha = sha256_hex(&fs::read(&built_path).map_err(|e| e.to_string())?);
        if sha == cfg.code_sha256 {
            "100% MATCHING (SHA-256 vérifié)"
        } else {
            "NON-MATCHING (modifications en cours)"
        }
    } else {
        "Non compilé (`./blueorb build`)"
    };

    let bar_width = 40usize;
    let filled = ((pct / 100.0) * (bar_width as f64)).round() as usize;
    let bar = format!(
        "[{}{}]",
        "#".repeat(filled.min(bar_width)),
        "-".repeat(bar_width.saturating_sub(filled))
    );

    println!("=== Progression BlueOrb — {} ({}) ===", cfg.game, cfg.product_code);
    println!("  État du build    : {}", matching_status);
    println!("  Taille .text     : {} octets ({:.2} Mio)", total_text, total_text as f64 / (1024.0 * 1024.0));
    println!("  Fonctions .text  : {} ({} ARM, {} Thumb)", arm_funcs + thumb_funcs, arm_funcs, thumb_funcs);
    println!("  Interfaces CRO   : {} fonctions liées aux 145 Dll*.cro", cro_linked);
    println!("  Unités C++ (src/): {} fichiers ({} octets, {:.4}%)", units.len(), cpp_bytes, pct);
    println!("  {}", bar);
    Ok(())
}

pub fn inspect_cro(project_root: &Path, name_filter: Option<&str>) -> Result<(), String> {
    let cro_dir = project_root.join("baserom").join("cro");
    if !cro_dir.is_dir() {
        return Err("Dossier baserom/cro introuvable (lancez d'abord `./blueorb extract`)".into());
    }

    let mut files: Vec<_> = fs::read_dir(&cro_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|x| x.to_string_lossy().eq_ignore_ascii_case("cro"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();

    if let Some(q) = name_filter {
        let q_low = q.to_lowercase();
        let target = files
            .into_iter()
            .find(|p| p.file_name().unwrap().to_string_lossy().to_lowercase().contains(&q_low))
            .ok_or_else(|| format!("Aucun module CRO correspondant à '{}'", q))?;
        let info = parse_cro_file(&target)?;
        println!("=== Module CRO : {} ({}) ===", info.module_name, target.display());
        println!("  Code : offset=0x{:X}, taille=0x{:X} ({} octets)", info.code_offset, info.code_size, info.code_size);
        println!("  Imports depuis code.bin (static.crs) : {} symboles", info.static_anon_imports.len());
        println!("  Exports nommés ({}) :", info.named_exports.len());
        for exp in &info.named_exports {
            if let Some(ref dem) = exp.demangled {
                println!("    seg[{}]+0x{:06X}  {}  ->  {}", exp.seg_idx, exp.seg_offset, exp.name, dem);
            } else {
                println!("    seg[{}]+0x{:06X}  {}", exp.seg_idx, exp.seg_offset, exp.name);
            }
        }
    } else {
        println!("=== {} Modules relogeables CRO dans Pokémon Saphir Alpha ===", files.len());
        println!("{:34} | {:>10} | {:>12} | {:>8}", "Module", ".text (B)", "Imports code", "Exports");
        println!("{}", "-".repeat(74));
        for p in files.iter().take(40) {
            if let Ok(info) = parse_cro_file(p) {
                let fname = p.file_name().unwrap().to_string_lossy();
                println!(
                    "{:34} | 0x{:08X} | {:12} | {:8}",
                    fname,
                    info.code_size,
                    info.static_anon_imports.len(),
                    info.named_exports.len()
                );
            }
        }
        if files.len() > 40 {
            println!("... ({} autres modules — utilisez `./blueorb cro <NomModule>` pour le détail)", files.len() - 40);
        }
    }
    Ok(())
}
