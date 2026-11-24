// Découpage de baserom/code.bin en unités ASM GNU ARMv6K + génération du script de liaison ldscript.ld

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::arm_disasm::{decode_arm, decode_thumb};
use crate::config::{load_splits, load_symbol_table, CppUnit, ProjectConfig, SymMode, SymbolEntry};

#[inline]
fn u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

#[inline]
fn u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}

#[derive(Clone, Debug)]
pub enum BuildSegment {
    AsmText {
        start: u32,
        end: u32,
        asm_rel_path: String,
        obj_rel_path: String,
    },
    CppText {
        unit: CppUnit,
        obj_rel_path: String,
    },
}

pub struct SplitPlan {
    pub text_segments: Vec<BuildSegment>,
    pub rodata_asm_files: Vec<(String, String)>, // (asm_rel_path, obj_rel_path)
    pub data_asm_files: Vec<(String, String)>,
}

/// Calcule le plan des segments sans réécrire les fichiers ASM (utile pour `build` incrémental).
pub fn compute_split_plan(
    config_dir: &Path,
    cfg: &ProjectConfig,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> Result<SplitPlan, String> {
    let cpp_units = load_splits(&config_dir.join("splits.toml"))?;
    let text_start = cfg.text_addr;
    let text_end = cfg.text_addr + cfg.text_size;

    let mut text_segments = Vec::new();
    let mut cursor = text_start;

    for unit in cpp_units {
        if unit.start < cursor || unit.end > text_end {
            return Err(format!(
                "Unité C++ '{}' (0x{:08X}..0x{:08X}) hors des bornes de .text (0x{:08X}..0x{:08X})",
                unit.source, unit.start, unit.end, text_start, text_end
            ));
        }
        if cursor < unit.start {
            append_asm_chunks(cursor, unit.start, symbols, &mut text_segments);
        }
        let stem = sanitize_rel_stem(&unit.source);
        let obj_rel = format!("build/obj/cpp_{}.o", stem);
        text_segments.push(BuildSegment::CppText {
            unit,
            obj_rel_path: obj_rel,
        });
        cursor = text_segments
            .last()
            .map(|s| match s {
                BuildSegment::CppText { unit, .. } => unit.end,
                BuildSegment::AsmText { end, .. } => *end,
            })
            .unwrap();
    }

    if cursor < text_end {
        append_asm_chunks(cursor, text_end, symbols, &mut text_segments);
    }

    let rodata_asm_files = vec![(
        "asm/rodata/rodata.s".to_string(),
        "build/obj/asm_rodata.o".to_string(),
    )];
    let data_asm_files = vec![(
        "asm/data/data.s".to_string(),
        "build/obj/asm_data.o".to_string(),
    )];

    Ok(SplitPlan {
        text_segments,
        rodata_asm_files,
        data_asm_files,
    })
}

fn append_asm_chunks(
    start: u32,
    end: u32,
    symbols: &BTreeMap<u32, SymbolEntry>,
    out: &mut Vec<BuildSegment>,
) {
    const CHUNK_TARGET: u32 = 0x10000; // Tranches de ~64 Ko pour parallélisme rapide
    let mut cur = start;
    while cur < end {
        let mut next = (cur + CHUNK_TARGET).min(end);
        if next < end {
            // Aligner la coupure sur un symbole ARM 4-octets proche si possible
            let search_end = (next + 0x1000).min(end);
            if let Some((&sym_addr, _)) = symbols
                .range(next..search_end)
                .find(|(&a, s)| (a & 3) == 0 && s.mode == SymMode::Arm)
            {
                next = sym_addr;
            } else {
                next &= !3;
            }
            if next <= cur {
                next = (cur + CHUNK_TARGET).min(end) & !3;
            }
            if end - next < 0x1000 {
                next = end;
            }
        }
        let asm_rel = format!("asm/text/text_{:08X}.s", cur);
        let obj_rel = format!("build/obj/asm_text_{:08X}.o", cur);
        out.push(BuildSegment::AsmText {
            start: cur,
            end: next,
            asm_rel_path: asm_rel,
            obj_rel_path: obj_rel,
        });
        cur = next;
    }
}

fn sanitize_rel_stem(rel: &str) -> String {
    rel.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub fn split_baserom(
    baserom_dir: &Path,
    config_dir: &Path,
    asm_dir: &Path,
    build_dir: &Path,
    cfg: &ProjectConfig,
) -> Result<SplitPlan, String> {
    let code_path = baserom_dir.join("code.bin");
    let code = fs::read(&code_path)
        .map_err(|e| format!("Impossible de lire '{}': {}", code_path.display(), e))?;

    let mut symbols = load_symbol_table(&config_dir.join("symbols.txt"));
    for (addr, usym) in load_symbol_table(&config_dir.join("user_symbols.txt")) {
        symbols.insert(addr, usym);
    }

    let plan = compute_split_plan(config_dir, cfg, &symbols)?;

    // Nettoyer asm/text pour éviter les anciens découpages obsolètes quand splits.toml évolue
    let asm_text_dir = asm_dir.join("text");
    if asm_text_dir.exists() {
        fs::remove_dir_all(&asm_text_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&asm_text_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(asm_dir.join("rodata")).map_err(|e| e.to_string())?;
    fs::create_dir_all(asm_dir.join("data")).map_err(|e| e.to_string())?;
    fs::create_dir_all(build_dir).map_err(|e| e.to_string())?;

    // Génération parallèle des tranches .text en assembleur GNU ARMv6K
    let asm_chunks: Vec<(u32, u32, String)> = plan
        .text_segments
        .iter()
        .filter_map(|seg| match seg {
            BuildSegment::AsmText {
                start,
                end,
                asm_rel_path,
                ..
            } => Some((*start, *end, asm_rel_path.clone())),
            _ => None,
        })
        .collect();

    let project_root = asm_dir.parent().unwrap_or_else(|| Path::new("."));
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);
    let batch_size = (asm_chunks.len() + threads - 1) / threads.max(1);

    let mut errors: Vec<String> = Vec::new();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for batch in asm_chunks.chunks(batch_size.max(1)) {
            let code_ref = &code;
            let syms_ref = &symbols;
            handles.push(s.spawn(move || -> Result<(), String> {
                for (start, end, rel_path) in batch {
                    let text = emit_text_chunk(*start, *end, code_ref, cfg, syms_ref);
                    let full_path = project_root.join(rel_path);
                    fs::write(&full_path, text).map_err(|e| e.to_string())?;
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

    // Génération de asm/rodata/rodata.s et asm/data/data.s avec tous les symboles exportés + .incbin
    let rodata_s = emit_incbin_section(
        ".rodata",
        "\"a\", %progbits",
        cfg.rodata_addr,
        cfg.rodata_size,
        cfg.text_padded,
        &symbols,
    );
    fs::write(asm_dir.join("rodata").join("rodata.s"), rodata_s).map_err(|e| e.to_string())?;

    let data_s = emit_incbin_section(
        ".data",
        "\"aw\", %progbits",
        cfg.data_addr,
        cfg.data_size,
        cfg.text_padded + cfg.rodata_padded,
        &symbols,
    );
    fs::write(asm_dir.join("data").join("data.s"), data_s).map_err(|e| e.to_string())?;

    // Génération du script de liaison GNU ld
    write_linker_script(&build_dir.join("ldscript.ld"), &plan, cfg, &symbols)?;

    let cpp_count = plan
        .text_segments
        .iter()
        .filter(|s| matches!(s, BuildSegment::CppText { .. }))
        .count();
    println!(
        "[split] {} fichiers ASM .text + {} unités C++ + .rodata/.data + build/ldscript.ld générés.",
        asm_chunks.len(),
        cpp_count
    );

    Ok(plan)
}

fn emit_text_chunk(
    start: u32,
    end: u32,
    code: &[u8],
    cfg: &ProjectConfig,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> String {
    let byte_len = (end - start) as usize;
    let mut out = String::with_capacity(byte_len * 14 + 512);
    out.push_str("@ Généré automatiquement par `blueorb split` — Pokémon Saphir Alpha (sango-2)\n");
    out.push_str(".syntax unified\n");
    out.push_str(".arch armv6k\n");
    out.push_str(".fpu vfpv2\n");
    out.push_str(".section .text, \"ax\", %progbits\n");
    out.push_str(".balign 4\n");

    // Déterminer le mode initial en cherchant le symbole précédent <= start
    let mut mode = symbols
        .range(..=start)
        .next_back()
        .map(|(_, s)| match s.mode {
            SymMode::Thumb => SymMode::Thumb,
            _ => SymMode::Arm,
        })
        .unwrap_or(SymMode::Arm);

    if mode == SymMode::Thumb {
        out.push_str(".thumb\n");
    } else {
        out.push_str(".arm\n");
    }

    let sym_lookup = |addr: u32| -> Option<String> { symbols.get(&addr).map(|s| s.name.clone()) };

    let mut pc = start;
    while pc < end {
        if let Some(sym) = symbols.get(&pc) {
            let target_mode = if sym.mode == SymMode::Thumb {
                SymMode::Thumb
            } else {
                SymMode::Arm
            };
            if target_mode != mode {
                mode = target_mode;
                if mode == SymMode::Thumb {
                    out.push_str("\n.thumb\n");
                } else {
                    out.push_str("\n.arm\n");
                }
            } else {
                out.push('\n');
            }
            if let Some(ref dem) = sym.demangled {
                let _ = writeln!(out, "@ C++: {}", dem);
            }
            if !sym.cro_users.is_empty() {
                let _ = writeln!(out, "@ CRO: {}", sym.cro_users.join(", "));
            }
            if mode == SymMode::Thumb {
                out.push_str(".thumb_func\n");
            }
            let _ = writeln!(out, ".global {}", sym.name);
            let _ = writeln!(out, ".type {}, %function", sym.name);
            let _ = writeln!(out, "{}:", sym.name);
        }

        let file_off = (pc - cfg.text_addr) as usize;

        // Si on est en mode ARM mais qu'un symbole Thumb commence à pc + 2, émettre 2 octets en Thumb d'abord !
        if mode == SymMode::Arm && pc + 2 < end && symbols.contains_key(&(pc + 2)) {
            mode = SymMode::Thumb;
            out.push_str(".thumb\n");
        }

        if mode == SymMode::Arm {
            if pc + 4 > end {
                let hw = u16_le(code, file_off);
                let _ = writeln!(out, "    .short 0x{:04X}", hw);
                pc += 2;
                continue;
            }
            let word = u32_le(code, file_off);
            let dec = decode_arm(pc, word, sym_lookup);
            let pool_note = format_pool_comment(dec.pool_load_addr, code, cfg, symbols);

            // Relocalisations symboliques exactes sur BL / BLX / B vers fonctions connues
            if (word & 0xFF00_0000) == 0xEB00_0000 {
                if let Some((target, _)) = dec.branch_target {
                    if let Some(tsym) = symbols.get(&target) {
                        if tsym.mode == SymMode::Arm {
                            let _ = writeln!(out, "    bl {:36} @ {:08X}: {:08X}", tsym.name, pc, word);
                            pc += 4;
                            continue;
                        }
                    }
                }
            } else if (word & 0xFE00_0000) == 0xFA00_0000 {
                if let Some((target, _)) = dec.branch_target {
                    if let Some(tsym) = symbols.get(&target) {
                        if tsym.mode == SymMode::Thumb {
                            let _ = writeln!(out, "    blx {:35} @ {:08X}: {:08X}", tsym.name, pc, word);
                            pc += 4;
                            continue;
                        }
                    }
                }
            } else if (word & 0xFF00_0000) == 0xEA00_0000 {
                if let Some((target, _)) = dec.branch_target {
                    if let Some(tsym) = symbols.get(&target) {
                        if tsym.mode == SymMode::Arm {
                            let _ = writeln!(out, "    b {:37} @ {:08X}: {:08X}", tsym.name, pc, word);
                            pc += 4;
                            continue;
                        }
                    }
                }
            } else if word == 0xE12F_FF1E {
                let _ = writeln!(out, "    bx lr                                   @ {:08X}: {:08X}", pc, word);
                pc += 4;
                continue;
            } else if word == 0xE1A0_0000 {
                let _ = writeln!(out, "    mov r0, r0                              @ {:08X}: {:08X} (nop)", pc, word);
                pc += 4;
                continue;
            } else if (word & 0xFFFF_0000) == 0xE92D_0000 && (word & 0xFFFF).count_ones() >= 2 {
                let _ = writeln!(out, "    {:39} @ {:08X}: {:08X}", dec.text, pc, word);
                pc += 4;
                continue;
            } else if (word & 0xFFFF_0000) == 0xE8BD_0000 && (word & 0xFFFF).count_ones() >= 2 {
                let _ = writeln!(out, "    {:39} @ {:08X}: {:08X}", dec.text, pc, word);
                pc += 4;
                continue;
            }

            let _ = writeln!(
                out,
                "    .inst 0x{:08X}                        @ {:08X}: {}{}",
                word, pc, dec.text, pool_note
            );
            pc += 4;
        } else {
            // Mode Thumb
            let hw1 = u16_le(code, file_off);
            let can_32 = pc + 4 <= end && !symbols.contains_key(&(pc + 2));
            let hw2_opt = if can_32 {
                Some(u16_le(code, file_off + 2))
            } else {
                None
            };
            let dec = decode_thumb(pc, hw1, hw2_opt, sym_lookup);
            let pool_note = format_pool_comment(dec.pool_load_addr, code, cfg, symbols);

            if dec.size == 4 {
                let hw2 = hw2_opt.unwrap();
                let is_bl = (hw2 & 0x1000) != 0;
                if let Some((target, _)) = dec.branch_target {
                    if let Some(tsym) = symbols.get(&target) {
                        if is_bl && tsym.mode == SymMode::Thumb {
                            let _ = writeln!(
                                out,
                                "    bl {:36} @ {:08X}: {:04X} {:04X}",
                                tsym.name, pc, hw1, hw2
                            );
                            pc += 4;
                            continue;
                        } else if !is_bl && tsym.mode == SymMode::Arm && (target & 3) == 0 {
                            let _ = writeln!(
                                out,
                                "    blx {:35} @ {:08X}: {:04X} {:04X}",
                                tsym.name, pc, hw1, hw2
                            );
                            pc += 4;
                            continue;
                        }
                    }
                }
                let _ = writeln!(
                    out,
                    "    .inst.n 0x{:04X}, 0x{:04X}                  @ {:08X}: {}",
                    hw1, hw2, pc, dec.text
                );
                pc += 4;
            } else {
                let _ = writeln!(
                    out,
                    "    .inst.n 0x{:04X}                          @ {:08X}: {}{}",
                    hw1, pc, dec.text, pool_note
                );
                pc += 2;
            }
        }
    }

    // Toujours terminer en mode .arm sur une frontière 4 octets pour éviter tout padding 0x46C0 implicite
    if mode == SymMode::Thumb {
        out.push_str(".arm\n");
    }
    out
}

fn format_pool_comment(
    pool_addr: Option<u32>,
    code: &[u8],
    cfg: &ProjectConfig,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> String {
    let Some(pa) = pool_addr else {
        return String::new();
    };
    if pa < cfg.text_addr || pa + 4 > cfg.text_addr + cfg.text_size {
        return format!(" [pool @ 0x{:08X}]", pa);
    }
    let off = (pa - cfg.text_addr) as usize;
    let val = u32_le(code, off);
    if let Some(sym) = symbols.get(&val).or_else(|| symbols.get(&(val & !1))) {
        format!(" [=0x{:08X} -> {}]", val, sym.name)
    } else {
        format!(" [=0x{:08X}]", val)
    }
}

fn emit_incbin_section(
    sec_name: &str,
    sec_flags: &str,
    sec_vma: u32,
    sec_size: u32,
    file_base_off: u32,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> String {
    let sec_end = sec_vma + sec_size;
    let mut out = String::with_capacity(16384);
    let _ = writeln!(out, "@ Généré automatiquement par `blueorb split`");
    let _ = writeln!(out, ".syntax unified");
    let _ = writeln!(out, ".section {}, {}", sec_name, sec_flags);
    let _ = writeln!(out, ".balign 4\n");

    let mut cursor = sec_vma;
    for (&addr, sym) in symbols.range(sec_vma..sec_end) {
        if addr > cursor {
            let chunk_len = addr - cursor;
            let bin_off = file_base_off + (cursor - sec_vma);
            let _ = writeln!(
                out,
                "    .incbin \"baserom/code.bin\", 0x{:X}, 0x{:X}",
                bin_off, chunk_len
            );
            cursor = addr;
        }
        if let Some(ref dem) = sym.demangled {
            let _ = writeln!(out, "@ C++: {}", dem);
        }
        let _ = writeln!(out, ".global {}", sym.name);
        let _ = writeln!(out, "{}:", sym.name);
    }

    if cursor < sec_end {
        let chunk_len = sec_end - cursor;
        let bin_off = file_base_off + (cursor - sec_vma);
        let _ = writeln!(
            out,
            "    .incbin \"baserom/code.bin\", 0x{:X}, 0x{:X}",
            bin_off, chunk_len
        );
    }
    out
}

pub fn write_linker_script(
    ld_path: &Path,
    plan: &SplitPlan,
    cfg: &ProjectConfig,
    symbols: &BTreeMap<u32, SymbolEntry>,
) -> Result<(), String> {
    let mut ld = String::with_capacity(32768);
    ld.push_str("/* Script de liaison généré par BlueOrb — compatible macOS, Linux et Windows */\n");
    ld.push_str("OUTPUT_FORMAT(\"elf32-littlearm\", \"elf32-littlearm\", \"elf32-littlearm\")\n");
    ld.push_str("OUTPUT_ARCH(arm)\n");
    ld.push_str("ENTRY(__ctr_start)\n\n");

    // Définition des symboles .bss globaux référencés
    let bss_end = cfg.bss_addr + cfg.bss_size;
    let _ = writeln!(ld, "__bss_start__ = 0x{:08X};", cfg.bss_addr);
    let _ = writeln!(ld, "__bss_end__   = 0x{:08X};", bss_end);
    for (&addr, sym) in symbols.range(cfg.bss_addr..=bss_end) {
        let _ = writeln!(ld, "{} = 0x{:08X};", sym.name, addr);
    }
    ld.push('\n');

    ld.push_str("SECTIONS\n{\n");

    // Section .text avec placement ordonné exact de chaque unité ASM et C++
    let _ = writeln!(ld, "    .text 0x{:08X} : SUBALIGN(4)", cfg.text_addr);
    ld.push_str("    {\n");
    for seg in &plan.text_segments {
        match seg {
            BuildSegment::AsmText {
                start,
                obj_rel_path,
                ..
            } => {
                let rel_off = start - cfg.text_addr;
                let _ = writeln!(ld, "        . = 0x{:08X};", rel_off);
                let _ = writeln!(ld, "        \"{}\"(.text)", obj_rel_path);
            }
            BuildSegment::CppText { unit, obj_rel_path } => {
                let rel_off = unit.start - cfg.text_addr;
                let expected_end = unit.end - cfg.text_addr;
                let _ = writeln!(ld, "        . = 0x{:08X}; /* {} */", rel_off, unit.source);
                let _ = writeln!(ld, "        \"{}\"(.text .text.*)", obj_rel_path);
                let _ = writeln!(
                    ld,
                    "        ASSERT(. <= 0x{:08X}, \"Débordement de taille dans {}\");",
                    expected_end, unit.source
                );
            }
        }
    }
    let _ = writeln!(ld, "        . = 0x{:08X};", cfg.text_padded - 4);
    ld.push_str("        LONG(0x00000000);\n");
    ld.push_str("    } =0x00000000\n\n");

    // Section .rodata
    let _ = writeln!(ld, "    .rodata 0x{:08X} : SUBALIGN(4)", cfg.rodata_addr);
    ld.push_str("    {\n");
    for (_, obj_rel) in &plan.rodata_asm_files {
        let _ = writeln!(ld, "        \"{}\"(.rodata .rodata.*)", obj_rel);
    }
    let _ = writeln!(ld, "        . = 0x{:08X};", cfg.rodata_padded - 4);
    ld.push_str("        LONG(0x00000000);\n");
    ld.push_str("    } =0x00000000\n\n");

    // Section .data
    let _ = writeln!(ld, "    .data 0x{:08X} : SUBALIGN(4)", cfg.data_addr);
    ld.push_str("    {\n");
    for (_, obj_rel) in &plan.data_asm_files {
        let _ = writeln!(ld, "        \"{}\"(.data .data.*)", obj_rel);
    }
    let _ = writeln!(ld, "        . = 0x{:08X};", cfg.data_padded - 4);
    ld.push_str("        LONG(0x00000000);\n");
    ld.push_str("    } =0x00000000\n\n");

    // Section .bss (NOLOAD, commence après la page .data 0x00630000 ; la zone 0x0062F9F4..0x00630000 est déjà à zéro dans .data)
    let data_page_end = cfg.data_addr + cfg.data_padded;
    let bss_tail_size = (cfg.bss_addr + cfg.bss_size).saturating_sub(data_page_end);
    let _ = writeln!(ld, "    .bss 0x{:08X} (NOLOAD) :", data_page_end);
    ld.push_str("    {\n");
    let _ = writeln!(ld, "        *(.bss .bss.* COMMON)");
    let _ = writeln!(ld, "        . = 0x{:08X};", bss_tail_size);
    ld.push_str("    }\n\n");

    // Nettoyage des métadonnées compilateur pour que objcopy produise exactement 0x530000 octets
    ld.push_str("    /DISCARD/ :\n    {\n");
    ld.push_str("        *(.ARM.exidx*)\n");
    ld.push_str("        *(.ARM.extab*)\n");
    ld.push_str("        *(.ARM.attributes*)\n");
    ld.push_str("        *(.comment*)\n");
    ld.push_str("        *(.note*)\n");
    ld.push_str("        *(.eh_frame*)\n");
    ld.push_str("    }\n}\n");

    fs::write(ld_path, ld).map_err(|e| e.to_string())
}
