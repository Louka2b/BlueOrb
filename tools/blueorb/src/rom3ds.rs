// Extraction et reconstruction d'images 3DS (NCSD / NCCH / ExHeader / ExeFS / RomFS IVFC / LayeredFS)

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::blz::{compress_blz, decompress_blz};
use crate::config::ProjectConfig;
use crate::sha256::{sha256_bytes, sha256_hex};

const MEDIA_UNIT: u64 = 0x200;

#[inline]
fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

#[inline]
fn read_u64_le(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(buf[off..off + 8].try_into().unwrap())
}

#[inline]
fn align_up(val: usize, align: usize) -> usize {
    (val + align - 1) & !(align - 1)
}

#[inline]
fn align_up_u64(val: u64, align: u64) -> u64 {
    (val + align - 1) & !(align - 1)
}

pub fn extract_rom(
    rom_path: &Path,
    baserom_dir: &Path,
    cfg: &ProjectConfig,
    extract_all_romfs: bool,
) -> Result<(), String> {
    let mut f = File::open(rom_path)
        .map_err(|e| format!("Impossible d'ouvrir la ROM '{}': {}", rom_path.display(), e))?;

    let mut header = [0u8; 0x200];
    f.read_exact(&mut header)
        .map_err(|e| format!("Erreur de lecture de l'en-tête ROM: {}", e))?;

    // Détection NCSD (.3ds) ou NCCH direct (.cxi / .app)
    let ncch_offset: u64 = if &header[0x100..0x104] == b"NCSD" {
        let part0_off_mu = read_u32_le(&header, 0x120) as u64;
        part0_off_mu * MEDIA_UNIT
    } else if &header[0x100..0x104] == b"NCCH" {
        0
    } else {
        return Err("Format ROM non reconnu (magic NCSD ou NCCH introuvable à +0x100)".into());
    };

    f.seek(SeekFrom::Start(ncch_offset))
        .map_err(|e| e.to_string())?;
    let mut ncch_hdr = [0u8; 0x200];
    f.read_exact(&mut ncch_hdr).map_err(|e| e.to_string())?;
    if &ncch_hdr[0x100..0x104] != b"NCCH" {
        return Err("En-tête NCCH invalide dans la partition 0".into());
    }

    let crypto_flags = ncch_hdr[0x188 + 7];
    if (crypto_flags & 0x04) == 0 {
        return Err(format!(
            "Cette ROM 3DS est chiffrée (NCCH Flags[7]=0x{:02X}). Décryptez-la d'abord (NoCrypto=0x04 attendu).",
            crypto_flags
        ));
    }

    let product_code = String::from_utf8_lossy(&ncch_hdr[0x150..0x160])
        .trim_end_matches('\0')
        .to_string();
    let exhdr_size = read_u32_le(&ncch_hdr, 0x180) as usize;
    let exefs_off = (read_u32_le(&ncch_hdr, 0x1A0) as u64) * MEDIA_UNIT;
    let exefs_size = (read_u32_le(&ncch_hdr, 0x1A4) as usize) * (MEDIA_UNIT as usize);
    let romfs_off = (read_u32_le(&ncch_hdr, 0x1B0) as u64) * MEDIA_UNIT;
    let romfs_size = (read_u32_le(&ncch_hdr, 0x1B4) as u64) * MEDIA_UNIT;

    println!(
        "[extract] Produit: {} | NCCH @ 0x{:X} | ExeFS @ +0x{:X} (0x{:X} B) | RomFS @ +0x{:X}",
        product_code, ncch_offset, exefs_off, exefs_size, romfs_off
    );

    fs::create_dir_all(baserom_dir.join("exefs")).map_err(|e| e.to_string())?;
    fs::create_dir_all(baserom_dir.join("cro")).map_err(|e| e.to_string())?;

    fs::write(baserom_dir.join("ncch_header.bin"), &ncch_hdr).map_err(|e| e.to_string())?;

    // Lecture ExHeader (0x800 octets complets : SCI + ACI + AccessDesc)
    let exhdr_total = exhdr_size * 2;
    let mut exhdr = vec![0u8; exhdr_total];
    f.seek(SeekFrom::Start(ncch_offset + 0x200))
        .map_err(|e| e.to_string())?;
    f.read_exact(&mut exhdr).map_err(|e| e.to_string())?;
    fs::write(baserom_dir.join("exheader.bin"), &exhdr).map_err(|e| e.to_string())?;

    let sci_flags = exhdr[0x0D];
    let is_code_compressed = (sci_flags & 1) != 0;
    let text_addr = read_u32_le(&exhdr, 0x10);
    let text_size = read_u32_le(&exhdr, 0x18);
    let ro_addr = read_u32_le(&exhdr, 0x20);
    let ro_size = read_u32_le(&exhdr, 0x28);
    let data_addr = read_u32_le(&exhdr, 0x30);
    let data_size = read_u32_le(&exhdr, 0x38);
    let bss_size = read_u32_le(&exhdr, 0x3C);

    println!(
        "[extract] SCI: compressed={} | .text=0x{:08X}(0x{:X}) | .rodata=0x{:08X}(0x{:X}) | .data=0x{:08X}(0x{:X}) | .bss=0x{:X}",
        is_code_compressed, text_addr, text_size, ro_addr, ro_size, data_addr, data_size, bss_size
    );

    // Lecture ExeFS
    f.seek(SeekFrom::Start(ncch_offset + exefs_off))
        .map_err(|e| e.to_string())?;
    let mut exefs = vec![0u8; exefs_size];
    f.read_exact(&mut exefs).map_err(|e| e.to_string())?;

    let mut found_code = false;
    for i in 0..10 {
        let entry_off = i * 16;
        let raw_name = &exefs[entry_off..entry_off + 8];
        let name = String::from_utf8_lossy(raw_name)
            .trim_end_matches('\0')
            .to_string();
        let f_off = read_u32_le(&exefs, entry_off + 8) as usize;
        let f_sz = read_u32_le(&exefs, entry_off + 12) as usize;
        if name.is_empty() && f_sz == 0 {
            continue;
        }
        let data_start = 0x200 + f_off;
        let data_end = data_start + f_sz;
        if data_end > exefs.len() {
            return Err(format!("Entrée ExeFS '{}' hors limites", name));
        }
        let slice = &exefs[data_start..data_end];
        if name == ".code" {
            found_code = true;
            fs::write(baserom_dir.join("exefs").join("code_raw.bin"), slice)
                .map_err(|e| e.to_string())?;
            let decomp = if is_code_compressed {
                decompress_blz(slice)?
            } else {
                slice.to_vec()
            };
            let code_hash = sha256_hex(&decomp);
            fs::write(baserom_dir.join("code.bin"), &decomp).map_err(|e| e.to_string())?;
            println!(
                "[extract] code.bin décompressé: 0x{:X} octets (SHA-256: {})",
                decomp.len(),
                code_hash
            );
            if !cfg.code_sha256.is_empty() && code_hash != cfg.code_sha256 {
                println!(
                    "[attention] Le SHA-256 de code.bin ({}) diffère de blueorb.toml ({}) — révision ROM différente ?",
                    code_hash, cfg.code_sha256
                );
            }
        } else {
            let safe_name = format!("{}.bin", name.trim_start_matches('.'));
            fs::write(baserom_dir.join("exefs").join(safe_name), slice)
                .map_err(|e| e.to_string())?;
        }
    }

    if !found_code {
        return Err("Section .code introuvable dans ExeFS".into());
    }

    // Extraction des modules CRO / static.crs depuis RomFS IVFC Level 3
    if romfs_size > 0 {
        extract_romfs_modules(
            &mut f,
            ncch_offset + romfs_off,
            baserom_dir,
            extract_all_romfs,
        )?;
    }

    Ok(())
}

fn extract_romfs_modules(
    f: &mut File,
    romfs_base: u64,
    baserom_dir: &Path,
    extract_all: bool,
) -> Result<(), String> {
    f.seek(SeekFrom::Start(romfs_base))
        .map_err(|e| e.to_string())?;
    let mut ivfc = [0u8; 0x60];
    f.read_exact(&mut ivfc).map_err(|e| e.to_string())?;
    if &ivfc[0..4] != b"IVFC" {
        return Err("En-tête RomFS IVFC invalide".into());
    }

    let master_hash_sz = read_u32_le(&ivfc, 0x08) as u64;
    let l3_block_log2 = read_u32_le(&ivfc, 0x4C);
    let l3_align = 1u64 << l3_block_log2;
    let l3_base = romfs_base + align_up_u64(0x60 + master_hash_sz, l3_align);

    f.seek(SeekFrom::Start(l3_base)).map_err(|e| e.to_string())?;
    let mut l3_hdr = [0u8; 40];
    f.read_exact(&mut l3_hdr).map_err(|e| e.to_string())?;

    let dir_meta_off = read_u32_le(&l3_hdr, 12) as u64;
    let dir_meta_len = read_u32_le(&l3_hdr, 16) as usize;
    let file_meta_off = read_u32_le(&l3_hdr, 28) as u64;
    let file_meta_len = read_u32_le(&l3_hdr, 32) as usize;
    let data_off = read_u32_le(&l3_hdr, 36) as u64;

    let mut dir_meta = vec![0u8; dir_meta_len];
    f.seek(SeekFrom::Start(l3_base + dir_meta_off))
        .map_err(|e| e.to_string())?;
    f.read_exact(&mut dir_meta).map_err(|e| e.to_string())?;

    let mut file_meta = vec![0u8; file_meta_len];
    f.seek(SeekFrom::Start(l3_base + file_meta_off))
        .map_err(|e| e.to_string())?;
    f.read_exact(&mut file_meta).map_err(|e| e.to_string())?;

    let mut cro_count = 0usize;
    let mut other_count = 0usize;

    struct StackItem {
        dir_off: usize,
        rel_path: String,
    }
    let mut stack = vec![StackItem {
        dir_off: 0,
        rel_path: String::new(),
    }];

    while let Some(item) = stack.pop() {
        let d = item.dir_off;
        if d + 24 > dir_meta.len() {
            continue;
        }
        let child_dir = read_u32_le(&dir_meta, d + 8);
        let child_file = read_u32_le(&dir_meta, d + 12);
        let name_len = read_u32_le(&dir_meta, d + 20) as usize;
        let dir_name = decode_utf16le(&dir_meta[d + 24..d + 24 + name_len]);
        let cur_rel = if dir_name.is_empty() {
            item.rel_path.clone()
        } else if item.rel_path.is_empty() {
            dir_name
        } else {
            format!("{}/{}", item.rel_path, dir_name)
        };

        // Fichiers de ce dossier
        let mut f_off = child_file;
        while f_off != 0xFFFF_FFFF && (f_off as usize) + 32 <= file_meta.len() {
            let fo = f_off as usize;
            let f_sib = read_u32_le(&file_meta, fo + 4);
            let f_data_off = read_u64_le(&file_meta, fo + 8);
            let f_data_len = read_u64_le(&file_meta, fo + 16);
            let f_name_len = read_u32_le(&file_meta, fo + 28) as usize;
            let f_name = decode_utf16le(&file_meta[fo + 32..fo + 32 + f_name_len]);

            let lower = f_name.to_lowercase();
            let is_cro_related =
                lower.ends_with(".cro") || lower.ends_with(".crs") || lower.ends_with(".crr");

            if is_cro_related || extract_all {
                let target_path = if lower.ends_with(".cro") && !extract_all {
                    baserom_dir.join("cro").join(&f_name)
                } else if (lower.ends_with(".crs") || lower.ends_with(".crr")) && !extract_all {
                    baserom_dir.join(&f_name)
                } else {
                    let mut p = baserom_dir.join("romfs");
                    if !cur_rel.is_empty() {
                        for comp in cur_rel.split('/') {
                            p.push(comp);
                        }
                    }
                    fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                    p.join(&f_name)
                };

                f.seek(SeekFrom::Start(l3_base + data_off + f_data_off))
                    .map_err(|e| e.to_string())?;
                stream_to_file(f, &target_path, f_data_len)?;
                if lower.ends_with(".cro") {
                    cro_count += 1;
                } else {
                    other_count += 1;
                }
            }

            f_off = f_sib;
        }

        // Sous-dossiers
        let mut c_off = child_dir;
        while c_off != 0xFFFF_FFFF && (c_off as usize) + 24 <= dir_meta.len() {
            let next_sib = read_u32_le(&dir_meta, (c_off as usize) + 4);
            stack.push(StackItem {
                dir_off: c_off as usize,
                rel_path: cur_rel.clone(),
            });
            c_off = next_sib;
        }
    }

    println!(
        "[extract] RomFS: {} modules Dll*.cro extraits + {} fichiers systèmes (static.crs/static.crr)",
        cro_count, other_count
    );
    Ok(())
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let mut u16s = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        u16s.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    String::from_utf16_lossy(&u16s)
}

fn stream_to_file(src: &mut File, dst_path: &Path, mut remaining: u64) -> Result<(), String> {
    let mut out = File::create(dst_path)
        .map_err(|e| format!("Impossible de créer '{}': {}", dst_path.display(), e))?;
    let mut buf = vec![0u8; 256 * 1024];
    while remaining > 0 {
        let to_read = (buf.len() as u64).min(remaining) as usize;
        src.read_exact(&mut buf[..to_read])
            .map_err(|e| e.to_string())?;
        out.write_all(&buf[..to_read]).map_err(|e| e.to_string())?;
        remaining -= to_read as u64;
    }
    Ok(())
}

/// Construit le dossier LayeredFS (Luma3DS / Azahar / Lime3DS / Citra) et reconstruit une ROM .3ds complète.
pub fn pack_rom(
    code_bin_path: &Path,
    baserom_dir: &Path,
    orig_rom_path: &Path,
    out_rom_path: Option<&Path>,
    build_dir: &Path,
    cfg: &ProjectConfig,
    compress_code: bool,
) -> Result<(), String> {
    let code_bin = fs::read(code_bin_path)
        .map_err(|e| format!("Impossible de lire '{}': {}", code_bin_path.display(), e))?;

    // 1. Export LayeredFS (code.bin non compressé pour Luma3DS / Azahar / Citra)
    let luma_dir = build_dir
        .join("luma")
        .join("titles")
        .join(&cfg.title_id);
    fs::create_dir_all(&luma_dir).map_err(|e| e.to_string())?;
    let luma_code = luma_dir.join("code.bin");
    fs::write(&luma_code, &code_bin).map_err(|e| e.to_string())?;
    println!(
        "[pack] Mod LayeredFS prêt (Azahar/Citra/Luma3DS) : {}",
        luma_code.display()
    );

    let Some(target_rom) = out_rom_path else {
        return Ok(());
    };

    if !orig_rom_path.exists() {
        return Err(format!(
            "ROM source '{}' introuvable pour reconstruire le fichier .3ds",
            orig_rom_path.display()
        ));
    }

    // 2. Préparation du .code pour ExeFS
    let baserom_code_path = baserom_dir.join("code.bin");
    let orig_raw_code_path = baserom_dir.join("exefs").join("code_raw.bin");
    let banner = fs::read(baserom_dir.join("exefs").join("banner.bin"))
        .map_err(|e| format!("banner.bin manquant dans baserom/exefs (lancez `extract`): {}", e))?;
    let icon = fs::read(baserom_dir.join("exefs").join("icon.bin"))
        .map_err(|e| format!("icon.bin manquant dans baserom/exefs (lancez `extract`): {}", e))?;
    let mut exhdr = fs::read(baserom_dir.join("exheader.bin"))
        .map_err(|e| format!("exheader.bin manquant dans baserom (lancez `extract`): {}", e))?;

    let (exefs_code_bytes, is_compressed) = if compress_code {
        // Si code.bin est strictement identique au baserom, réutiliser le flux BLZ original Nintendo est instantané !
        let same_as_base = baserom_code_path.exists()
            && fs::read(&baserom_code_path)
                .map(|b| b == code_bin)
                .unwrap_or(false);
        if same_as_base && orig_raw_code_path.exists() {
            println!("[pack] code.bin identique à l'original : réutilisation du bloc BLZ exact.");
            (fs::read(&orig_raw_code_path).map_err(|e| e.to_string())?, true)
        } else {
            println!("[pack] Compression BLZ de code.bin (0x{:X} octets)...", code_bin.len());
            let comp = compress_blz(&code_bin);
            let did_compress = comp.len() < code_bin.len();
            println!(
                "[pack] Taille .code compressé : 0x{:X} octets ({:.1}%)",
                comp.len(),
                (comp.len() as f64) * 100.0 / (code_bin.len() as f64)
            );
            (comp, did_compress)
        }
    } else {
        (code_bin.clone(), false)
    };

    // Mise à jour du flag SCI (bit 0 = compressé) dans ExHeader + hash SHA-256 dans ACI si présent
    if is_compressed {
        exhdr[0x0D] |= 0x01;
    } else {
        exhdr[0x0D] &= !0x01;
    }

    // Construction de l'image ExeFS (en-tête 0x200 + données alignées sur 0x200)
    let files: [(&[u8; 8], &[u8]); 3] = [
        (b".code\0\0\0", &exefs_code_bytes),
        (b"banner\0\0", &banner),
        (b"icon\0\0\0\0", &icon),
    ];

    let mut exefs_hdr = [0u8; 0x200];
    let mut data_cursor = 0usize;
    for (idx, (name8, blob)) in files.iter().enumerate() {
        let entry_off = idx * 16;
        exefs_hdr[entry_off..entry_off + 8].copy_from_slice(*name8);
        exefs_hdr[entry_off + 8..entry_off + 12].copy_from_slice(&(data_cursor as u32).to_le_bytes());
        exefs_hdr[entry_off + 12..entry_off + 16].copy_from_slice(&(blob.len() as u32).to_le_bytes());

        let file_hash = sha256_bytes(blob);
        let hash_off = 0x200 - (idx + 1) * 0x20;
        exefs_hdr[hash_off..hash_off + 0x20].copy_from_slice(&file_hash);

        data_cursor = align_up(data_cursor + blob.len(), 0x200);
    }

    let mut new_exefs = vec![0u8; 0x200 + data_cursor];
    new_exefs[..0x200].copy_from_slice(&exefs_hdr);
    for (idx, (_, blob)) in files.iter().enumerate() {
        let f_off = read_u32_le(&exefs_hdr, idx * 16 + 8) as usize;
        new_exefs[0x200 + f_off..0x200 + f_off + blob.len()].copy_from_slice(blob);
    }

    // Reconstruction streaming du .3ds
    let mut src_rom = File::open(orig_rom_path).map_err(|e| e.to_string())?;
    let orig_len = src_rom.metadata().map_err(|e| e.to_string())?.len();

    let mut ncsd = [0u8; 0x4000];
    src_rom.read_exact(&mut ncsd).map_err(|e| e.to_string())?;
    if &ncsd[0x100..0x104] != b"NCSD" {
        return Err("La ROM source doit être au format NCSD (.3ds) pour reconstruire un .3ds".into());
    }

    let part0_off = (read_u32_le(&ncsd, 0x120) as u64) * MEDIA_UNIT;
    let part0_orig_sz = (read_u32_le(&ncsd, 0x124) as u64) * MEDIA_UNIT;

    src_rom
        .seek(SeekFrom::Start(part0_off))
        .map_err(|e| e.to_string())?;
    let mut ncch = [0u8; 0x200];
    src_rom.read_exact(&mut ncch).map_err(|e| e.to_string())?;

    let exefs_rel_off = (read_u32_le(&ncch, 0x1A0) as u64) * MEDIA_UNIT;
    let exefs_hash_reg_mu = read_u32_le(&ncch, 0x1A8) as usize;
    let orig_romfs_rel_off = (read_u32_le(&ncch, 0x1B0) as u64) * MEDIA_UNIT;

    let new_exefs_mu = (new_exefs.len() as u64) / MEDIA_UNIT;
    let min_romfs_rel_off = align_up_u64(exefs_rel_off + (new_exefs.len() as u64), 0x1000);
    let new_romfs_rel_off = orig_romfs_rel_off.max(min_romfs_rel_off);
    let delta_bytes = new_romfs_rel_off - orig_romfs_rel_off;
    let delta_mu = (delta_bytes / MEDIA_UNIT) as u32;

    // Mise à jour des hashes et tailles NCCH
    let exhdr_hash_len = read_u32_le(&ncch, 0x180) as usize;
    let exhdr_sha = sha256_bytes(&exhdr[..exhdr_hash_len]);
    ncch[0x160..0x180].copy_from_slice(&exhdr_sha);

    let superblock_len = (exefs_hash_reg_mu * 0x200).min(new_exefs.len());
    let exefs_sb_sha = sha256_bytes(&new_exefs[..superblock_len]);
    ncch[0x180..0x1A0].copy_from_slice(&exefs_sb_sha);

    ncch[0x1A4..0x1A8].copy_from_slice(&(new_exefs_mu as u32).to_le_bytes());
    let new_romfs_mu = (new_romfs_rel_off / MEDIA_UNIT) as u32;
    ncch[0x1B0..0x1B4].copy_from_slice(&new_romfs_mu.to_le_bytes());

    let orig_ncch_mu = read_u32_le(&ncch, 0x104);
    let new_ncch_mu = orig_ncch_mu + delta_mu;
    ncch[0x104..0x108].copy_from_slice(&new_ncch_mu.to_le_bytes());

    // Mise à jour de la table des partitions NCSD si RomFS a été décalé
    if delta_mu > 0 {
        ncsd[0x124..0x128].copy_from_slice(&new_ncch_mu.to_le_bytes());
        for p in 1..8 {
            let p_off_pos = 0x120 + p * 8;
            let p_sz_pos = p_off_pos + 4;
            let p_off = read_u32_le(&ncsd, p_off_pos);
            let p_sz = read_u32_le(&ncsd, p_sz_pos);
            if p_sz > 0 && (p_off as u64) * MEDIA_UNIT >= part0_off + part0_orig_sz {
                let shifted = p_off + delta_mu;
                ncsd[p_off_pos..p_off_pos + 4].copy_from_slice(&shifted.to_le_bytes());
            }
        }
    }

    if let Some(parent) = target_rom.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    println!(
        "[pack] Écriture de la ROM reconstruite vers '{}'...",
        target_rom.display()
    );

    let mut out = File::create(target_rom).map_err(|e| e.to_string())?;
    // Préfixe NCSD avant part0
    out.write_all(&ncsd[..(part0_off as usize).min(ncsd.len())])
        .map_err(|e| e.to_string())?;
    if part0_off > ncsd.len() as u64 {
        src_rom
            .seek(SeekFrom::Start(ncsd.len() as u64))
            .map_err(|e| e.to_string())?;
        copy_range(&mut src_rom, &mut out, part0_off - (ncsd.len() as u64))?;
    }

    // NCCH Header + ExHeader + PlainRegion/Logo jusqu'à ExeFS
    out.write_all(&ncch).map_err(|e| e.to_string())?;
    out.write_all(&exhdr).map_err(|e| e.to_string())?;

    let after_exhdr = 0x200 + (exhdr.len() as u64);
    if exefs_rel_off > after_exhdr {
        src_rom
            .seek(SeekFrom::Start(part0_off + after_exhdr))
            .map_err(|e| e.to_string())?;
        copy_range(&mut src_rom, &mut out, exefs_rel_off - after_exhdr)?;
    }

    // Nouvel ExeFS + padding zéro jusqu'au début de RomFS
    out.write_all(&new_exefs).map_err(|e| e.to_string())?;
    let pad_before_romfs = new_romfs_rel_off - (exefs_rel_off + new_exefs.len() as u64);
    if pad_before_romfs > 0 {
        let zeros = vec![0u8; pad_before_romfs as usize];
        out.write_all(&zeros).map_err(|e| e.to_string())?;
    }

    // Copie streaming de RomFS + partitions restantes (manuels, mises à jour, padding cartouche)
    let tail_src_start = part0_off + orig_romfs_rel_off;
    let tail_len = orig_len.saturating_sub(tail_src_start);
    src_rom
        .seek(SeekFrom::Start(tail_src_start))
        .map_err(|e| e.to_string())?;
    copy_range(&mut src_rom, &mut out, tail_len)?;
    out.flush().map_err(|e| e.to_string())?;

    println!(
        "[pack] ROM .3ds reconstruite avec succès : {} ({:.2} Go)",
        target_rom.display(),
        (out.metadata().map(|m| m.len()).unwrap_or(0) as f64) / (1024.0 * 1024.0 * 1024.0)
    );
    Ok(())
}

fn copy_range(src: &mut File, dst: &mut File, mut remaining: u64) -> Result<(), String> {
    let mut buf = vec![0u8; 8 * 1024 * 1024]; // Buffer 8 Mo multi-plateforme
    while remaining > 0 {
        let chunk = (buf.len() as u64).min(remaining) as usize;
        src.read_exact(&mut buf[..chunk]).map_err(|e| e.to_string())?;
        dst.write_all(&buf[..chunk]).map_err(|e| e.to_string())?;
        remaining -= chunk as u64;
    }
    Ok(())
}
