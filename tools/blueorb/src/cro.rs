// Analyseur de modules relogeables Nintendo 3DS CRO0 (static.crs & Dll*.cro) + démagleur C++ Itanium

use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct CroSegment {
    pub offset: u32,
    pub size: u32,
    pub seg_type: u32, // 0=text, 1=rodata, 2=data, 3=bss
}

#[derive(Clone, Debug)]
pub struct CroNamedExport {
    pub name: String,
    pub demangled: Option<String>,
    pub seg_idx: u32,
    pub seg_offset: u32,
}

#[derive(Clone, Debug)]
pub struct CroModuleInfo {
    pub module_name: String,
    pub code_offset: u32,
    pub code_size: u32,
    pub segments: Vec<CroSegment>,
    pub named_exports: Vec<CroNamedExport>,
    pub static_anon_imports: Vec<(u32, u32)>, // (seg_idx, seg_offset) ciblant static.crs (code.bin)
}

#[inline]
fn u32_le(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(data[off..off + 4].try_into().unwrap())
}

fn read_cstr(data: &[u8], off: usize) -> String {
    if off >= data.len() {
        return String::new();
    }
    let end = data[off..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| off + p)
        .unwrap_or(data.len());
    String::from_utf8_lossy(&data[off..end]).into_owned()
}

pub fn parse_cro_file(path: &Path) -> Result<CroModuleInfo, String> {
    let data = fs::read(path)
        .map_err(|e| format!("Impossible de lire '{}': {}", path.display(), e))?;
    if data.len() < 0x130 || &data[0x80..0x84] != b"CRO0" {
        return Err(format!("Magic CRO0 invalide dans '{}'", path.display()));
    }

    let code_offset = u32_le(&data, 0xB0);
    let code_size = u32_le(&data, 0xB4);
    let mod_name_off = u32_le(&data, 0xC0) as usize;
    let seg_table_off = u32_le(&data, 0xC8) as usize;
    let seg_num = u32_le(&data, 0xCC) as usize;
    let exp_named_off = u32_le(&data, 0xD0) as usize;
    let exp_named_num = u32_le(&data, 0xD4) as usize;
    let imp_mod_off = u32_le(&data, 0xF0) as usize;
    let imp_mod_num = u32_le(&data, 0xF4) as usize;

    let raw_mod_name = read_cstr(&data, mod_name_off);
    let module_name = if raw_mod_name.is_empty() {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into())
    } else {
        raw_mod_name
    };

    let mut segments = Vec::with_capacity(seg_num);
    for i in 0..seg_num {
        let off = seg_table_off + i * 12;
        if off + 12 <= data.len() {
            segments.push(CroSegment {
                offset: u32_le(&data, off),
                size: u32_le(&data, off + 4),
                seg_type: u32_le(&data, off + 8),
            });
        }
    }

    let mut named_exports = Vec::with_capacity(exp_named_num);
    for i in 0..exp_named_num {
        let off = exp_named_off + i * 8;
        if off + 8 <= data.len() {
            let name_off = u32_le(&data, off) as usize;
            let seg_tag = u32_le(&data, off + 4);
            let name = read_cstr(&data, name_off);
            let demangled = demangle_itanium_preview(&name);
            named_exports.push(CroNamedExport {
                name,
                demangled,
                seg_idx: seg_tag & 0x0F,
                seg_offset: seg_tag >> 4,
            });
        }
    }

    let mut static_anon_imports = Vec::new();
    for m in 0..imp_mod_num {
        let m_off = imp_mod_off + m * 20;
        if m_off + 20 > data.len() {
            break;
        }
        let imp_name_off = u32_le(&data, m_off) as usize;
        let anon_off = u32_le(&data, m_off + 12) as usize;
        let anon_num = u32_le(&data, m_off + 16) as usize;
        let imp_name = read_cstr(&data, imp_name_off);
        if imp_name.contains("static") {
            for a in 0..anon_num {
                let a_off = anon_off + a * 8;
                if a_off + 8 <= data.len() {
                    let seg_tag = u32_le(&data, a_off);
                    static_anon_imports.push((seg_tag & 0x0F, seg_tag >> 4));
                }
            }
        }
    }

    Ok(CroModuleInfo {
        module_name,
        code_offset,
        code_size,
        segments,
        named_exports,
        static_anon_imports,
    })
}

/// Démangleur léger des symboles C++ Itanium (`_ZN...`, `_ZTV...`, `_ZNSs...`) pour lisibilité immédiate.
pub fn demangle_itanium_preview(sym: &str) -> Option<String> {
    if !sym.starts_with("_Z") {
        return None;
    }
    let mut rest = &sym[2..];
    let mut prefix = "";
    if let Some(r) = rest.strip_prefix("TV") {
        prefix = "vtable for ";
        rest = r;
    } else if let Some(r) = rest.strip_prefix("TI") {
        prefix = "typeinfo for ";
        rest = r;
    } else if let Some(r) = rest.strip_prefix("TS") {
        prefix = "typeinfo name for ";
        rest = r;
    } else if let Some(r) = rest.strip_prefix("GV") {
        prefix = "guard variable for ";
        rest = r;
    }

    let is_const = if let Some(r) = rest.strip_prefix("NK") {
        rest = r;
        true
    } else if let Some(r) = rest.strip_prefix('N') {
        rest = r;
        false
    } else {
        false
    };

    let mut parts: Vec<String> = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with('E') {
            break;
        }
        if let Some(r) = rest.strip_prefix("St") {
            parts.push("std".into());
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("Ss") {
            parts.push("std::string".into());
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("Sa") {
            parts.push("std::allocator".into());
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("C1") {
            parts.push("{ctor}".into());
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("C2") {
            parts.push("{base_ctor}".into());
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("D0") {
            parts.push("{deleting_dtor}".into());
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("D1") {
            parts.push("{dtor}".into());
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix("D2") {
            parts.push("{base_dtor}".into());
            rest = r;
            continue;
        }
        if rest.starts_with('I') {
            // Sauter les arguments templates imbriqués I...E
            let mut depth = 0usize;
            let mut idx = 0usize;
            for (i, ch) in rest.char_indices() {
                if ch == 'I' {
                    depth += 1;
                } else if ch == 'E' {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        idx = i + 1;
                        break;
                    }
                }
            }
            if idx > 0 {
                if let Some(last) = parts.last_mut() {
                    last.push_str("<...>");
                }
                rest = &rest[idx..];
                continue;
            } else {
                break;
            }
        }

        // Lire une longueur décimale suivie de l'identifiant
        let digits: usize = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            if let Ok(len) = rest[..digits].parse::<usize>() {
                let after = &rest[digits..];
                if len <= after.len() {
                    parts.push(after[..len].to_string());
                    rest = &after[len..];
                    continue;
                }
            }
        }
        break;
    }

    if parts.is_empty() {
        return None;
    }
    let mut joined = format!("{}{}", prefix, parts.join("::"));
    if prefix.is_empty() {
        joined.push_str("(...)");
        if is_const {
            joined.push_str(" const");
        }
    }
    Some(joined)
}
