// Désassembleur pur Rust ARMv6K + Thumb + VFPv2 (Nintendo 3DS MPCore)

const COND_NAMES: [&str; 16] = [
    "eq", "ne", "hs", "lo", "mi", "pl", "vs", "vc",
    "hi", "ls", "ge", "lt", "gt", "le", "", "nv",
];

const REG_NAMES: [&str; 16] = [
    "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7",
    "r8", "r9", "sl", "fp", "ip", "sp", "lr", "pc",
];

const DP_OPCODES: [&str; 16] = [
    "and", "eor", "sub", "rsb", "add", "adc", "sbc", "rsc",
    "tst", "teq", "cmp", "cmn", "orr", "mov", "bic", "mvn",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchKind {
    Jump,
    CallArm,
    CallThumb,
}

#[derive(Clone, Debug)]
pub struct DecodedInst {
    pub size: u32, // 2 ou 4 octets
    pub text: String,
    pub branch_target: Option<(u32, BranchKind)>,
    pub pool_load_addr: Option<u32>,
}

#[inline]
fn reg(r: u32) -> &'static str {
    REG_NAMES[(r & 0xF) as usize]
}

#[inline]
fn cond_str(c: u32) -> &'static str {
    COND_NAMES[(c & 0xF) as usize]
}

fn format_reglist(mask: u32) -> String {
    let mut list = Vec::new();
    for i in 0..16 {
        if (mask & (1 << i)) != 0 {
            list.push(REG_NAMES[i]);
        }
    }
    format!("{{{}}}", list.join(", "))
}

fn decode_arm_shifted_reg(word: u32) -> String {
    let rm = reg(word & 0xF);
    let shift_type = (word >> 5) & 0x3;
    let st_name = match shift_type {
        0 => "lsl",
        1 => "lsr",
        2 => "asr",
        _ => "ror",
    };
    if (word & (1 << 4)) == 0 {
        let amt = (word >> 7) & 0x1F;
        if amt == 0 {
            if shift_type == 0 {
                return rm.to_string();
            } else if shift_type == 3 {
                return format!("{}, rrx", rm);
            } else {
                return format!("{}, {} #32", rm, st_name);
            }
        }
        format!("{}, {} #{}", rm, st_name, amt)
    } else {
        let rs = reg((word >> 8) & 0xF);
        format!("{}, {} {}", rm, st_name, rs)
    }
}

pub fn decode_arm<F>(pc: u32, word: u32, sym_lookup: F) -> DecodedInst
where
    F: Fn(u32) -> Option<String>,
{
    let cond = (word >> 28) & 0xF;

    // ARM BLX <label> (inconditionnel cond == 0xF : 0xFA / 0xFB)
    if cond == 0xF {
        if (word & 0xFE00_0000) == 0xFA00_0000 {
            let h = ((word >> 24) & 1) << 1;
            let imm24 = word & 0x00FF_FFFF;
            let sign_ext = ((imm24 as i32) << 8) >> 6;
            let target = (pc.wrapping_add(8) as i32).wrapping_add(sign_ext) as u32 | h;
            let label = sym_lookup(target).unwrap_or_else(|| format!("0x{:08X}", target));
            return DecodedInst {
                size: 4,
                text: format!("blx {}", label),
                branch_target: Some((target, BranchKind::CallThumb)),
                pool_load_addr: None,
            };
        }
        return DecodedInst {
            size: 4,
            text: format!(".word 0x{:08X}", word),
            branch_target: None,
            pool_load_addr: None,
        };
    }

    let cc = cond_str(cond);
    let top3 = (word >> 25) & 0x7;

    // Branchements B / BL (top3 == 0b101)
    if top3 == 0b101 {
        let is_link = (word & (1 << 24)) != 0;
        let imm24 = word & 0x00FF_FFFF;
        let offset = ((imm24 as i32) << 8) >> 6;
        let target = (pc.wrapping_add(8) as i32).wrapping_add(offset) as u32;
        let mnem = if is_link {
            format!("bl{}", cc)
        } else {
            format!("b{}", cc)
        };
        let label = sym_lookup(target).unwrap_or_else(|| format!("0x{:08X}", target));
        let kind = if is_link {
            BranchKind::CallArm
        } else {
            BranchKind::Jump
        };
        return DecodedInst {
            size: 4,
            text: format!("{} {}", mnem, label),
            branch_target: Some((target, kind)),
            pool_load_addr: None,
        };
    }

    // Coprocesseur VFPv2 (cp10 / cp11) & SVC (top3 == 0b110 ou 0b111)
    if top3 == 0b110 || top3 == 0b111 {
        if (word & 0x0F00_0000) == 0x0F00_0000 {
            let imm = word & 0x00FF_FFFF;
            return DecodedInst {
                size: 4,
                text: format!("svc{} #0x{:X}", cc, imm),
                branch_target: None,
                pool_load_addr: None,
            };
        }
        if let Some((vfp_txt, pool)) = decode_vfp(pc, word, cc) {
            return DecodedInst {
                size: 4,
                text: vfp_txt,
                branch_target: None,
                pool_load_addr: pool,
            };
        }
    }

    // BX / BLX Rm / CLZ
    if (word & 0x0FFF_FFF0) == 0x012F_FF10 {
        let rm = reg(word & 0xF);
        return DecodedInst {
            size: 4,
            text: format!("bx{} {}", cc, rm),
            branch_target: None,
            pool_load_addr: None,
        };
    }
    if (word & 0x0FFF_FFF0) == 0x012F_FF30 {
        let rm = reg(word & 0xF);
        return DecodedInst {
            size: 4,
            text: format!("blx{} {}", cc, rm),
            branch_target: None,
            pool_load_addr: None,
        };
    }
    if (word & 0x0FFF_0FF0) == 0x016F_0F10 {
        let rd = reg((word >> 12) & 0xF);
        let rm = reg(word & 0xF);
        return DecodedInst {
            size: 4,
            text: format!("clz{} {}, {}", cc, rd, rm),
            branch_target: None,
            pool_load_addr: None,
        };
    }

    // Multiplications MUL / MLA / UMULL / SMULL
    if (word & 0x0FC0_00F0) == 0x0000_0090 {
        let accumulate = (word & (1 << 21)) != 0;
        let s = if (word & (1 << 20)) != 0 { "s" } else { "" };
        let rd = reg((word >> 16) & 0xF);
        let rn = reg((word >> 12) & 0xF);
        let rs = reg((word >> 8) & 0xF);
        let rm = reg(word & 0xF);
        let text = if accumulate {
            format!("mla{}{} {}, {}, {}, {}", s, cc, rd, rm, rs, rn)
        } else {
            format!("mul{}{} {}, {}, {}", s, cc, rd, rm, rs)
        };
        return DecodedInst {
            size: 4,
            text,
            branch_target: None,
            pool_load_addr: None,
        };
    }

    // Extensions ARMv6 (UXTB, UXTH, SXTB, SXTH)
    if (word & 0x0F8F_03F0) == 0x068F_0070 {
        let op = (word >> 20) & 0x7;
        let rd = reg((word >> 12) & 0xF);
        let rm = reg(word & 0xF);
        let rot = ((word >> 10) & 0x3) * 8;
        let mnem = match op {
            0b010 => Some("sxtb"),
            0b011 => Some("sxth"),
            0b110 => Some("uxtb"),
            0b111 => Some("uxth"),
            _ => None,
        };
        if let Some(m) = mnem {
            let rot_s = if rot == 0 {
                String::new()
            } else {
                format!(", ror #{}", rot)
            };
            return DecodedInst {
                size: 4,
                text: format!("{}{} {}, {}{}", m, cc, rd, rm, rot_s),
                branch_target: None,
                pool_load_addr: None,
            };
        }
    }

    // Extra Load/Store (LDRH, STRH, LDRSB, LDRSH, LDRD, STRD)
    if top3 == 0b000 && (word & 0x90) == 0x90 && (word & 0x60) != 0 {
        let p = (word & (1 << 24)) != 0;
        let u = (word & (1 << 23)) != 0;
        let imm_mode = (word & (1 << 22)) != 0;
        let w = (word & (1 << 21)) != 0;
        let l = (word & (1 << 20)) != 0;
        let rn = (word >> 16) & 0xF;
        let rd = (word >> 12) & 0xF;
        let sh = (word >> 5) & 0x3;

        let mnem_opt = match (l, sh) {
            (false, 1) => Some("strh"),
            (true, 1) => Some("ldrh"),
            (true, 2) => Some("ldrsb"),
            (true, 3) => Some("ldrsh"),
            (false, 2) => Some("ldrd"),
            (false, 3) => Some("strd"),
            _ => None,
        };
        if let Some(mnem) = mnem_opt {
            let sign = if u { "" } else { "-" };
            let addr_s = if imm_mode {
                let imm8 = ((word >> 4) & 0xF0) | (word & 0xF);
                if p {
                    if imm8 == 0 && u && !w {
                        format!("[{}]", reg(rn))
                    } else {
                        format!(
                            "[{}, #{}0x{:X}]{}",
                            reg(rn),
                            sign,
                            imm8,
                            if w { "!" } else { "" }
                        )
                    }
                } else {
                    format!("[{}], #{}0x{:X}", reg(rn), sign, imm8)
                }
            } else {
                let rm = reg(word & 0xF);
                if p {
                    format!(
                        "[{}, {}{}]{}",
                        reg(rn),
                        sign,
                        rm,
                        if w { "!" } else { "" }
                    )
                } else {
                    format!("[{}], {}{}", reg(rn), sign, rm)
                }
            };
            let regs_s = if mnem == "ldrd" || mnem == "strd" {
                format!("{}, {}", reg(rd), reg(rd + 1))
            } else {
                reg(rd).to_string()
            };
            return DecodedInst {
                size: 4,
                text: format!("{}{} {}, {}", mnem, cc, regs_s, addr_s),
                branch_target: None,
                pool_load_addr: None,
            };
        }
    }

    // Data Processing (top3 == 0b000 ou 0b001)
    if top3 == 0b000 || top3 == 0b001 {
        let is_imm = top3 == 0b001;
        let opcode = (word >> 21) & 0xF;
        let s_bit = (word & (1 << 20)) != 0;
        let rn = (word >> 16) & 0xF;
        let rd = (word >> 12) & 0xF;

        // Eviter les encodages spéciaux (MRS/MSR/BX non pris plus haut) lorsque S==0 et opcode 8..=11
        if !(opcode >= 8 && opcode <= 11 && !s_bit) {
            if word == 0xE1A0_0000 {
                return DecodedInst {
                    size: 4,
                    text: "nop".into(),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
            let op2 = if is_imm {
                let imm8 = word & 0xFF;
                let rot = ((word >> 8) & 0xF) * 2;
                let val = imm8.rotate_right(rot);
                if val <= 9 {
                    format!("#{}", val)
                } else {
                    format!("#0x{:X}", val)
                }
            } else {
                decode_arm_shifted_reg(word)
            };

            let base = DP_OPCODES[opcode as usize];
            let s_str = if s_bit && !(8..=11).contains(&opcode) {
                "s"
            } else {
                ""
            };
            let text = if (8..=11).contains(&opcode) {
                format!("{}{} {}, {}", base, cc, reg(rn), op2)
            } else if opcode == 13 || opcode == 15 {
                format!("{}{}{} {}, {}", base, s_str, cc, reg(rd), op2)
            } else {
                format!("{}{}{} {}, {}, {}", base, s_str, cc, reg(rd), reg(rn), op2)
            };
            return DecodedInst {
                size: 4,
                text,
                branch_target: None,
                pool_load_addr: None,
            };
        }
    }

    // Single Data Transfer LDR / STR / LDRB / STRB (top3 == 0b010 ou 0b011)
    if top3 == 0b010 || top3 == 0b011 {
        if top3 == 0b011 && (word & 0x10) != 0 {
            // Instructions media ARMv6 non décodées individuellement
            return DecodedInst {
                size: 4,
                text: format!(".inst 0x{:08X}", word),
                branch_target: None,
                pool_load_addr: None,
            };
        }
        let is_reg = top3 == 0b011;
        let p = (word & (1 << 24)) != 0;
        let u = (word & (1 << 23)) != 0;
        let b = (word & (1 << 22)) != 0;
        let w = (word & (1 << 21)) != 0;
        let l = (word & (1 << 20)) != 0;
        let rn = (word >> 16) & 0xF;
        let rd = (word >> 12) & 0xF;

        let mnem = match (l, b) {
            (true, false) => "ldr",
            (true, true) => "ldrb",
            (false, false) => "str",
            (false, true) => "strb",
        };
        let sign = if u { "" } else { "-" };
        let mut pool_addr = None;

        let addr_s = if !is_reg {
            let imm12 = word & 0xFFF;
            if rn == 15 && p && !w && l && !b {
                let target = if u {
                    pc.wrapping_add(8).wrapping_add(imm12)
                } else {
                    pc.wrapping_add(8).wrapping_sub(imm12)
                };
                pool_addr = Some(target);
            }
            if p {
                if imm12 == 0 && u && !w {
                    format!("[{}]", reg(rn))
                } else {
                    format!(
                        "[{}, #{}0x{:X}]{}",
                        reg(rn),
                        sign,
                        imm12,
                        if w { "!" } else { "" }
                    )
                }
            } else {
                format!("[{}], #{}0x{:X}", reg(rn), sign, imm12)
            }
        } else {
            let shifted = decode_arm_shifted_reg(word);
            if p {
                format!(
                    "[{}, {}{}]{}",
                    reg(rn),
                    sign,
                    shifted,
                    if w { "!" } else { "" }
                )
            } else {
                format!("[{}], {}{}", reg(rn), sign, shifted)
            }
        };

        return DecodedInst {
            size: 4,
            text: format!("{}{} {}, {}", mnem, cc, reg(rd), addr_s),
            branch_target: None,
            pool_load_addr: pool_addr,
        };
    }

    // Block Data Transfer LDM / STM / PUSH / POP (top3 == 0b100)
    if top3 == 0b100 {
        let p = (word & (1 << 24)) != 0;
        let u = (word & (1 << 23)) != 0;
        let s = (word & (1 << 22)) != 0;
        let w = (word & (1 << 21)) != 0;
        let l = (word & (1 << 20)) != 0;
        let rn = (word >> 16) & 0xF;
        let reglist = word & 0xFFFF;

        if !s && rn == 13 && w && reglist.count_ones() >= 2 {
            if !l && p && !u {
                return DecodedInst {
                    size: 4,
                    text: format!("push{} {}", cc, format_reglist(reglist)),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
            if l && !p && u {
                return DecodedInst {
                    size: 4,
                    text: format!("pop{} {}", cc, format_reglist(reglist)),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
        }

        let mode = match (p, u) {
            (false, true) => "ia",
            (true, true) => "ib",
            (false, false) => "da",
            (true, false) => "db",
        };
        let base = if l { "ldm" } else { "stm" };
        let wb = if w { "!" } else { "" };
        let caret = if s { "^" } else { "" };
        return DecodedInst {
            size: 4,
            text: format!(
                "{}{}{} {}{}, {}{}",
                base,
                mode,
                cc,
                reg(rn),
                wb,
                format_reglist(reglist),
                caret
            ),
            branch_target: None,
            pool_load_addr: None,
        };
    }

    DecodedInst {
        size: 4,
        text: format!(".inst 0x{:08X}", word),
        branch_target: None,
        pool_load_addr: None,
    }
}

fn decode_vfp(pc: u32, word: u32, cc: &str) -> Option<(String, Option<u32>)> {
    let coproc = (word >> 8) & 0xF;
    if coproc != 10 && coproc != 11 {
        return None;
    }
    let is_dp = coproc == 11;
    let top4 = (word >> 24) & 0xF;

    // VLDR / VSTR (0xD ou 0xC avec P=1, W=0)
    if (top4 == 0xD || top4 == 0xC) && (word & (1 << 24)) != 0 && (word & (1 << 21)) == 0 {
        let u = (word & (1 << 23)) != 0;
        let d = (word >> 22) & 1;
        let l = (word & (1 << 20)) != 0;
        let rn = (word >> 16) & 0xF;
        let vd = (word >> 12) & 0xF;
        let imm = (word & 0xFF) * 4;
        let fpreg = if is_dp {
            format!("d{}", (d << 4) | vd)
        } else {
            format!("s{}", (vd << 1) | d)
        };
        let mnem = if l { "vldr" } else { "vstr" };
        let sign = if u { "" } else { "-" };
        let mut pool = None;
        if rn == 15 && l && !is_dp {
            pool = Some(if u {
                pc.wrapping_add(8).wrapping_add(imm)
            } else {
                pc.wrapping_add(8).wrapping_sub(imm)
            });
        }
        let addr = if imm == 0 && u {
            format!("[{}]", reg(rn))
        } else {
            format!("[{}, #{}0x{:X}]", reg(rn), sign, imm)
        };
        return Some((format!("{}{} {}, {}", mnem, cc, fpreg, addr), pool));
    }

    // VMRS APSR_nzcv, fpscr
    if (word & 0x0FFF_FFFF) == 0x0EF1_FA10 {
        return Some((format!("vmrs{} APSR_nzcv, fpscr", cc), None));
    }

    // Opérations arithmétiques VFP (0xE avec bit 4 == 0)
    if top4 == 0xE && (word & 0x10) == 0 && !is_dp {
        let d = (word >> 22) & 1;
        let vd = ((word >> 12) & 0xF) << 1 | d;
        let vn = ((word >> 16) & 0xF) << 1 | ((word >> 7) & 1);
        let vm = (word & 0xF) << 1 | ((word >> 5) & 1);
        let opc1 = (word >> 20) & 0xB;
        let opc2 = (word >> 6) & 1;

        let bin_op = match (opc1, opc2) {
            (0x0, 0) => Some("vmla.f32"),
            (0x0, 1) => Some("vmls.f32"),
            (0x2, 0) => Some("vmul.f32"),
            (0x3, 0) => Some("vadd.f32"),
            (0x3, 1) => Some("vsub.f32"),
            (0x8, 0) => Some("vdiv.f32"),
            _ => None,
        };
        if let Some(op) = bin_op {
            return Some((format!("{}{} s{}, s{}, s{}", op, cc, vd, vn, vm), None));
        }
    }

    None
}

pub fn decode_thumb<F>(pc: u32, hw1: u16, hw2_opt: Option<u16>, sym_lookup: F) -> DecodedInst
where
    F: Fn(u32) -> Option<String>,
{
    // 32-bit Thumb BL / BLX : hw1 = 11110 S imm10, hw2 = 11 J1 1 J2 imm11 (BL) ou 11 J1 0 J2 imm10 0 (BLX)
    if (hw1 & 0xF800) == 0xF000 {
        if let Some(hw2) = hw2_opt {
            if (hw2 & 0xD000) == 0xD000 || (hw2 & 0xD001) == 0xC000 {
                let is_blx = (hw2 & 0x1000) == 0;
                let s = ((hw1 >> 10) & 1) as i32;
                let j1 = ((hw2 >> 13) & 1) as i32;
                let j2 = ((hw2 >> 11) & 1) as i32;
                let i1 = (!(j1 ^ s)) & 1;
                let i2 = (!(j2 ^ s)) & 1;
                let imm10 = (hw1 & 0x03FF) as i32;
                let imm11 = (hw2 & 0x07FF) as i32;
                let raw = (s << 24) | (i1 << 23) | (i2 << 22) | (imm10 << 12) | (imm11 << 1);
                let signed_off = (raw << 7) >> 7;
                let base_pc = if is_blx { (pc.wrapping_add(4)) & !3 } else { pc.wrapping_add(4) };
                let target = (base_pc as i32).wrapping_add(signed_off) as u32;
                let label = sym_lookup(target).unwrap_or_else(|| format!("0x{:08X}", target));
                let (mnem, kind) = if is_blx {
                    ("blx", BranchKind::CallArm)
                } else {
                    ("bl", BranchKind::CallThumb)
                };
                return DecodedInst {
                    size: 4,
                    text: format!("{} {}", mnem, label),
                    branch_target: Some((target, kind)),
                    pool_load_addr: None,
                };
            }
        }
    }

    let top3 = (hw1 >> 13) & 0x7;
    match top3 {
        0b000 => {
            let op = (hw1 >> 11) & 0x3;
            let rd = REG_NAMES[(hw1 & 0x7) as usize];
            let rs = REG_NAMES[((hw1 >> 3) & 0x7) as usize];
            if op != 0b11 {
                let amt = (hw1 >> 6) & 0x1F;
                let mnem = match op {
                    0 => "lsls",
                    1 => "lsrs",
                    _ => "asrs",
                };
                if op == 0 && amt == 0 {
                    return DecodedInst {
                        size: 2,
                        text: format!("movs {}, {}", rd, rs),
                        branch_target: None,
                        pool_load_addr: None,
                    };
                }
                return DecodedInst {
                    size: 2,
                    text: format!("{} {}, {}, #{}", mnem, rd, rs, amt),
                    branch_target: None,
                    pool_load_addr: None,
                };
            } else {
                let is_imm = (hw1 & (1 << 10)) != 0;
                let is_sub = (hw1 & (1 << 9)) != 0;
                let rn_or_imm = (hw1 >> 6) & 0x7;
                let mnem = if is_sub { "subs" } else { "adds" };
                let third = if is_imm {
                    format!("#{}", rn_or_imm)
                } else {
                    REG_NAMES[rn_or_imm as usize].to_string()
                };
                return DecodedInst {
                    size: 2,
                    text: format!("{} {}, {}, {}", mnem, rd, rs, third),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
        }
        0b001 => {
            let op = (hw1 >> 11) & 0x3;
            let rd = REG_NAMES[((hw1 >> 8) & 0x7) as usize];
            let imm8 = hw1 & 0xFF;
            let mnem = match op {
                0 => "movs",
                1 => "cmp",
                2 => "adds",
                _ => "subs",
            };
            return DecodedInst {
                size: 2,
                text: format!("{} {}, #0x{:X}", mnem, rd, imm8),
                branch_target: None,
                pool_load_addr: None,
            };
        }
        0b010 => {
            if (hw1 & 0xFC00) == 0x4000 {
                let alu = (hw1 >> 6) & 0xF;
                let rs = REG_NAMES[((hw1 >> 3) & 0x7) as usize];
                let rd = REG_NAMES[(hw1 & 0x7) as usize];
                const T_ALU: [&str; 16] = [
                    "ands", "eors", "lsls", "lsrs", "asrs", "adcs", "sbcs", "rors",
                    "tst", "rsbs", "cmp", "cmn", "orrs", "muls", "bics", "mvns",
                ];
                let txt = if alu == 9 {
                    format!("rsbs {}, {}, #0", rd, rs)
                } else {
                    format!("{} {}, {}", T_ALU[alu as usize], rd, rs)
                };
                return DecodedInst {
                    size: 2,
                    text: txt,
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
            if (hw1 & 0xFC00) == 0x4400 {
                let op = (hw1 >> 8) & 0x3;
                let h1 = (hw1 >> 7) & 1;
                let h2 = (hw1 >> 6) & 1;
                let rs = REG_NAMES[((h2 << 3) | ((hw1 >> 3) & 0x7)) as usize];
                let rd = REG_NAMES[((h1 << 3) | (hw1 & 0x7)) as usize];
                if hw1 == 0x46C0 {
                    return DecodedInst {
                        size: 2,
                        text: "nop".into(),
                        branch_target: None,
                        pool_load_addr: None,
                    };
                }
                let txt = match op {
                    0 => format!("add {}, {}", rd, rs),
                    1 => format!("cmp {}, {}", rd, rs),
                    2 => format!("mov {}, {}", rd, rs),
                    _ => {
                        if h1 != 0 {
                            format!("blx {}", rs)
                        } else {
                            format!("bx {}", rs)
                        }
                    }
                };
                return DecodedInst {
                    size: 2,
                    text: txt,
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
            if (hw1 & 0xF800) == 0x4800 {
                let rd = REG_NAMES[((hw1 >> 8) & 0x7) as usize];
                let imm = ((hw1 & 0xFF) as u32) * 4;
                let pool = ((pc.wrapping_add(4)) & !3).wrapping_add(imm);
                return DecodedInst {
                    size: 2,
                    text: format!("ldr {}, [pc, #0x{:X}]", rd, imm),
                    branch_target: None,
                    pool_load_addr: Some(pool),
                };
            }
        }
        0b011 => {
            let b = (hw1 & (1 << 12)) != 0;
            let l = (hw1 & (1 << 11)) != 0;
            let imm = ((hw1 >> 6) & 0x1F) * if b { 1 } else { 4 };
            let rb = REG_NAMES[((hw1 >> 3) & 0x7) as usize];
            let rd = REG_NAMES[(hw1 & 0x7) as usize];
            let mnem = match (l, b) {
                (true, false) => "ldr",
                (false, false) => "str",
                (true, true) => "ldrb",
                (false, true) => "strb",
            };
            return DecodedInst {
                size: 2,
                text: format!("{} {}, [{}, #0x{:X}]", mnem, rd, rb, imm),
                branch_target: None,
                pool_load_addr: None,
            };
        }
        0b100 => {
            if (hw1 & 0xF000) == 0x8000 {
                let l = (hw1 & (1 << 11)) != 0;
                let imm = ((hw1 >> 6) & 0x1F) * 2;
                let rb = REG_NAMES[((hw1 >> 3) & 0x7) as usize];
                let rd = REG_NAMES[(hw1 & 0x7) as usize];
                let mnem = if l { "ldrh" } else { "strh" };
                return DecodedInst {
                    size: 2,
                    text: format!("{} {}, [{}, #0x{:X}]", mnem, rd, rb, imm),
                    branch_target: None,
                    pool_load_addr: None,
                };
            } else {
                let l = (hw1 & (1 << 11)) != 0;
                let rd = REG_NAMES[((hw1 >> 8) & 0x7) as usize];
                let imm = (hw1 & 0xFF) * 4;
                let mnem = if l { "ldr" } else { "str" };
                return DecodedInst {
                    size: 2,
                    text: format!("{} {}, [sp, #0x{:X}]", mnem, rd, imm),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
        }
        0b101 => {
            if (hw1 & 0xF000) == 0xA000 {
                let is_sp = (hw1 & (1 << 11)) != 0;
                let rd = REG_NAMES[((hw1 >> 8) & 0x7) as usize];
                let imm = (hw1 & 0xFF) * 4;
                let base = if is_sp { "sp" } else { "pc" };
                return DecodedInst {
                    size: 2,
                    text: format!("add {}, {}, #0x{:X}", rd, base, imm),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
            if (hw1 & 0xFF00) == 0xB000 {
                let is_sub = (hw1 & 0x80) != 0;
                let imm = (hw1 & 0x7F) * 4;
                let mnem = if is_sub { "sub" } else { "add" };
                return DecodedInst {
                    size: 2,
                    text: format!("{} sp, #0x{:X}", mnem, imm),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
            if (hw1 & 0xF600) == 0xB400 {
                let is_pop = (hw1 & (1 << 11)) != 0;
                let r_bit = (hw1 & (1 << 8)) != 0;
                let mut mask = (hw1 & 0xFF) as u32;
                if r_bit {
                    mask |= if is_pop { 1 << 15 } else { 1 << 14 };
                }
                let mnem = if is_pop { "pop" } else { "push" };
                return DecodedInst {
                    size: 2,
                    text: format!("{} {}", mnem, format_reglist(mask)),
                    branch_target: None,
                    pool_load_addr: None,
                };
            }
        }
        0b110 => {
            if (hw1 & 0xF000) == 0xD000 {
                let cond = (hw1 >> 8) & 0xF;
                if cond < 14 {
                    let off8 = (hw1 & 0xFF) as i8 as i32;
                    let target = (pc.wrapping_add(4) as i32).wrapping_add(off8 * 2) as u32;
                    let label = sym_lookup(target).unwrap_or_else(|| format!("0x{:08X}", target));
                    return DecodedInst {
                        size: 2,
                        text: format!("b{} {}", COND_NAMES[cond as usize], label),
                        branch_target: Some((target, BranchKind::Jump)),
                        pool_load_addr: None,
                    };
                }
            }
        }
        0b111 => {
            if (hw1 & 0xF800) == 0xE000 {
                let imm11 = (hw1 & 0x07FF) as i32;
                let signed = ((imm11 << 21) >> 20) as i32;
                let target = (pc.wrapping_add(4) as i32).wrapping_add(signed) as u32;
                let label = sym_lookup(target).unwrap_or_else(|| format!("0x{:08X}", target));
                return DecodedInst {
                    size: 2,
                    text: format!("b {}", label),
                    branch_target: Some((target, BranchKind::Jump)),
                    pool_load_addr: None,
                };
            }
        }
        _ => {}
    }

    DecodedInst {
        size: 2,
        text: format!(".inst.n 0x{:04X}", hw1),
        branch_target: None,
        pool_load_addr: None,
    }
}
