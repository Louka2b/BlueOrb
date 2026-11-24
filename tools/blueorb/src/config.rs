// Lecture multi-plateforme (LF & CRLF) de blueorb.toml, splits.toml et symbols.txt

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct ProjectConfig {
    pub game: String,
    pub product_code: String,
    pub title_id: String,
    pub default_rom: String,
    pub code_sha256: String,
    pub code_file_size: u32,

    pub text_addr: u32,
    pub text_size: u32,
    pub text_padded: u32,

    pub rodata_addr: u32,
    pub rodata_size: u32,
    pub rodata_padded: u32,

    pub data_addr: u32,
    pub data_size: u32,
    pub data_padded: u32,

    pub bss_addr: u32,
    pub bss_size: u32,

    pub default_compiler: String,
    pub clang_cxx: String,
    pub gcc_cxx: String,
    pub armcc_cxx: String,
    pub assembler: String,
    pub linker: String,
    pub objcopy: String,
    pub objdump: String,
    pub clang_flags: Vec<String>,
    pub gcc_flags: Vec<String>,
    pub as_flags: Vec<String>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            game: "Pokemon Alpha Sapphire (USA) (Rev 2)".into(),
            product_code: "CTR-P-ECLA".into(),
            title_id: "000400000011C500".into(),
            default_rom: "Pokemon Alpha Sapphire (USA) (En,Ja,Fr,De,Es,It,Ko) (Rev 2).3ds".into(),
            code_sha256: "b7f9ce60361f3709ed0ce879658afe10a712f7db060640fa72bba826301b7c16".into(),
            code_file_size: 0x530000,
            text_addr: 0x00100000,
            text_size: 0x004795FC,
            text_padded: 0x0047A000,
            rodata_addr: 0x0057A000,
            rodata_size: 0x00071A14,
            rodata_padded: 0x00072000,
            data_addr: 0x005EC000,
            data_size: 0x000439F4,
            data_padded: 0x00044000,
            bss_addr: 0x0062F9F4,
            bss_size: 0x0007EC2C,
            default_compiler: "clang".into(),
            clang_cxx: "clang++".into(),
            gcc_cxx: "arm-none-eabi-g++".into(),
            armcc_cxx: "armcc".into(),
            assembler: "arm-none-eabi-as".into(),
            linker: "arm-none-eabi-ld".into(),
            objcopy: "arm-none-eabi-objcopy".into(),
            objdump: "arm-none-eabi-objdump".into(),
            clang_flags: vec![
                "-target".into(),
                "armv6k-none-eabi".into(),
                "-mfloat-abi=hard".into(),
                "-mfpu=vfpv2".into(),
                "-fno-exceptions".into(),
                "-fno-rtti".into(),
                "-fno-PIC".into(),
                "-fno-unwind-tables".into(),
                "-std=gnu++11".into(),
                "-O2".into(),
                "-Iinclude".into(),
            ],
            gcc_flags: vec![
                "-march=armv6k".into(),
                "-mfloat-abi=hard".into(),
                "-mfpu=vfpv2".into(),
                "-fno-exceptions".into(),
                "-fno-rtti".into(),
                "-std=gnu++11".into(),
                "-O2".into(),
                "-Iinclude".into(),
            ],
            as_flags: vec![
                "-march=armv6k".into(),
                "-mfpu=vfpv2".into(),
                "-mfloat-abi=hard".into(),
            ],
        }
    }
}

impl ProjectConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut cfg = Self::default();
        if !path.exists() {
            return Ok(cfg);
        }
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Impossible de lire {}: {}", path.display(), e))?;
        let pairs = parse_flat_toml(&content);

        if let Some(v) = pairs.get("project.game") {
            cfg.game = unquote(v);
        }
        if let Some(v) = pairs.get("project.product_code") {
            cfg.product_code = unquote(v);
        }
        if let Some(v) = pairs.get("project.title_id") {
            cfg.title_id = unquote(v);
        }
        if let Some(v) = pairs.get("project.default_rom") {
            cfg.default_rom = unquote(v);
        }
        if let Some(v) = pairs.get("baserom.code_sha256") {
            cfg.code_sha256 = unquote(v).to_lowercase();
        }
        if let Some(v) = pairs.get("baserom.code_file_size").and_then(|s| parse_u32(s)) {
            cfg.code_file_size = v;
        }
        if let Some(v) = pairs.get("memory_map.text_addr").and_then(|s| parse_u32(s)) {
            cfg.text_addr = v;
        }
        if let Some(v) = pairs.get("memory_map.text_size").and_then(|s| parse_u32(s)) {
            cfg.text_size = v;
        }
        if let Some(v) = pairs.get("memory_map.text_padded").and_then(|s| parse_u32(s)) {
            cfg.text_padded = v;
        }
        if let Some(v) = pairs.get("memory_map.rodata_addr").and_then(|s| parse_u32(s)) {
            cfg.rodata_addr = v;
        }
        if let Some(v) = pairs.get("memory_map.rodata_size").and_then(|s| parse_u32(s)) {
            cfg.rodata_size = v;
        }
        if let Some(v) = pairs.get("memory_map.rodata_padded").and_then(|s| parse_u32(s)) {
            cfg.rodata_padded = v;
        }
        if let Some(v) = pairs.get("memory_map.data_addr").and_then(|s| parse_u32(s)) {
            cfg.data_addr = v;
        }
        if let Some(v) = pairs.get("memory_map.data_size").and_then(|s| parse_u32(s)) {
            cfg.data_size = v;
        }
        if let Some(v) = pairs.get("memory_map.data_padded").and_then(|s| parse_u32(s)) {
            cfg.data_padded = v;
        }
        if let Some(v) = pairs.get("memory_map.bss_addr").and_then(|s| parse_u32(s)) {
            cfg.bss_addr = v;
        }
        if let Some(v) = pairs.get("memory_map.bss_size").and_then(|s| parse_u32(s)) {
            cfg.bss_size = v;
        }
        if let Some(v) = pairs.get("toolchain.default_compiler") {
            cfg.default_compiler = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.clang_cxx") {
            cfg.clang_cxx = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.gcc_cxx") {
            cfg.gcc_cxx = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.armcc_cxx") {
            cfg.armcc_cxx = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.assembler") {
            cfg.assembler = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.linker") {
            cfg.linker = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.objcopy") {
            cfg.objcopy = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.objdump") {
            cfg.objdump = unquote(v);
        }
        if let Some(v) = pairs.get("toolchain.clang_flags") {
            cfg.clang_flags = parse_string_array(v);
        }
        if let Some(v) = pairs.get("toolchain.gcc_flags") {
            cfg.gcc_flags = parse_string_array(v);
        }
        if let Some(v) = pairs.get("toolchain.as_flags") {
            cfg.as_flags = parse_string_array(v);
        }

        Ok(cfg)
    }
}

#[derive(Clone, Debug)]
pub struct CppUnit {
    pub start: u32,
    pub end: u32,
    pub source: String,
    pub compiler: Option<String>,
    pub flags: Vec<String>,
    pub gcc_flags: Vec<String>,
}

pub fn load_splits(path: &Path) -> Result<Vec<CppUnit>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Impossible de lire {}: {}", path.display(), e))?;

    let mut units = Vec::new();
    let mut cur: Option<BTreeMap<String, String>> = None;
    let mut multiline_key: Option<String> = None;
    let mut multiline_val = String::new();

    for raw_line in content.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(ref k) = multiline_key {
            multiline_val.push(' ');
            multiline_val.push_str(line);
            if line.contains(']') {
                if let Some(ref mut map) = cur {
                    map.insert(k.clone(), multiline_val.clone());
                }
                multiline_key = None;
                multiline_val.clear();
            }
            continue;
        }
        if line == "[[unit]]" {
            if let Some(map) = cur.take() {
                units.push(build_unit(map)?);
            }
            cur = Some(BTreeMap::new());
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if val.starts_with('[') && !val.contains(']') {
                multiline_key = Some(key);
                multiline_val = val;
            } else if let Some(ref mut map) = cur {
                map.insert(key, val);
            }
        }
    }
    if let Some(map) = cur.take() {
        units.push(build_unit(map)?);
    }

    units.sort_by_key(|u| u.start);
    for w in units.windows(2) {
        if w[0].end > w[1].start {
            return Err(format!(
                "Chevauchement dans splits.toml entre {} (0x{:08X}..0x{:08X}) et {} (0x{:08X}..0x{:08X})",
                w[0].source, w[0].start, w[0].end, w[1].source, w[1].start, w[1].end
            ));
        }
    }
    for u in &units {
        if (u.start % 4) != 0 || (u.end % 4) != 0 || u.end <= u.start {
            return Err(format!(
                "Plage invalide pour {} (0x{:08X}..0x{:08X}) : doit être alignée sur 4 octets et non vide",
                u.source, u.start, u.end
            ));
        }
    }

    Ok(units)
}

fn build_unit(map: BTreeMap<String, String>) -> Result<CppUnit, String> {
    let start = map
        .get("start")
        .and_then(|s| parse_u32(s))
        .ok_or_else(|| "Champ `start` manquant dans [[unit]]".to_string())?;
    let end = map
        .get("end")
        .and_then(|s| parse_u32(s))
        .ok_or_else(|| "Champ `end` manquant dans [[unit]]".to_string())?;
    let source = map
        .get("source")
        .map(|s| unquote(s))
        .ok_or_else(|| "Champ `source` manquant dans [[unit]]".to_string())?;
    let compiler = map.get("compiler").map(|s| unquote(s));
    let flags = map
        .get("flags")
        .map(|s| parse_string_array(s))
        .unwrap_or_default();
    let gcc_flags = map
        .get("gcc_flags")
        .map(|s| parse_string_array(s))
        .unwrap_or_default();

    Ok(CppUnit {
        start,
        end,
        source,
        compiler,
        flags,
        gcc_flags,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymMode {
    Arm,
    Thumb,
    Data,
}

impl SymMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymMode::Arm => "arm",
            SymMode::Thumb => "thumb",
            SymMode::Data => "data",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim() {
            "thumb" | "T" => SymMode::Thumb,
            "data" | "D" => SymMode::Data,
            _ => SymMode::Arm,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SymbolEntry {
    pub addr: u32,
    pub mode: SymMode,
    pub name: String,
    pub demangled: Option<String>,
    pub cro_users: Vec<String>,
}

pub fn load_symbol_table(path: &Path) -> BTreeMap<u32, SymbolEntry> {
    let mut out = BTreeMap::new();
    let Ok(content) = fs::read_to_string(path) else {
        return out;
    };
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Format: 0x00100000 arm __ctr_start [; cro=DllBattle,DllField | demangled=...]
        let (main_part, meta_part) = match line.split_once(';') {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (line, None),
        };
        let mut parts = main_part.split_whitespace();
        let Some(addr_s) = parts.next() else { continue };
        let Some(mode_s) = parts.next() else { continue };
        let Some(name_s) = parts.next() else { continue };
        let Some(addr) = parse_u32(addr_s) else { continue };

        let mut cro_users = Vec::new();
        let mut demangled = None;
        if let Some(meta) = meta_part {
            for seg in meta.split('|') {
                let seg = seg.trim();
                if let Some(rest) = seg.strip_prefix("cro=") {
                    cro_users = rest
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                } else if let Some(rest) = seg.strip_prefix("demangled=") {
                    demangled = Some(rest.trim().to_string());
                }
            }
        }

        out.insert(
            addr,
            SymbolEntry {
                addr,
                mode: SymMode::from_str(mode_s),
                name: name_s.to_string(),
                demangled,
                cro_users,
            },
        );
    }
    out
}

pub fn parse_u32(s: &str) -> Option<u32> {
    let clean = s.trim().replace('_', "");
    if let Some(hex) = clean.strip_prefix("0x").or_else(|| clean.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        clean.parse::<u32>().ok()
    }
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')) {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn strip_comment(s: &str) -> &str {
    let mut in_quotes = false;
    for (idx, ch) in s.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == '#' && !in_quotes {
            return &s[..idx];
        }
    }
    s
}

fn parse_string_array(s: &str) -> Vec<String> {
    let t = s.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or(t);
    inner
        .split(',')
        .map(|item| unquote(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

fn parse_flat_toml(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut section = String::new();
    let mut multi_key: Option<String> = None;
    let mut multi_val = String::new();

    for raw_line in content.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(ref k) = multi_key {
            multi_val.push(' ');
            multi_val.push_str(line);
            if line.contains(']') {
                map.insert(k.clone(), multi_val.clone());
                multi_key = None;
                multi_val.clear();
            }
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && !line.starts_with("[[") {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let full_key = if section.is_empty() {
                k.trim().to_string()
            } else {
                format!("{}.{}", section, k.trim())
            };
            let val = v.trim().to_string();
            if val.starts_with('[') && !val.contains(']') {
                multi_key = Some(full_key);
                multi_val = val;
            } else {
                map.insert(full_key, val);
            }
        }
    }
    map
}
