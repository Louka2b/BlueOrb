// Détection multi-plateforme de la chaîne d'outils ARM (macOS ARM64, Linux, Windows x86_64)

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ResolvedToolchain {
    pub assembler: PathBuf,
    pub linker: PathBuf,
    pub objcopy: PathBuf,
    pub objdump: Option<PathBuf>,
    pub clang_cxx: Option<PathBuf>,
    pub gcc_cxx: Option<PathBuf>,
    pub armcc_cxx: Option<PathBuf>,
}

impl ResolvedToolchain {
    pub fn host_summary() -> String {
        format!("{}-{}", env::consts::OS, env::consts::ARCH)
    }

    pub fn resolve(
        cfg_clang: &str,
        cfg_gcc: &str,
        cfg_armcc: &str,
        cfg_as: &str,
        cfg_ld: &str,
        cfg_objcopy: &str,
        cfg_objdump: &str,
    ) -> Result<Self, String> {
        let search_dirs = candidate_search_dirs();

        let assembler = find_program(cfg_as, &search_dirs).ok_or_else(|| {
            missing_tool_help("arm-none-eabi-as (GNU Binutils ARM)")
        })?;
        let linker = find_program(cfg_ld, &search_dirs).ok_or_else(|| {
            missing_tool_help("arm-none-eabi-ld (GNU Binutils ARM)")
        })?;
        let objcopy = find_program(cfg_objcopy, &search_dirs).ok_or_else(|| {
            missing_tool_help("arm-none-eabi-objcopy (GNU Binutils ARM)")
        })?;
        let objdump = find_program(cfg_objdump, &search_dirs);

        let clang_cxx = find_program(cfg_clang, &search_dirs).filter(|p| clang_supports_arm(p));
        let gcc_cxx = find_program(cfg_gcc, &search_dirs);
        let armcc_cxx = find_program(cfg_armcc, &search_dirs);

        if clang_cxx.is_none() && gcc_cxx.is_none() && armcc_cxx.is_none() {
            return Err(missing_tool_help(
                "compilateur C++ cible ARM (clang++ avec backend ARM ou arm-none-eabi-g++)",
            ));
        }

        Ok(Self {
            assembler,
            linker,
            objcopy,
            objdump,
            clang_cxx,
            gcc_cxx,
            armcc_cxx,
        })
    }
}

pub fn candidate_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // 1. Dossiers du PATH courant
    if let Some(path_os) = env::var_os("PATH") {
        for p in env::split_paths(&path_os) {
            if !p.as_os_str().is_empty() {
                dirs.push(p);
            }
        }
    }

    // 2. Variables d'environnement devkitPro / devkitARM / ARM_GCC
    for var in ["DEVKITARM", "ARM_TOOLCHAIN_PATH"] {
        if let Ok(val) = env::var(var) {
            let pb = PathBuf::from(&val);
            dirs.push(pb.join("bin"));
            dirs.push(pb);
        }
    }
    if let Ok(dkp) = env::var("DEVKITPRO") {
        dirs.push(PathBuf::from(dkp).join("devkitARM").join("bin"));
    }

    // 3. Emplacements standards selon l'OS (macOS ARM64/Intel, Linux, Windows x86_64)
    if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/opt/homebrew/opt/llvm/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/opt/devkitpro/devkitARM/bin"));
        dirs.push(PathBuf::from("/Applications/ARM/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
    } else if cfg!(target_os = "windows") {
        dirs.push(PathBuf::from(r"C:\devkitPro\devkitARM\bin"));
        dirs.push(PathBuf::from(r"C:\Program Files\LLVM\bin"));
        dirs.push(PathBuf::from(r"C:\msys64\ucrt64\bin"));
        dirs.push(PathBuf::from(r"C:\msys64\mingw64\bin"));

        if let Ok(userprofile) = env::var("USERPROFILE") {
            let up = PathBuf::from(userprofile);
            dirs.push(up.join("scoop").join("shims"));
            dirs.push(up.join(".cargo").join("bin"));
        }
        if let Ok(localapp) = env::var("LOCALAPPDATA") {
            scan_arm_gnu_subdirs(&PathBuf::from(localapp).join("Programs"), &mut dirs);
        }
        for root in [r"C:\Program Files", r"C:\Program Files (x86)"] {
            scan_arm_gnu_subdirs(Path::new(root), &mut dirs);
        }
    } else {
        // Linux (x86_64 / aarch64) & BSD
        dirs.push(PathBuf::from("/opt/devkitpro/devkitARM/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
        dirs.push(PathBuf::from("/bin"));
    }

    dirs
}

fn scan_arm_gnu_subdirs(parent: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let mut found: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.contains("arm gnu toolchain") || name.contains("gnu arm embedded") || name.contains("arm-none-eabi") {
            let p = entry.path();
            if p.join("bin").is_dir() {
                found.push(p.join("bin"));
            }
            if let Ok(sub) = fs::read_dir(&p) {
                for sub_entry in sub.flatten() {
                    let sp = sub_entry.path().join("bin");
                    if sp.is_dir() {
                        found.push(sp);
                    }
                }
            }
        }
    }
    // Trier en ordre décroissant pour privilégier la version la plus récente
    found.sort();
    found.reverse();
    out.extend(found);
}

pub fn find_program(name: &str, search_dirs: &[PathBuf]) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        if is_executable_file(p) {
            return Some(p.to_path_buf());
        }
        if cfg!(target_os = "windows") && p.extension().is_none() {
            let with_exe = p.with_extension("exe");
            if is_executable_file(&with_exe) {
                return Some(with_exe);
            }
        }
        return None;
    }

    for dir in search_dirs {
        let cand = dir.join(name);
        if is_executable_file(&cand) {
            return Some(cand);
        }
        if cfg!(target_os = "windows") && !name.to_lowercase().ends_with(".exe") {
            let cand_exe = dir.join(format!("{}.exe", name));
            if is_executable_file(&cand_exe) {
                return Some(cand_exe);
            }
        }
    }
    None
}

fn is_executable_file(p: &Path) -> bool {
    let Ok(meta) = fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        (meta.permissions().mode() & 0o111) != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn clang_supports_arm(clang_path: &Path) -> bool {
    let Ok(out) = Command::new(clang_path).arg("--print-targets").output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().any(|l| {
        let t = l.trim();
        t.starts_with("arm ") || t.starts_with("arm-") || t.starts_with("arm\t")
    })
}

pub fn missing_tool_help(missing: &str) -> String {
    let install_hint = if cfg!(target_os = "macos") {
        "Sur macOS (Apple Silicon ARM64 / Intel), installez la toolchain ARM via Homebrew :\n  \
         brew install --cask gcc-arm-embedded\n  \
         xcode-select --install"
    } else if cfg!(target_os = "windows") {
        "Sur Windows (x86_64), installez la toolchain ARM et LLVM via Winget (PowerShell) :\n  \
         winget install -e --id Arm.GnuArmEmbeddedToolchain\n  \
         winget install -e --id LLVM.LLVM\n  \
         (Redémarrez ensuite votre terminal ou lancez `blueorb doctor`)"
    } else {
        "Sur Linux (Ubuntu/Debian/Arch/Fedora), installez les paquets ARM :\n  \
         Ubuntu/Debian : sudo apt install binutils-arm-none-eabi gcc-arm-none-eabi g++-arm-none-eabi clang\n  \
         Arch Linux    : sudo pacman -S arm-none-eabi-binutils arm-none-eabi-gcc clang\n  \
         Fedora        : sudo dnf install arm-none-eabi-binutils-cs arm-none-eabi-gcc-cs-c++ clang"
    };

    format!("Outil requis introuvable : {}\n\n{}", missing, install_hint)
}

pub fn run_doctor(
    cfg_clang: &str,
    cfg_gcc: &str,
    cfg_armcc: &str,
    cfg_as: &str,
    cfg_ld: &str,
    cfg_objcopy: &str,
    cfg_objdump: &str,
) -> Result<(), String> {
    println!("=== BlueOrb Doctor ({}) ===", ResolvedToolchain::host_summary());
    let dirs = candidate_search_dirs();

    let check = |label: &str, name: &str, req: bool| -> Option<PathBuf> {
        match find_program(name, &dirs) {
            Some(p) => {
                let ver = Command::new(&p)
                    .arg("--version")
                    .output()
                    .ok()
                    .and_then(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .next()
                            .map(|s| s.trim().to_string())
                    })
                    .unwrap_or_default();
                println!("  [OK] {:24} -> {} ({})", label, p.display(), ver);
                Some(p)
            }
            None => {
                if req {
                    println!("  [!!] {:24} -> INTROUVABLE (requis)", label);
                } else {
                    println!("  [--] {:24} -> non installé (optionnel)", label);
                }
                None
            }
        }
    };

    let as_p = check("Assembler (GNU as)", cfg_as, true);
    let ld_p = check("Linker (GNU ld)", cfg_ld, true);
    let oc_p = check("Objcopy", cfg_objcopy, true);
    let _od_p = check("Objdump", cfg_objdump, false);
    let clang_p = check("Clang++ (ARMv6K)", cfg_clang, false);
    if let Some(ref cp) = clang_p {
        if clang_supports_arm(cp) {
            println!("       └─ Cible ARMv6K supportée par Clang++ : OUI");
        } else {
            println!("       └─ Cible ARMv6K supportée par Clang++ : NON (bascule auto sur GCC)");
        }
    }
    let gcc_p = check("GCC ARM (g++)", cfg_gcc, false);
    let _armcc_p = check("ARMCC (RVCT 4.1)", cfg_armcc, false);

    if as_p.is_none() || ld_p.is_none() || oc_p.is_none() {
        println!();
        return Err(missing_tool_help("GNU Binutils arm-none-eabi"));
    }
    let has_valid_clang = clang_p.as_ref().map(|p| clang_supports_arm(p)).unwrap_or(false);
    if !has_valid_clang && gcc_p.is_none() {
        println!();
        return Err(missing_tool_help("clang++ (ARM) ou arm-none-eabi-g++"));
    }

    println!("\nEnvironnement prêt pour la décompilation et recompilation sur {} !", ResolvedToolchain::host_summary());
    Ok(())
}
