// Extraction de l'icône officielle 3DS SMDH (Kyogre / Saphir Alpha) -> PNG, macOS .icns et Windows .ico

use std::fs;
use std::path::Path;

fn morton_index_8x8(x: usize, y: usize) -> usize {
    (x & 1)
        | ((y & 1) << 1)
        | ((x & 2) << 1)
        | ((y & 2) << 2)
        | ((x & 4) << 2)
        | ((y & 4) << 3)
}

/// Décode l'icône 48x48 RGB565 (Morton Z-order) d'un fichier SMDH (`icon.bin`) en RGBA8888 48x48.
pub fn decode_smdh_icon_48x48(smdh: &[u8]) -> Result<Vec<u8>, String> {
    const ICON_48_OFFSET: usize = 0x24C0;
    const ICON_48_BYTES: usize = 48 * 48 * 2;
    if smdh.len() < ICON_48_OFFSET + ICON_48_BYTES || &smdh[0..4] != b"SMDH" {
        return Err("Fichier SMDH icon.bin invalide ou tronqué".into());
    }

    let raw = &smdh[ICON_48_OFFSET..ICON_48_OFFSET + ICON_48_BYTES];
    let mut rgba = vec![0u8; 48 * 48 * 4];

    for y in 0..48usize {
        for x in 0..48usize {
            let tx = x / 8;
            let ty = y / 8;
            let lx = x % 8;
            let ly = y % 8;
            let tile_idx = ty * 6 + tx;
            let px_idx = tile_idx * 64 + morton_index_8x8(lx, ly);
            let byte_off = px_idx * 2;
            let px = u16::from_le_bytes([raw[byte_off], raw[byte_off + 1]]);

            let r5 = ((px >> 11) & 0x1F) as u8;
            let g6 = ((px >> 5) & 0x3F) as u8;
            let b5 = (px & 0x1F) as u8;

            let dst = (y * 48 + x) * 4;
            rgba[dst] = (r5 << 3) | (r5 >> 2);
            rgba[dst + 1] = (g6 << 2) | (g6 >> 4);
            rgba[dst + 2] = (b5 << 3) | (b5 >> 2);
            rgba[dst + 3] = 255;
        }
    }

    Ok(rgba)
}

/// Redimensionnement bilinéaire propre (recadré sur l'illustration 40x40 interne sans bordure grise 3DS) + coins squircle.
pub fn upscale_rgba_icon(src_48: &[u8], dst_size: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dst_size * dst_size * 4];
    let margin = 4.0f32;
    let span = 39.0f32; // 4.0 .. 43.0 (exclut le cadre blanc/gris du menu HOME 3DS)
    let denom = (dst_size.saturating_sub(1)).max(1) as f32;
    let radius = (dst_size as f32) * 0.22;

    for y in 0..dst_size {
        let sy = margin + ((y as f32) / denom) * span;
        let y0 = (sy.floor() as usize).min(47);
        let y1 = (y0 + 1).min(47);
        let fy = sy - (y0 as f32);

        for x in 0..dst_size {
            let sx = margin + ((x as f32) / denom) * span;
            let x0 = (sx.floor() as usize).min(47);
            let x1 = (x0 + 1).min(47);
            let fx = sx - (x0 as f32);

            let p00 = (y0 * 48 + x0) * 4;
            let p10 = (y0 * 48 + x1) * 4;
            let p01 = (y1 * 48 + x0) * 4;
            let p11 = (y1 * 48 + x1) * 4;

            let out_idx = (y * dst_size + x) * 4;
            for c in 0..3 {
                let top = (src_48[p00 + c] as f32) * (1.0 - fx) + (src_48[p10 + c] as f32) * fx;
                let bot = (src_48[p01 + c] as f32) * (1.0 - fx) + (src_48[p11 + c] as f32) * fx;
                let val = top * (1.0 - fy) + bot * fy;
                dst[out_idx + c] = val.round().clamp(0.0, 255.0) as u8;
            }

            // Masque anti-crénelé aux 4 coins arrondis
            let xf = x as f32 + 0.5;
            let yf = y as f32 + 0.5;
            let szf = dst_size as f32;
            let dx = if xf < radius {
                radius - xf
            } else if xf > szf - radius {
                xf - (szf - radius)
            } else {
                0.0
            };
            let dy = if yf < radius {
                radius - yf
            } else if yf > szf - radius {
                yf - (szf - radius)
            } else {
                0.0
            };
            let dist = (dx * dx + dy * dy).sqrt();
            let alpha = if dist <= radius - 1.0 {
                255u8
            } else if dist >= radius + 0.5 {
                0u8
            } else {
                (((radius + 0.5 - dist) / 1.5) * 255.0).round().clamp(0.0, 255.0) as u8
            };
            dst[out_idx + 3] = alpha;
        }
    }
    dst
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in bytes {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = crc32(&out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let row_bytes = (width as usize) * 4;
    let mut raw_scanlines = Vec::with_capacity((row_bytes + 1) * (height as usize));
    for y in 0..(height as usize) {
        raw_scanlines.push(0u8); // Filter type 0 (None)
        let start = y * row_bytes;
        raw_scanlines.extend_from_slice(&rgba[start..start + row_bytes]);
    }

    // Flux Zlib valide (RFC 1950) avec blocs DEFLATE non compressés (BTYPE=00)
    let mut zlib = Vec::with_capacity(raw_scanlines.len() + 64);
    zlib.extend_from_slice(&[0x78, 0x01]);
    let chunks: Vec<&[u8]> = raw_scanlines.chunks(65535).collect();
    for (i, ch) in chunks.iter().enumerate() {
        let is_last = i + 1 == chunks.len();
        zlib.push(if is_last { 0x01 } else { 0x00 });
        let len = ch.len() as u16;
        let nlen = !len;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&nlen.to_le_bytes());
        zlib.extend_from_slice(ch);
    }
    zlib.extend_from_slice(&adler32(&raw_scanlines).to_be_bytes());

    let mut png = Vec::with_capacity(zlib.len() + 128);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    write_png_chunk(&mut png, b"IHDR", &ihdr);
    write_png_chunk(&mut png, b"IDAT", &zlib);
    write_png_chunk(&mut png, b"IEND", &[]);
    png
}

pub fn generate_app_icons(
    smdh_path: &Path,
    icns_out: Option<&Path>,
    ico_out: Option<&Path>,
    png_out: Option<&Path>,
) -> Result<(), String> {
    let smdh = fs::read(smdh_path)
        .map_err(|e| format!("Impossible de lire '{}': {}", smdh_path.display(), e))?;
    let base48 = decode_smdh_icon_48x48(&smdh)?;

    let rgba256 = upscale_rgba_icon(&base48, 256);
    let png256 = encode_png_rgba(256, 256, &rgba256);

    if let Some(p) = png_out {
        fs::write(p, &png256).map_err(|e| e.to_string())?;
    }

    if let Some(p) = icns_out {
        let rgba512 = upscale_rgba_icon(&base48, 512);
        let png512 = encode_png_rgba(512, 512, &rgba512);
        let total_len = 8 + (8 + png256.len()) + (8 + png512.len());
        let mut icns = Vec::with_capacity(total_len);
        icns.extend_from_slice(b"icns");
        icns.extend_from_slice(&(total_len as u32).to_be_bytes());
        icns.extend_from_slice(b"ic08");
        icns.extend_from_slice(&((8 + png256.len()) as u32).to_be_bytes());
        icns.extend_from_slice(&png256);
        icns.extend_from_slice(b"ic09");
        icns.extend_from_slice(&((8 + png512.len()) as u32).to_be_bytes());
        icns.extend_from_slice(&png512);
        fs::write(p, icns).map_err(|e| e.to_string())?;
    }

    if let Some(p) = ico_out {
        let mut ico = Vec::with_capacity(22 + png256.len());
        ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]); // ICONDIR (1 icône)
        ico.extend_from_slice(&[0, 0, 0, 0]); // 256x256
        ico.extend_from_slice(&1u16.to_le_bytes()); // color planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(png256.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes()); // offset
        ico.extend_from_slice(&png256);
        fs::write(p, ico).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Génère une icône procédurale originale 100% légale (Orbe Saphir géométrique, zéro donnée Nintendo/Pokémon)
/// pour les distributions publiques et GitHub Releases.
fn render_procedural_orb_rgba(size: usize) -> Vec<u8> {
    let mut dst = vec![0u8; size * size * 4];
    let szf = size as f32;
    let radius = szf * 0.22;
    let center = szf * 0.5;
    let orb_r = szf * 0.34;

    for y in 0..size {
        let yf = y as f32 + 0.5;
        for x in 0..size {
            let xf = x as f32 + 0.5;
            let idx = (y * size + x) * 4;

            // Fond sombre ardoise/nuit
            let bg_t = (yf / szf).clamp(0.0, 1.0);
            let mut r = 12.0 + 8.0 * bg_t;
            let mut g = 18.0 + 14.0 * bg_t;
            let mut b = 32.0 + 26.0 * bg_t;

            // Orbe saphir central avec reflet spéculaire doux
            let ox = xf - center;
            let oy = yf - center;
            let odist = (ox * ox + oy * oy).sqrt();
            if odist <= orb_r + 1.5 {
                let edge_aa = ((orb_r + 1.0 - odist) / 1.5).clamp(0.0, 1.0);
                let nx = ox / orb_r;
                let ny = oy / orb_r;
                let nz = (1.0 - (nx * nx + ny * ny).min(1.0)).sqrt();

                // Éclairage directionnel haut-gauche
                let lx = -0.38f32;
                let ly = -0.45f32;
                let lz = 0.808f32;
                let diff = (nx * lx + ny * ly + nz * lz).max(0.0);
                let spec = diff.powf(18.0);
                let rim = (1.0 - nz).powf(2.4);

                let or_col = (18.0 + 48.0 * diff + 210.0 * spec + 45.0 * rim).clamp(0.0, 255.0);
                let og_col = (74.0 + 115.0 * diff + 225.0 * spec + 95.0 * rim).clamp(0.0, 255.0);
                let ob_col = (176.0 + 75.0 * diff + 255.0 * spec + 75.0 * rim).clamp(0.0, 255.0);

                r = r * (1.0 - edge_aa) + or_col * edge_aa;
                g = g * (1.0 - edge_aa) + og_col * edge_aa;
                b = b * (1.0 - edge_aa) + ob_col * edge_aa;
            }

            // Masque squircle macOS/Windows propre
            let dx = if xf < radius {
                radius - xf
            } else if xf > szf - radius {
                xf - (szf - radius)
            } else {
                0.0
            };
            let dy = if yf < radius {
                radius - yf
            } else if yf > szf - radius {
                yf - (szf - radius)
            } else {
                0.0
            };
            let cdist = (dx * dx + dy * dy).sqrt();
            let alpha = if cdist <= radius - 1.0 {
                255u8
            } else if cdist >= radius + 0.5 {
                0u8
            } else {
                (((radius + 0.5 - cdist) / 1.5) * 255.0).round().clamp(0.0, 255.0) as u8
            };

            dst[idx] = r.round() as u8;
            dst[idx + 1] = g.round() as u8;
            dst[idx + 2] = b.round() as u8;
            dst[idx + 3] = alpha;
        }
    }
    dst
}

pub fn generate_clean_orb_icons(
    icns_out: Option<&Path>,
    ico_out: Option<&Path>,
    png_out: Option<&Path>,
) -> Result<(), String> {
    let rgba256 = render_procedural_orb_rgba(256);
    let png256 = encode_png_rgba(256, 256, &rgba256);

    if let Some(p) = png_out {
        fs::write(p, &png256).map_err(|e| e.to_string())?;
    }

    if let Some(p) = icns_out {
        let rgba512 = render_procedural_orb_rgba(512);
        let png512 = encode_png_rgba(512, 512, &rgba512);
        let total_len = 8 + (8 + png256.len()) + (8 + png512.len());
        let mut icns = Vec::with_capacity(total_len);
        icns.extend_from_slice(b"icns");
        icns.extend_from_slice(&(total_len as u32).to_be_bytes());
        icns.extend_from_slice(b"ic08");
        icns.extend_from_slice(&((8 + png256.len()) as u32).to_be_bytes());
        icns.extend_from_slice(&png256);
        icns.extend_from_slice(b"ic09");
        icns.extend_from_slice(&((8 + png512.len()) as u32).to_be_bytes());
        icns.extend_from_slice(&png512);
        fs::write(p, icns).map_err(|e| e.to_string())?;
    }

    if let Some(p) = ico_out {
        let mut ico = Vec::with_capacity(22 + png256.len());
        ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
        ico.extend_from_slice(&[0, 0, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(png256.len() as u32).to_be_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(&png256);
        fs::write(p, ico).map_err(|e| e.to_string())?;
    }

    Ok(())
}
