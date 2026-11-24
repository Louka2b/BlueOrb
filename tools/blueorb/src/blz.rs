// Décompression et compression BLZ (Backward LZ77 Nintendo 3DS pour ExeFS .code)

pub fn decompress_blz(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 8 {
        return Ok(data.to_vec());
    }
    let add_size = u32::from_le_bytes(data[data.len() - 4..].try_into().unwrap()) as usize;
    if add_size == 0 {
        return Ok(data.to_vec());
    }
    let hdr_and_comp = u32::from_le_bytes(data[data.len() - 8..data.len() - 4].try_into().unwrap());
    let hdr_size = ((hdr_and_comp >> 24) & 0xFF) as usize;
    let comp_size = (hdr_and_comp & 0x00FF_FFFF) as usize;

    if hdr_size < 8 || comp_size > data.len() || hdr_size > comp_size {
        return Err(format!(
            "Pied de page BLZ invalide: hdr_size=0x{:X}, comp_size=0x{:X}, len=0x{:X}",
            hdr_size,
            comp_size,
            data.len()
        ));
    }

    let total_out = data
        .len()
        .checked_add(add_size)
        .ok_or_else(|| "Débordement de taille BLZ".to_string())?;
    let mut out = vec![0u8; total_out];
    let uncomp_prefix = data.len() - comp_size;
    out[..uncomp_prefix].copy_from_slice(&data[..uncomp_prefix]);

    let mut src = data.len() - hdr_size;
    let mut dst = total_out;
    let src_end = uncomp_prefix;

    while dst > uncomp_prefix && src > src_end {
        src -= 1;
        let mut flags = data[src];
        for _ in 0..8 {
            if dst <= uncomp_prefix || src <= src_end {
                break;
            }
            if (flags & 0x80) == 0 {
                src -= 1;
                dst -= 1;
                out[dst] = data[src];
            } else {
                if src < src_end + 2 {
                    return Err("Flux BLZ tronqué sur une paire LZ".to_string());
                }
                src -= 2;
                let pair = (data[src] as usize) | ((data[src + 1] as usize) << 8);
                let length = (pair >> 12) + 3;
                let disp = (pair & 0x0FFF) + 3;
                for _ in 0..length {
                    if dst == 0 || dst - 1 + disp >= total_out {
                        return Err(format!(
                            "Référence arrière BLZ hors limites: dst=0x{:X}, disp=0x{:X}",
                            dst, disp
                        ));
                    }
                    dst -= 1;
                    out[dst] = out[dst + disp];
                }
            }
            flags <<= 1;
        }
    }

    if dst != uncomp_prefix {
        return Err(format!(
            "Décompression BLZ incomplète: dst=0x{:X}, attendu=0x{:X}",
            dst, uncomp_prefix
        ));
    }

    Ok(out)
}

/// Compression Backward LZ77 compatible 3DS ExeFS `.code` (avec table de hachage rapide).
pub fn compress_blz(raw: &[u8]) -> Vec<u8> {
    if raw.len() < 16 {
        return raw.to_vec();
    }

    let n = raw.len();
    // Chaîne de hachage sur triplets (pos -> prochaine pos plus à droite avec même hash, <= 0x1002)
    const HASH_BITS: usize = 15;
    const HASH_SIZE: usize = 1 << HASH_BITS;
    let mut head = vec![u32::MAX; HASH_SIZE];
    let mut next_pos = vec![u32::MAX; n];

    // On parcourt de droite à gauche pour chaîner les positions déjà disponibles dans le futur décompressé
    // Lors de la décompression à rebours, à la position `pos` (on écrit raw[pos - len .. pos]),
    // la fenêtre de référence est `pos + 3 ..= min(pos + 0x1002, n)` (fin de match).
    // Plus simplement: si on inverse le buffer `rev[i] = raw[n - 1 - i]`, BLZ est exactement un LZ77 standard avant!
    let mut rev = Vec::with_capacity(n);
    for &b in raw.iter().rev() {
        rev.push(b);
    }

    let hash3 = |buf: &[u8], i: usize| -> usize {
        let v = ((buf[i] as usize) << 16) | ((buf[i + 1] as usize) << 8) | (buf[i + 2] as usize);
        (v.wrapping_mul(0x9E3779B1) >> (32 - HASH_BITS)) & (HASH_SIZE - 1)
    };

    // Compression LZ77 avant sur `rev` avec distance [3..=0x1002] et longueur [3..=18]
    // On enregistre le flux inversé et on Suit le "pire écart" pour calculer la portion sûre en place (in-place safety).
    let mut rev_comp: Vec<u8> = Vec::with_capacity(n);
    let mut pos = 0usize;

    // On traite `rev` de 0 à n
    // Pour garantir la décompression en place du noyau 3DS (`dst` ne doit jamais être rattrapé par `src`),
    // on garde la meilleure frontière préfixe non-compressé (`best_save`).
    let mut best_gain: isize = 0;
    let mut best_raw_done: usize = 0;
    let mut best_comp_len: usize = 0;

    while pos < n {
        let flag_idx = rev_comp.len();
        rev_comp.push(0);
        let mut flags = 0u8;

        for bit in 0..8 {
            if pos >= n {
                break;
            }

            let mut best_len = 0usize;
            let mut best_disp = 0usize;

            if pos + 3 <= n && pos >= 3 {
                let h = hash3(&rev, pos);
                let mut cand = head[h];
                let min_cand = pos.saturating_sub(0x1002);
                let max_len = 18.min(n - pos);
                let mut steps = 0usize;

                while cand != u32::MAX && (cand as usize) >= min_cand && steps < 64 {
                    let c = cand as usize;
                    let dist = pos - c;
                    if dist >= 3 {
                        if rev[c + best_len.saturating_sub(1)] == rev[pos + best_len.saturating_sub(1)]
                            && rev[c] == rev[pos]
                            && rev[c + 1] == rev[pos + 1]
                            && rev[c + 2] == rev[pos + 2]
                        {
                            let mut l = 3;
                            while l < max_len && rev[c + l] == rev[pos + l] {
                                l += 1;
                            }
                            if l > best_len {
                                best_len = l;
                                best_disp = dist;
                                if best_len == max_len {
                                    break;
                                }
                            }
                        }
                    }
                    cand = next_pos[c];
                    steps += 1;
                }
            }

            if best_len >= 3 {
                flags |= 0x80 >> bit;
                let pair = (((best_len - 3) as u16) << 12) | ((best_disp - 3) as u16);
                // Dans BLZ inversé, `src -= 2; pair = data[src] | (data[src+1] << 8)`
                // Donc dans `rev_comp` (ordre inverse de `data`), l'octet haut (`data[src+1]`) vient d'abord, puis l'octet bas (`data[src]`).
                rev_comp.push((pair >> 8) as u8);
                rev_comp.push((pair & 0xFF) as u8);

                for k in 0..best_len {
                    if pos + k + 3 <= n {
                        let h = hash3(&rev, pos + k);
                        next_pos[pos + k] = head[h];
                        head[h] = (pos + k) as u32;
                    }
                }
                pos += best_len;
            } else {
                rev_comp.push(rev[pos]);
                if pos + 3 <= n {
                    let h = hash3(&rev, pos);
                    next_pos[pos] = head[h];
                    head[h] = pos as u32;
                }
                pos += 1;
            }
        }

        rev_comp[flag_idx] = flags;

        let gain = (pos as isize) - (rev_comp.len() as isize);
        if gain >= best_gain {
            best_gain = gain;
            best_raw_done = pos;
            best_comp_len = rev_comp.len();
        }
    }

    if best_gain <= 12 {
        // Pas de gain utile: retourner avec footer add_size = 0 ou tel quel
        return raw.to_vec();
    }

    let uncomp_prefix_len = n - best_raw_done;
    let comp_body_len = best_comp_len;
    let mut out = Vec::with_capacity(uncomp_prefix_len + comp_body_len + 12);
    out.extend_from_slice(&raw[..uncomp_prefix_len]);
    for &b in rev_comp[..comp_body_len].iter().rev() {
        out.push(b);
    }

    // Alignement sur 4 octets avant les 8 octets de métadonnées BLZ (hdr_size entre 8 et 11)
    let pad = (4 - (out.len() % 4)) % 4;
    for _ in 0..pad {
        out.push(0xFF);
    }
    let hdr_size = 8 + pad;
    let comp_total = comp_body_len + hdr_size;
    let total_file_len = out.len() + 8;
    let add_size = (n - total_file_len) as u32;
    let hdr_and_comp = ((hdr_size as u32) << 24) | ((comp_total as u32) & 0x00FF_FFFF);

    out.extend_from_slice(&hdr_and_comp.to_le_bytes());
    out.extend_from_slice(&add_size.to_le_bytes());
    out
}
