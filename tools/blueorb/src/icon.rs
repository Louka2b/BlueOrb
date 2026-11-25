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

/// Shades a single subpixel sample `(xf, yf)` in `[0, size]` for the Sapphire Dive-Ball launcher icon.
/// 100% original procedural 3D ray-sphere artwork (zero ripped ROM bytes).
fn shade_sapphire_ball_sample(xf: f32, yf: f32, szf: f32) -> [f32; 4] {
    let corner_r = szf * 0.225;

    // 1. macOS / Windows squircle badge mask
    let dx = if xf < corner_r {
        corner_r - xf
    } else if xf > szf - corner_r {
        xf - (szf - corner_r)
    } else {
        0.0
    };
    let dy = if yf < corner_r {
        corner_r - yf
    } else if yf > szf - corner_r {
        yf - (szf - corner_r)
    } else {
        0.0
    };
    let cdist = (dx * dx + dy * dy).sqrt();
    if cdist > corner_r + 0.6 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let badge_alpha = (corner_r + 0.5 - cdist).clamp(0.0, 1.0);

    // Normalized coordinates in [-1, 1]
    let ux = (xf / szf) * 2.0 - 1.0;
    let uy = (yf / szf) * 2.0 - 1.0;

    // 2. Deep ocean-slate background badge with subtle radial glow + drop shadow
    let radial = (ux * ux + (uy + 0.06) * (uy + 0.06)).sqrt();
    let glow = (1.0 - radial * 0.85).clamp(0.0, 1.0).powi(2);
    let mut r = 13.0 + 14.0 * glow;
    let mut g = 20.0 + 34.0 * glow;
    let mut b = 34.0 + 68.0 * glow;

    // Soft contact shadow beneath the ball
    let sx = ux / 0.68;
    let sy = (uy - 0.74) / 0.16;
    let shadow_d = (sx * sx + sy * sy).sqrt();
    if shadow_d < 1.0 {
        let sh = (1.0 - shadow_d).powi(2) * 0.58;
        r *= 1.0 - sh;
        g *= 1.0 - sh;
        b *= 1.0 - sh;
    }

    // Subtle 1px inner border highlight on the badge
    let border_band = (corner_r - cdist).abs();
    let edge_dist = xf.min(szf - xf).min(yf.min(szf - yf));
    let rim_d = if dx > 0.0 && dy > 0.0 { border_band } else { edge_dist };
    if rim_d < szf * 0.014 {
        let t = (1.0 - rim_d / (szf * 0.014)).clamp(0.0, 1.0) * 0.22;
        r = r * (1.0 - t) + 120.0 * t;
        g = g * (1.0 - t) + 165.0 * t;
        b = b * (1.0 - t) + 235.0 * t;
    }

    // 3. 3D Monster-Ball / Sapphire Dive-Ball ray-sphere intersection
    let ball_r = 0.77f32;
    let bx = ux;
    let by = uy + 0.015; // slight optical lift
    let bdist2 = bx * bx + by * by;
    let bdist = bdist2.sqrt();

    if bdist <= ball_r {
        let nx = bx / ball_r;
        let ny_screen = -by / ball_r; // +Y up in 3D world space
        let nz = (1.0 - (nx * nx + ny_screen * ny_screen).min(1.0)).sqrt();

        // Tilt sphere slightly forward (-11 deg pitch) so the seam curves naturally in 3D
        let pitch: f32 = -0.19;
        let (sp, cp) = pitch.sin_cos();
        let px = nx;
        let py = ny_screen * cp - nz * sp;
        let pz = ny_screen * sp + nz * cp;

        // Button front axis centered slightly right/up or straight ahead along tilted equator (0, 0, 1)
        // Angular distance to the front clasp center `(0, 0, 1)` in tilted ball space:
        let clasp_dot = pz.clamp(-1.0, 1.0);
        let clasp_ang = clasp_dot.acos(); // radians from front button center

        // Lighting vectors (key light top-left, cool rim light bottom-right)
        let lx = -0.40f32;
        let ly = 0.56f32;
        let lz = 0.724f32;
        let ndotl = (nx * lx + ny_screen * ly + nz * lz).max(0.0);

        // Half-vector for crisp glossy clear-coat reflection
        let hx = lx;
        let hy = ly;
        let hz = lz + 1.0;
        let hlen = (hx * hx + hy * hy + hz * hz).sqrt();
        let ndoth = ((nx * hx + ny_screen * hy + nz * hz) / hlen).max(0.0);
        let spec_sharp = ndoth.powf(56.0);
        let spec_broad = ndoth.powf(12.0);
        let fresnel = (1.0 - nz).powf(3.0);

        // Material regions on the tilted sphere:
        // - Clasp outer dark bezel: `clasp_ang < 0.285`
        // - Clasp white metallic ring: `clasp_ang < 0.215`
        // - Clasp inner groove: `clasp_ang < 0.128`
        // - Clasp glowing sapphire core: `clasp_ang < 0.105`
        // - Equatorial dark seam band: `|py| < 0.068`
        let (mut mr, mut mg, mut mb, gloss) = if clasp_ang < 0.105 {
            // Glowing faceted Sapphire core inside the button
            let core_t = (1.0 - clasp_ang / 0.105).clamp(0.0, 1.0);
            let cr = 24.0 + 135.0 * core_t * core_t;
            let cg = 148.0 + 95.0 * core_t;
            let cb = 245.0 + 10.0 * core_t;
            (cr, cg, cb, 0.9)
        } else if clasp_ang < 0.130 {
            // Thin recessed inner ring groove
            (22.0, 27.0, 38.0, 0.25)
        } else if clasp_ang < 0.215 {
            // Classic white/silver Monster-Ball button ring
            let shade = 0.62 + 0.38 * ndotl;
            (236.0 * shade, 242.0 * shade, 250.0 * shade, 0.55)
        } else if clasp_ang < 0.282 || py.abs() < 0.066 {
            // Recessed dark obsidian seam band & outer button housing
            let shade = 0.55 + 0.45 * ndotl;
            (24.0 * shade + 8.0, 28.0 * shade + 10.0, 38.0 * shade + 14.0, 0.35)
        } else if py > 0.0 {
            // UPPER HEMISPHERE: Iconic Dive-Ball / Sapphire two-tone ocean blue + crisp wave crest!
            let wave_main = 0.23 + 0.085 * (px * 5.2).cos() - 0.04 * px * px;
            let wave_dist = (py - wave_main).abs();
            let in_crest = wave_dist < 0.044 && pz > -0.2;

            // Upper sky-cyan cap above a second subtle wave (`py > wave_upper`), just like a Dive Ball!
            let wave_upper = 0.56 + 0.055 * (px * 4.2).cos();
            let cap_blend = ((py - wave_upper) / 0.032).clamp(0.0, 1.0);

            let shade = 0.34 + 0.66 * ndotl;
            // Base deep royal Alpha Sapphire
            let mut sr = 10.0 + 28.0 * shade + 20.0 * spec_broad;
            let mut sg = 58.0 + 102.0 * shade + 46.0 * spec_broad;
            let mut sb = 166.0 + 86.0 * shade + 28.0 * spec_broad;

            // Blend lighter aqua-sky upper cap
            if cap_blend > 0.0 {
                let cr = 46.0 + 78.0 * shade;
                let cg = 148.0 + 82.0 * shade;
                let cb = 238.0 + 17.0 * shade;
                sr = sr * (1.0 - cap_blend) + cr * cap_blend;
                sg = sg * (1.0 - cap_blend) + cg * cap_blend;
                sb = sb * (1.0 - cap_blend) + cb * cap_blend;
            }

            // Crisp ice-white / pale-cyan wave ribbon
            if in_crest {
                let blend = ((0.044 - wave_dist) / 0.015).clamp(0.0, 1.0);
                let wr = 185.0 + 65.0 * shade;
                let wg = 232.0 + 22.0 * shade;
                let wb = 255.0;
                sr = sr * (1.0 - blend) + wr * blend;
                sg = sg * (1.0 - blend) + wg * blend;
                sb = sb * (1.0 - blend) + wb * blend;
            }

            // Seam lip highlight right above the dark band
            if py < 0.084 {
                sr += 26.0 * ndotl;
                sg += 40.0 * ndotl;
                sb += 48.0 * ndotl;
            }

            (sr, sg, sb, 1.0)
        } else {
            // LOWER HEMISPHERE: Classic Pearl-White Monster-Ball bottom shell
            let shade = 0.58 + 0.42 * ndotl;
            // Cool ocean bounce light near bottom rim
            let bounce = (-ny_screen).clamp(0.0, 1.0).powi(3) * 0.25;
            let lr = 240.0 * shade - 36.0 * bounce;
            let lg = 245.0 * shade - 8.0 * bounce;
            let lb = 252.0 * shade + 12.0 * bounce;
            (lr, lg, lb, 0.65)
        };

        // Clear-coat specular highlight + rim sheen
        mr += 235.0 * spec_sharp * gloss + 32.0 * fresnel;
        mg += 242.0 * spec_sharp * gloss + 78.0 * fresnel;
        mb += 255.0 * spec_sharp * gloss + 135.0 * fresnel;

        // Ambient occlusion near outer sphere silhouette
        let edge_ao = (0.25 + 0.75 * nz.sqrt()).clamp(0.0, 1.0);
        r = (mr * edge_ao).clamp(0.0, 255.0);
        g = (mg * edge_ao).clamp(0.0, 255.0);
        b = (mb * edge_ao).clamp(0.0, 255.0);
    }

    [r.clamp(0.0, 255.0), g.clamp(0.0, 255.0), b.clamp(0.0, 255.0), badge_alpha * 255.0]
}

/// Renders the Sapphire Dive-Ball launcher icon at `size x size` RGBA with 4x4 supersampling (16x MSAA).
pub fn render_procedural_orb_rgba(size: usize) -> Vec<u8> {
    let mut dst = vec![0u8; size * size * 4];
    let szf = size as f32;
    let grid = if size <= 128 { 4 } else { 3 };
    let inv_samples = 1.0 / ((grid * grid) as f32);

    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..grid {
                let yf = y as f32 + (sy as f32 + 0.5) / (grid as f32);
                for sx in 0..grid {
                    let xf = x as f32 + (sx as f32 + 0.5) / (grid as f32);
                    let s = shade_sapphire_ball_sample(xf, yf, szf);
                    let w = s[3] / 255.0;
                    acc[0] += s[0] * w;
                    acc[1] += s[1] * w;
                    acc[2] += s[2] * w;
                    acc[3] += s[3];
                }
            }
            let a_avg = acc[3] * inv_samples;
            let idx = (y * size + x) * 4;
            if a_avg > 0.5 {
                let w_sum = (acc[3] / 255.0).max(1e-5);
                dst[idx] = (acc[0] / w_sum).round().clamp(0.0, 255.0) as u8;
                dst[idx + 1] = (acc[1] / w_sum).round().clamp(0.0, 255.0) as u8;
                dst[idx + 2] = (acc[2] / w_sum).round().clamp(0.0, 255.0) as u8;
                dst[idx + 3] = a_avg.round().clamp(0.0, 255.0) as u8;
            }
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
        let rgba128 = render_procedural_orb_rgba(128);
        let rgba512 = render_procedural_orb_rgba(512);
        let png128 = encode_png_rgba(128, 128, &rgba128);
        let png512 = encode_png_rgba(512, 512, &rgba512);
        let entries: [(&[u8; 4], &[u8]); 3] = [
            (b"ic07", &png128),
            (b"ic08", &png256),
            (b"ic09", &png512),
        ];
        let total_len: usize = 8 + entries.iter().map(|(_, d)| 8 + d.len()).sum::<usize>();
        let mut icns = Vec::with_capacity(total_len);
        icns.extend_from_slice(b"icns");
        icns.extend_from_slice(&(total_len as u32).to_be_bytes());
        for (tag, data) in entries {
            icns.extend_from_slice(tag);
            icns.extend_from_slice(&((8 + data.len()) as u32).to_be_bytes());
            icns.extend_from_slice(data);
        }
        fs::write(p, icns).map_err(|e| e.to_string())?;
    }

    if let Some(p) = ico_out {
        let sizes: [(u8, usize, Vec<u8>); 4] = [
            (16, 16, encode_png_rgba(16, 16, &render_procedural_orb_rgba(16))),
            (32, 32, encode_png_rgba(32, 32, &render_procedural_orb_rgba(32))),
            (48, 48, encode_png_rgba(48, 48, &render_procedural_orb_rgba(48))),
            (0, 256, png256),
        ];
        let count = sizes.len();
        let header_len = 6 + count * 16;
        let data_len: usize = sizes.iter().map(|(_, _, d)| d.len()).sum();
        let mut ico = Vec::with_capacity(header_len + data_len);
        ico.extend_from_slice(&[0, 0, 1, 0, count as u8, 0]);
        let mut offset = header_len as u32;
        for (dim_byte, _, png_bytes) in &sizes {
            ico.extend_from_slice(&[*dim_byte, *dim_byte, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(png_bytes.len() as u32).to_le_bytes());
            ico.extend_from_slice(&offset.to_le_bytes());
            offset += png_bytes.len() as u32;
        }
        for (_, _, png_bytes) in &sizes {
            ico.extend_from_slice(png_bytes);
        }
        fs::write(p, ico).map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sapphire_ball_icon_renders_valid_png() {
        let rgba = render_procedural_orb_rgba(256);
        assert_eq!(rgba.len(), 256 * 256 * 4);
        let png = encode_png_rgba(256, 256, &rgba);
        assert!(png.starts_with(&[137, 80, 78, 71]));
        if let Ok(preview_path) = std::env::var("BLUEORB_PREVIEW_ICON") {
            let rgba512 = render_procedural_orb_rgba(512);
            let png512 = encode_png_rgba(512, 512, &rgba512);
            fs::write(preview_path, png512).unwrap();
        }
    }
}
