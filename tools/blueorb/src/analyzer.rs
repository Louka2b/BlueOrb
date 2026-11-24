// Découverte automatique des fonctions, propagation ARM/Thumb et croisement CRO0 -> config/symbols.txt

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::config::{load_symbol_table, ProjectConfig, SymMode, SymbolEntry};
use crate::cro::parse_cro_file;

#[inline]
fn u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

#[inline]
fn u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}

pub fn analyze_baserom(
    baserom_dir: &Path,
    config_dir: &Path,
    cfg: &ProjectConfig,
) -> Result<usize, String> {
    let code_path = baserom_dir.join("code.bin");
    let code = fs::read(&code_path)
        .map_err(|e| format!("Impossible de lire '{}': {} (lancez d'abord `extract`)", code_path.display(), e))?;

    let text_start = cfg.text_addr;
    let text_end = cfg.text_addr + cfg.text_size;
    let rodata_start = cfg.rodata_addr;
    let _rodata_end = cfg.rodata_addr + cfg.rodata_size;
    let data_start = cfg.data_addr;
    let bss_end = cfg.bss_addr + cfg.bss_size;

    let mut symbols: BTreeMap<u32, SymbolEntry> = BTreeMap::new();

    // 1. Point d'entrée principal
    symbols.insert(
        text_start,
        SymbolEntry {
            addr: text_start,
            mode: SymMode::Arm,
            name: "__ctr_start".into(),
            demangled: None,
            cro_users: Vec::new(),
        },
    );

    // 2. Analyse de static.crs (exports nommés C++ du moteur principal)
    let static_crs_path = baserom_dir.join("static.crs");
    let mut seg_bases = vec![text_start, rodata_start, data_start];
    if static_crs_path.exists() {
        let crs = parse_cro_file(&static_crs_path)?;
        seg_bases = crs.segments.iter().map(|s| s.offset).collect();
        for exp in crs.named_exports {
            if let Some(&base) = seg_bases.get(exp.seg_idx as usize) {
                if base == 0 {
                    continue;
                }
                let is_text = exp.seg_idx == 0;
                let (addr, mode) = if is_text {
                    let is_thumb = (exp.seg_offset & 1) != 0;
                    (
                        base + (exp.seg_offset & !1),
                        if is_thumb { SymMode::Thumb } else { SymMode::Arm },
                    )
                } else {
                    (base + exp.seg_offset, SymMode::Data)
                };
                // Certaines adresses ont C1 et C2 au même offset : garder le premier symbole canonique
                symbols.entry(addr).or_insert(SymbolEntry {
                    addr,
                    mode,
                    name: exp.name,
                    demangled: exp.demangled,
                    cro_users: vec!["static.crs".into()],
                });
            }
        }
    }

    // 3. Analyse des 145 modules Dll*.cro pour récupérer les 5552 adresses exactes importées dans code.bin
    let cro_dir = baserom_dir.join("cro");
    let mut cro_module_count = 0usize;
    if cro_dir.is_dir() {
        let mut cro_files: Vec<_> = fs::read_dir(&cro_dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case("cro"))
                    .unwrap_or(false)
            })
            .collect();
        cro_files.sort();

        for cro_path in cro_files {
            let Ok(cro) = parse_cro_file(&cro_path) else {
                continue;
            };
            cro_module_count += 1;
            let short_mod = cro_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(cro.module_name);

            for (seg_idx, seg_off) in cro.static_anon_imports {
                let Some(&base) = seg_bases.get(seg_idx as usize) else {
                    continue;
                };
                if base == 0 {
                    continue;
                }
                if seg_idx == 0 {
                    let is_thumb = (seg_off & 1) != 0;
                    let addr = base + (seg_off & !1);
                    if addr >= text_start && addr < text_end {
                        let mode = if is_thumb { SymMode::Thumb } else { SymMode::Arm };
                        let entry = symbols.entry(addr).or_insert_with(|| SymbolEntry {
                            addr,
                            mode,
                            name: format!("func_{:08X}", addr),
                            demangled: None,
                            cro_users: Vec::new(),
                        });
                        if entry.cro_users.len() < 6 && !entry.cro_users.contains(&short_mod) {
                            entry.cro_users.push(short_mod.clone());
                        }
                    }
                } else {
                    let addr = base + seg_off;
                    if addr >= rodata_start && addr < bss_end {
                        let entry = symbols.entry(addr).or_insert_with(|| SymbolEntry {
                            addr,
                            mode: SymMode::Data,
                            name: if addr < data_start {
                                format!("rodata_{:08X}", addr)
                            } else if addr < cfg.bss_addr {
                                format!("data_{:08X}", addr)
                            } else {
                                format!("bss_{:08X}", addr)
                            },
                            demangled: None,
                            cro_users: Vec::new(),
                        });
                        if entry.cro_users.len() < 6 && !entry.cro_users.contains(&short_mod) {
                            entry.cro_users.push(short_mod.clone());
                        }
                    }
                }
            }
        }
    }

    // 4. Balayage des veneers RVCT ARM->Thumb (`add ip, pc, #1; bx ip` = 0xE28FC001, 0xE12FFF1C)
    let text_bytes = (cfg.text_size as usize).min(code.len());
    let mut off = 0usize;
    while off + 8 <= text_bytes {
        let w0 = u32_le(&code, off);
        let w1 = u32_le(&code, off + 4);
        if w0 == 0xE28F_C001 && w1 == 0xE12F_FF1C {
            let veneer_addr = text_start + (off as u32);
            let thumb_target = veneer_addr + 8;
            symbols.entry(veneer_addr).or_insert_with(|| SymbolEntry {
                addr: veneer_addr,
                mode: SymMode::Arm,
                name: format!("veneer_{:08X}", veneer_addr),
                demangled: None,
                cro_users: Vec::new(),
            });
            if thumb_target < text_end {
                symbols.entry(thumb_target).or_insert_with(|| SymbolEntry {
                    addr: thumb_target,
                    mode: SymMode::Thumb,
                    name: format!("func_{:08X}", thumb_target),
                    demangled: None,
                    cro_users: Vec::new(),
                });
            }
        }
        off += 4;
    }

    // 5. Balayage des appels ARM BL / BLX dans les zones ARM probables
    // Un appel ARM `EBxxxxxx` (BL) ou `FAxxxxxx`/`FBxxxxxx` (BLX) entouré d'instructions ARM valides
    let mut discovered_arm: BTreeSet<u32> = BTreeSet::new();
    let mut discovered_thumb: BTreeSet<u32> = BTreeSet::new();

    // D'abord, collecter les plages connues Thumb pour éviter de scanner les mots Thumb comme des BL ARM
    let mut thumb_ranges: Vec<(u32, u32)> = Vec::new();
    {
        let keys: Vec<(u32, SymMode)> = symbols.iter().map(|(&a, s)| (a, s.mode)).collect();
        for i in 0..keys.len() {
            if keys[i].1 == SymMode::Thumb {
                let start = keys[i].0;
                let end = keys.get(i + 1).map(|x| x.0).unwrap_or(text_start + 0x10000).min(start + 0x4000);
                thumb_ranges.push((start, end));
            }
        }
    }

    let is_in_thumb_range = |addr: u32| -> bool {
        thumb_ranges
            .binary_search_by(|&(s, e)| {
                if addr < s {
                    std::cmp::Ordering::Greater
                } else if addr >= e {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    };

    let mut pos = 0usize;
    while pos + 4 <= text_bytes {
        let pc = text_start + (pos as u32);
        if !is_in_thumb_range(pc) {
            let w = u32_le(&code, pos);
            // ARM BL inconditionnel (0xEBxxxxxx)
            if (w & 0xFF00_0000) == 0xEB00_0000 {
                let imm24 = w & 0x00FF_FFFF;
                let rel = ((imm24 as i32) << 8) >> 6;
                let target = (pc.wrapping_add(8) as i32).wrapping_add(rel) as u32;
                if target >= text_start && target + 4 <= text_end && (target & 3) == 0 {
                    let tw = u32_le(&code, (target - text_start) as usize);
                    if looks_like_arm_entry(tw) {
                        discovered_arm.insert(target);
                    }
                }
            } else if (w & 0xFE00_0000) == 0xFA00_0000 {
                // ARM BLX vers Thumb
                let h = ((w >> 24) & 1) << 1;
                let imm24 = w & 0x00FF_FFFF;
                let rel = ((imm24 as i32) << 8) >> 6;
                let target = ((pc.wrapping_add(8) as i32).wrapping_add(rel) as u32) | h;
                if target >= text_start && target + 2 <= text_end {
                    let thw = u16_le(&code, (target - text_start) as usize);
                    if looks_like_thumb_entry(thw) {
                        discovered_thumb.insert(target);
                    }
                }
            }
        } else {
            // Dans une zone Thumb connue, scanner les appels Thumb BL / BLX 32 bits
            for sub in [0usize, 2usize] {
                if pos + sub + 4 <= text_bytes {
                    let tpc = text_start + ((pos + sub) as u32);
                    let hw1 = u16_le(&code, pos + sub);
                    let hw2 = u16_le(&code, pos + sub + 2);
                    if (hw1 & 0xF800) == 0xF000 && ((hw2 & 0xD000) == 0xD000 || (hw2 & 0xD001) == 0xC000) {
                        let is_blx = (hw2 & 0x1000) == 0;
                        let s = ((hw1 >> 10) & 1) as i32;
                        let j1 = ((hw2 >> 13) & 1) as i32;
                        let j2 = ((hw2 >> 11) & 1) as i32;
                        let i1 = (!(j1 ^ s)) & 1;
                        let i2 = (!(j2 ^ s)) & 1;
                        let raw = (s << 24)
                            | (i1 << 23)
                            | (i2 << 22)
                            | (((hw1 & 0x03FF) as i32) << 12)
                            | (((hw2 & 0x07FF) as i32) << 1);
                        let signed_off = (raw << 7) >> 7;
                        let base_pc = if is_blx { (tpc + 4) & !3 } else { tpc + 4 };
                        let target = (base_pc as i32).wrapping_add(signed_off) as u32;
                        if target >= text_start && target + 4 <= text_end {
                            if is_blx && (target & 3) == 0 {
                                discovered_arm.insert(target);
                            } else if !is_blx {
                                discovered_thumb.insert(target);
                            }
                        }
                    }
                }
            }
        }
        pos += 4;
    }

    for addr in discovered_arm {
        symbols.entry(addr).or_insert_with(|| SymbolEntry {
            addr,
            mode: SymMode::Arm,
            name: format!("func_{:08X}", addr),
            demangled: None,
            cro_users: Vec::new(),
        });
    }
    for addr in discovered_thumb {
        symbols.entry(addr).or_insert_with(|| SymbolEntry {
            addr,
            mode: SymMode::Thumb,
            name: format!("func_{:08X}", addr),
            demangled: None,
            cro_users: Vec::new(),
        });
    }

    // 6. Appliquer les renommages prioritaires de l'utilisateur (`config/user_symbols.txt`)
    let user_syms_path = config_dir.join("user_symbols.txt");
    for (addr, usym) in load_symbol_table(&user_syms_path) {
        match symbols.get_mut(&addr) {
            Some(existing) => {
                existing.name = usym.name;
                existing.mode = usym.mode;
            }
            None => {
                symbols.insert(addr, usym);
            }
        }
    }

    // 7. Sauvegarde de `config/symbols.txt` (fins de ligne LF propres)
    fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let out_path = config_dir.join("symbols.txt");
    let mut out = String::with_capacity(symbols.len() * 64);
    out.push_str("# Base de symboles générée par `blueorb analyze` pour Pokémon Saphir Alpha (sango-2)\n");
    out.push_str("# Pour renommer durablement une fonction, ajoutez-la dans `config/user_symbols.txt`.\n\n");

    let mut arm_fn = 0usize;
    let mut thumb_fn = 0usize;
    let mut data_sym = 0usize;

    for sym in symbols.values() {
        match sym.mode {
            SymMode::Arm => arm_fn += 1,
            SymMode::Thumb => thumb_fn += 1,
            SymMode::Data => data_sym += 1,
        }
        use std::fmt::Write;
        let _ = write!(out, "0x{:08X} {:5} {}", sym.addr, sym.mode.as_str(), sym.name);
        if !sym.cro_users.is_empty() || sym.demangled.is_some() {
            out.push_str(" ; ");
            let mut first = true;
            if !sym.cro_users.is_empty() {
                let _ = write!(out, "cro={}", sym.cro_users.join(","));
                first = false;
            }
            if let Some(ref dem) = sym.demangled {
                if !first {
                    out.push_str(" | ");
                }
                let _ = write!(out, "demangled={}", dem);
            }
        }
        out.push('\n');
    }

    fs::write(&out_path, out).map_err(|e| e.to_string())?;
    println!(
        "[analyze] {} modules CRO croisés | {} symboles totaux ({} fonctions ARM, {} fonctions Thumb, {} données) -> {}",
        cro_module_count,
        symbols.len(),
        arm_fn,
        thumb_fn,
        data_sym,
        out_path.display()
    );
    Ok(symbols.len())
}

fn looks_like_arm_entry(w: u32) -> bool {
    let cond = (w >> 28) & 0xF;
    if cond != 0xE {
        return false;
    }
    // Prologue PUSH {.., lr}, SUB sp, sp, #.., MOV, LDR, CMP, BX LR, SVC
    let hi = (w >> 20) & 0xFF;
    matches!(
        hi,
        0x92 // stmfd sp!, {...}
        | 0x52 // str rx, [sp, #-4]!
        | 0x24 | 0x28 // sub/add
        | 0x1A | 0x3A | 0x3E // mov / mvn
        | 0x59 | 0x5D | 0x58 | 0x5C | 0x1D | 0x1C // ldr / str / ldrh / strh
        | 0x35 | 0x15 // cmp
        | 0x12 // bx lr
        | 0xA0..=0xBF // b / bl
        | 0xF0 // svc
        | 0x88 | 0x89 // stm / ldm
    )
}

fn looks_like_thumb_entry(hw: u16) -> bool {
    // PUSH {..}, MOVS, LDR, CMP, SUB sp, BX lr
    (hw & 0xFE00) == 0xB400
        || (hw & 0xFF80) == 0xB080
        || (hw & 0xE000) == 0x2000
        || (hw & 0xF800) == 0x4800
        || (hw & 0xF000) == 0x6000
        || (hw & 0xF000) == 0x7000
        || (hw & 0xF000) == 0x8000
        || (hw & 0xFF00) == 0x4600
        || hw == 0x4770
        || (hw & 0xE000) == 0x0000
}
