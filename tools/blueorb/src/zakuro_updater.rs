//! BlueOrb In-Place Launcher Auto-Updater.
//!
//! Checks `update-manifest.json` from GitHub Releases in the background, verifies SHA-256
//! checksums before touching anything, atomically swaps ONLY the launcher executable / `.app`
//! bundle (leaving saves, settings, ROM path, and already-recompiled native libraries untouched),
//! and relaunches automatically.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Color32, CornerRadius, RichText, Stroke};
use sha2::{Digest, Sha256};

use crate::menus::Action;

pub const CURRENT_BUILD_COMMIT: &str = "__BLUEORB_BUILD_COMMIT__";
const MANIFEST_URL: &str = "https://github.com/Louka2b/BlueOrb/releases/latest/download/update-manifest.json";
const DOWNLOAD_BASE: &str = "https://github.com/Louka2b/BlueOrb/releases/latest/download";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    pub file: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateManifest {
    pub commit: String,
    pub tag: String,
    #[allow(dead_code)]
    pub published_at: String,
    pub windows_x64: Option<AssetEntry>,
    pub macos_arm64: Option<AssetEntry>,
    pub linux_x64: Option<AssetEntry>,
}

impl UpdateManifest {
    pub fn asset_for_current_platform(&self) -> Option<&AssetEntry> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("windows", "x86_64") => self.windows_x64.as_ref(),
            ("macos", "aarch64") => self.macos_arm64.as_ref(),
            ("linux", "x86_64") => self.linux_x64.as_ref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate { commit: String },
    Available { tag: String, commit: String, file: String, sha256: String },
    Downloading { file: String, percent: u8 },
    Installing,
    ReadyToRestart,
    Error(String),
}

pub struct UpdaterRuntime {
    state: Arc<Mutex<UpdateStatus>>,
    auto_started: bool,
    busy: Arc<AtomicBool>,
}

impl UpdaterRuntime {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(UpdateStatus::Idle)),
            auto_started: false,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn auto_started(&self) -> bool {
        self.auto_started
    }

    pub fn should_exit_for_restart(&self) -> bool {
        matches!(self.status(), UpdateStatus::ReadyToRestart)
    }

    pub fn short_commit() -> String {
        let trimmed = CURRENT_BUILD_COMMIT.trim();
        if trimmed.is_empty() || trimmed == "__BLUEORB_BUILD_COMMIT__" || trimmed == "dev" {
            "dev".to_owned()
        } else {
            trimmed.chars().take(7).collect()
        }
    }

    /// Starts a background check against GitHub Releases `update-manifest.json`.
    pub fn start_check(&mut self, manual: bool) {
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(mut s) = self.state.lock() {
            *s = UpdateStatus::Checking;
        }
        let state = self.state.clone();
        let busy = self.busy.clone();
        std::thread::Builder::new()
            .name("blueorb-update-check".into())
            .spawn(move || {
                let res = fetch_manifest().and_then(|manifest| {
                    let asset = manifest
                        .asset_for_current_platform()
                        .cloned()
                        .ok_or_else(|| "No release package for this OS/architecture.".to_owned())?;
                    Ok((manifest, asset))
                });

                if let Ok(mut s) = state.lock() {
                    match res {
                        Ok((manifest, asset)) => {
                            if is_update_available(CURRENT_BUILD_COMMIT, &manifest.commit) {
                                *s = UpdateStatus::Available {
                                    tag: manifest.tag,
                                    commit: manifest.commit,
                                    file: asset.file,
                                    sha256: asset.sha256,
                                };
                            } else {
                                *s = UpdateStatus::UpToDate {
                                    commit: manifest.commit,
                                };
                            }
                        }
                        Err(err) => {
                            if manual {
                                *s = UpdateStatus::Error(err);
                            } else {
                                *s = UpdateStatus::Idle;
                            }
                        }
                    }
                }
                busy.store(false, Ordering::SeqCst);
            })
            .ok();
    }

    /// Downloads, SHA-256 verifies, stages, and schedules an in-place launcher swap + restart.
    pub fn start_install(&mut self, auto_triggered: bool) {
        let (file, expected_sha) = match self.status() {
            UpdateStatus::Available { file, sha256, .. } => (file, sha256),
            _ => return,
        };
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        if auto_triggered {
            self.auto_started = true;
        }
        if let Ok(mut s) = self.state.lock() {
            *s = UpdateStatus::Downloading {
                file: file.clone(),
                percent: 0,
            };
        }

        let state = self.state.clone();
        let busy = self.busy.clone();
        std::thread::Builder::new()
            .name("blueorb-update-install".into())
            .spawn(move || {
                let outcome = download_verify_and_stage(&file, &expected_sha, &state);
                if let Ok(mut s) = state.lock() {
                    match outcome {
                        Ok(()) => *s = UpdateStatus::ReadyToRestart,
                        Err(err) => *s = UpdateStatus::Error(err),
                    }
                }
                busy.store(false, Ordering::SeqCst);
            })
            .ok();
    }
}

pub fn is_update_available(current_commit: &str, remote_commit: &str) -> bool {
    let cur = current_commit.trim().to_ascii_lowercase();
    let rem = remote_commit.trim().to_ascii_lowercase();
    if rem.len() < 7 || cur.len() < 7 {
        return false;
    }
    if cur == "dev" || cur == "__blueorb_build_commit__" {
        return false;
    }
    !rem.starts_with(&cur) && !cur.starts_with(&rem)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn verify_sha256_bytes(bytes: &[u8], expected_hex: &str) -> Result<(), String> {
    let expected = expected_hex.trim().to_ascii_lowercase();
    if expected.len() != 64 {
        return Err("Invalid SHA-256 checksum in manifest.".to_owned());
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "SHA-256 verification failed (expected {}, got {}).",
            &expected[..12],
            &actual[..12]
        ))
    }
}

fn verify_sha256_file(path: &Path, expected_hex: &str) -> Result<(), String> {
    let expected = expected_hex.trim().to_ascii_lowercase();
    if expected.len() != 64 {
        return Err("Invalid SHA-256 checksum in manifest.".to_owned());
    }
    let mut f = fs::File::open(path).map_err(|e| format!("Failed to open update archive: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("Failed reading update archive: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "SHA-256 verification failed (expected {}, got {}).",
            &expected[..12],
            &actual[..12]
        ))
    }
}

pub fn parse_manifest(json: &str) -> Option<UpdateManifest> {
    let commit = extract_json_string(json, "commit")?;
    let tag = extract_json_string(json, "tag").unwrap_or_else(|| "latest".to_owned());
    let published_at = extract_json_string(json, "published_at").unwrap_or_default();
    let assets_obj = extract_json_object(json, "assets")?;

    let parse_asset = |key: &str| -> Option<AssetEntry> {
        let block = extract_json_object(assets_obj, key)?;
        Some(AssetEntry {
            file: extract_json_string(block, "file")?,
            sha256: extract_json_string(block, "sha256")?,
        })
    };

    Some(UpdateManifest {
        commit,
        tag,
        published_at,
        windows_x64: parse_asset("windows-x64"),
        macos_arm64: parse_asset("macos-arm64"),
        linux_x64: parse_asset("linux-x64"),
    })
}

fn extract_json_string(src: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let start = src.find(&pat)? + pat.len();
    let rest = src[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut esc = false;
    for ch in rest.chars() {
        if esc {
            out.push(ch);
            esc = false;
        } else if ch == '\\' {
            esc = true;
        } else if ch == '"' {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn extract_json_object<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let start = src.find(&pat)? + pat.len();
    let rest = src[start..].trim_start().strip_prefix(':')?.trim_start();
    if !rest.starts_with('{') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (idx, ch) in rest.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rest[..=idx]);
                }
            }
            _ => {}
        }
    }
    None
}

fn curl_program() -> &'static str {
    if cfg!(windows) {
        "curl.exe"
    } else {
        "curl"
    }
}

fn silent_command(prog: &str) -> Command {
    let cmd = Command::new(prog);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = cmd;
        cmd.creation_flags(CREATE_NO_WINDOW);
        return cmd;
    }
    #[allow(unreachable_code)]
    cmd
}

fn fetch_manifest() -> Result<UpdateManifest, String> {
    let out = silent_command(curl_program())
        .args([
            "-fsSL",
            "--connect-timeout",
            "8",
            "--max-time",
            "15",
            MANIFEST_URL,
        ])
        .output()
        .map_err(|e| format!("Could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("Manifest download returned status {}", out.status));
    }
    let body = String::from_utf8(out.stdout).map_err(|_| "Manifest is not UTF-8.".to_owned())?;
    parse_manifest(&body).ok_or_else(|| "Invalid update-manifest.json format.".to_owned())
}

fn probe_content_length(url: &str) -> Option<u64> {
    let out = silent_command(curl_program())
        .args(["-fsSIL", "--connect-timeout", "6", "--max-time", "10", url])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let headers = String::from_utf8_lossy(&out.stdout);
    headers
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse::<u64>().ok()
            } else {
                None
            }
        })
        .next_back()
}

fn unique_update_dir() -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("blueorb_update_{}_{}", std::process::id(), stamp));
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create temp folder: {e}"))?;
    Ok(dir)
}

fn download_verify_and_stage(
    file: &str,
    expected_sha: &str,
    state: &Arc<Mutex<UpdateStatus>>,
) -> Result<(), String> {
    let url = format!("{DOWNLOAD_BASE}/{file}");
    let total_bytes = probe_content_length(&url);
    let work_dir = unique_update_dir()?;
    let dl_path = work_dir.join(file);

    let mut child = silent_command(curl_program())
        .args([
            "-fL",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "15",
            "--max-time",
            "600",
            "-o",
        ])
        .arg(&dl_path)
        .arg(&url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start update download: {e}"))?;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let _ = fs::remove_dir_all(&work_dir);
                    return Err(format!("Update download failed ({status})."));
                }
                break;
            }
            Ok(None) => {
                if let (Some(total), Ok(meta)) = (total_bytes, fs::metadata(&dl_path)) {
                    if total > 0 {
                        let pct = ((meta.len() * 100) / total).min(99) as u8;
                        if let Ok(mut s) = state.lock() {
                            *s = UpdateStatus::Downloading {
                                file: file.to_owned(),
                                percent: pct,
                            };
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&work_dir);
                return Err(format!("Download wait error: {e}"));
            }
        }
    }

    if let Ok(mut s) = state.lock() {
        *s = UpdateStatus::Installing;
    }

    if let Err(err) = verify_sha256_file(&dl_path, expected_sha) {
        let _ = fs::remove_dir_all(&work_dir);
        return Err(err);
    }

    if let Err(err) = install_and_spawn_relauncher(&work_dir, &dl_path) {
        let _ = fs::remove_dir_all(&work_dir);
        return Err(err);
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn find_current_app_bundle(exe: &Path) -> Option<PathBuf> {
    let mut cur = Some(exe);
    while let Some(p) = cur {
        if p.extension().is_some_and(|e| e == "app") && p.is_dir() {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

#[cfg(target_os = "macos")]
fn install_and_spawn_relauncher(work_dir: &Path, archive_path: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate running executable: {e}"))?;
    let current_app = find_current_app_bundle(&exe)
        .ok_or_else(|| "BlueOrb must be inside BlueOrb.app to auto-update on macOS.".to_owned())?;
    let app_str = current_app.to_string_lossy();
    if app_str.starts_with("/Volumes/") {
        return Err("Drag BlueOrb.app to Applications first so auto-updates can write to it.".to_owned());
    }

    let stage_dir = work_dir.join("staged");
    fs::create_dir_all(&stage_dir).map_err(|e| format!("Staging dir error: {e}"))?;
    let ok = Command::new("ditto")
        .arg("-x")
        .arg("-k")
        .arg(archive_path)
        .arg(&stage_dir)
        .status()
        .map_err(|e| format!("Failed to extract update zip: {e}"))?;
    if !ok.success() {
        return Err("Could not unpack macOS update zip.".to_owned());
    }

    let staged_app = stage_dir.join("BlueOrb.app");
    if !staged_app.is_dir() {
        return Err("Update archive does not contain BlueOrb.app.".to_owned());
    }
    let _ = Command::new("xattr").args(["-cr"]).arg(&staged_app).status();

    let pid = std::process::id();
    let script = format!(
        "PID={pid}\nCUR=\"{cur}\"\nNEW=\"{new}\"\nWORK=\"{work}\"\nwhile kill -0 \"$PID\" 2>/dev/null; do sleep 0.2; done\nrm -rf \"${{CUR}}.old\"\nmv \"$CUR\" \"${{CUR}}.old\" && mv \"$NEW\" \"$CUR\" || {{ mv \"${{CUR}}.old\" \"$CUR\" 2>/dev/null || true; exit 1; }}\nrm -rf \"${{CUR}}.old\" \"$WORK\"\nxattr -cr \"$CUR\" 2>/dev/null || true\nopen -n \"$CUR\"\n",
        pid = pid,
        cur = current_app.to_string_lossy().replace('"', "\\\""),
        new = staged_app.to_string_lossy().replace('"', "\\\""),
        work = work_dir.to_string_lossy().replace('"', "\\\""),
    );
    Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start updater helper: {e}"))?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn install_and_spawn_relauncher(work_dir: &Path, archive_path: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate running exe: {e}"))?;
    let parent = exe.parent().ok_or_else(|| "Invalid executable directory.".to_owned())?;
    let staged_exe = parent.join("BlueOrb.exe.update_tmp");
    fs::copy(archive_path, &staged_exe)
        .map(|_| ())
        .or_else(|_| fs::rename(archive_path, &staged_exe))
        .map_err(|e| format!("Could not write staged Windows executable: {e}"))?;

    let cur_s = exe.to_string_lossy().to_string();
    let new_s = staged_exe.to_string_lossy().to_string();
    let work_s = work_dir.to_string_lossy().to_string();
    let bat_path = work_dir.join("apply_update.bat");
    let bat = format!(
        "@echo off\r\n:wait\r\ntimeout /t 1 /nobreak >nul\r\nmove /Y \"{new_s}\" \"{cur_s}\" >nul 2>&1\r\nif errorlevel 1 goto wait\r\nstart \"\" \"{cur_s}\"\r\nrmdir /S /Q \"{work_s}\" >nul 2>&1\r\n"
    );
    fs::write(&bat_path, bat).map_err(|e| format!("Could not write Windows update script: {e}"))?;

    silent_command("cmd")
        .args(["/C"])
        .arg(&bat_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start Windows updater helper: {e}"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_and_spawn_relauncher(work_dir: &Path, archive_path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let exe = std::env::current_exe().map_err(|e| format!("Cannot locate running executable: {e}"))?;
    let stage_dir = work_dir.join("staged");
    fs::create_dir_all(&stage_dir).map_err(|e| format!("Staging dir error: {e}"))?;
    let ok = Command::new("tar")
        .arg("-xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(&stage_dir)
        .status()
        .map_err(|e| format!("Failed to extract Linux archive: {e}"))?;
    if !ok.success() {
        return Err("Could not unpack Linux update archive.".to_owned());
    }
    let staged_bin = stage_dir.join("BlueOrb");
    if !staged_bin.is_file() {
        return Err("Linux archive does not contain BlueOrb executable.".to_owned());
    }
    fs::set_permissions(&staged_bin, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("chmod failed: {e}"))?;

    let backup = exe.with_extension("old_tmp");
    let _ = fs::remove_file(&backup);
    fs::rename(&exe, &backup).map_err(|e| format!("Cannot replace current binary: {e}"))?;
    if let Err(err) = fs::copy(&staged_bin, &exe) {
        let _ = fs::rename(&backup, &exe);
        return Err(format!("Could not install updated binary: {err}"));
    }
    let _ = fs::set_permissions(&exe, fs::Permissions::from_mode(0o755));
    let _ = fs::remove_file(&backup);
    let _ = fs::remove_dir_all(work_dir);

    Command::new(&exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Updated binary written, but relaunch failed: {e}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn install_and_spawn_relauncher(_work_dir: &Path, _archive_path: &Path) -> Result<(), String> {
    Err("Self-update is only supported on macOS, Windows, and Linux.".to_owned())
}

/// Compact pill for the Launcher top header showing live update state + quick click action.
pub fn draw_launcher_update_bar(ui: &mut egui::Ui, status: &UpdateStatus) -> Option<Action> {
    match status {
        UpdateStatus::Checking => {
            ui.label(RichText::new("Checking updates…").size(11.5).color(Color32::from_rgb(174, 218, 255)));
            None
        }
        UpdateStatus::Available { tag, commit, .. } => {
            let short: String = commit.chars().take(7).collect();
            let resp = ui.add(
                egui::Button::new(
                    RichText::new(format!("Update {tag} ({short})"))
                        .size(12.0)
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(Color32::from_rgb(42, 132, 232))
                .stroke(Stroke::new(1.0, Color32::from_rgb(138, 206, 255)))
                .corner_radius(CornerRadius::same(10)),
            );
            if resp.clicked() { Some(Action::InstallUpdate) } else { None }
        }
        UpdateStatus::Downloading { percent, .. } => {
            ui.label(
                RichText::new(format!("Updating {percent}%…"))
                    .size(12.0)
                    .strong()
                    .color(Color32::from_rgb(132, 224, 178)),
            );
            None
        }
        UpdateStatus::Installing | UpdateStatus::ReadyToRestart => {
            ui.label(
                RichText::new("Restarting to apply update…")
                    .size(12.0)
                    .strong()
                    .color(Color32::from_rgb(255, 226, 134)),
            );
            None
        }
        UpdateStatus::UpToDate { .. } | UpdateStatus::Idle | UpdateStatus::Error(_) => {
            let resp = ui.add(
                egui::Button::new(RichText::new("Updates").size(12.0).color(Color32::from_rgb(220, 232, 250)))
                    .fill(Color32::from_rgba_unmultiplied(12, 22, 42, 168))
                    .stroke(Stroke::new(1.0, Color32::from_rgba_unmultiplied(164, 208, 255, 76)))
                    .corner_radius(CornerRadius::same(10)),
            );
            if resp.clicked() { Some(Action::CheckUpdates) } else { None }
        }
    }
}

/// Update management card shown inside `Esc -> System & Cheats`.
pub fn draw_pause_update_section(
    ui: &mut egui::Ui,
    status: &UpdateStatus,
    auto_update: &mut bool,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let short_cur = UpdaterRuntime::short_commit();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Launcher Updates").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
        ui.label(RichText::new(format!("·  build {short_cur}")).size(11.5).monospace().color(Color32::from_rgb(142, 164, 194)));
    });
    ui.add_space(4.0);
    ui.checkbox(auto_update, "Auto-update launcher on startup when a new release is available");
    ui.add_space(6.0);

    match status {
        UpdateStatus::Idle => {
            if ui.button("Check for Updates").clicked() {
                actions.push(Action::CheckUpdates);
            }
        }
        UpdateStatus::Checking => {
            ui.label(RichText::new("Checking GitHub Releases…").size(12.0).color(Color32::from_rgb(154, 204, 255)));
        }
        UpdateStatus::UpToDate { commit } => {
            let short_rem: String = commit.chars().take(7).collect();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Up to date ({short_rem})")).size(12.0).color(Color32::from_rgb(122, 238, 174)));
                if ui.button("Check Again").clicked() {
                    actions.push(Action::CheckUpdates);
                }
            });
        }
        UpdateStatus::Available { tag, commit, file, .. } => {
            let short_rem: String = commit.chars().take(7).collect();
            ui.label(RichText::new(format!("New release {tag} ({short_rem}) — {file}")).size(12.0).color(Color32::from_rgb(255, 226, 134)));
            if ui.button("Download, Install & Restart").clicked() {
                actions.push(Action::InstallUpdate);
            }
        }
        UpdateStatus::Downloading { file, percent } => {
            ui.label(RichText::new(format!("Downloading {file} ({percent}%)…")).size(12.0));
            ui.add(egui::ProgressBar::new((*percent as f32) / 100.0).show_percentage());
        }
        UpdateStatus::Installing | UpdateStatus::ReadyToRestart => {
            ui.label(RichText::new("Swapping launcher in-place & restarting (saves kept safe)…").size(12.0).color(Color32::from_rgb(255, 226, 134)));
        }
        UpdateStatus::Error(err) => {
            ui.label(RichText::new(format!("Update check failed: {err}")).size(11.5).color(Color32::from_rgb(255, 146, 146)));
            if ui.button("Retry Check").clicked() {
                actions.push(Action::CheckUpdates);
            }
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parser_extracts_commit_and_platform_assets() {
        let sample = r#"{
          "commit": "0123456789abcdef0123456789abcdef01234567",
          "tag": "latest",
          "published_at": "2026-11-26T20:00:00Z",
          "assets": {
            "windows-x64": {
              "file": "BlueOrb-Windows-x64.exe",
              "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "macos-arm64": {
              "file": "BlueOrb-macOS-AppleSilicon.zip",
              "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            },
            "linux-x64": {
              "file": "BlueOrb-Linux-x86_64.tar.gz",
              "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
            }
          }
        }"#;
        let m = parse_manifest(sample).unwrap();
        assert_eq!(m.commit, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(m.tag, "latest");
        assert_eq!(m.windows_x64.unwrap().file, "BlueOrb-Windows-x64.exe");
        assert_eq!(m.macos_arm64.unwrap().file, "BlueOrb-macOS-AppleSilicon.zip");
        assert_eq!(m.linux_x64.unwrap().file, "BlueOrb-Linux-x86_64.tar.gz");
    }

    #[test]
    fn version_comparison_detects_new_commit_and_ignores_same_or_dev() {
        let sha_a = "abcdef1234567890abcdef1234567890abcdef12";
        let sha_b = "9999991234567890abcdef1234567890abcdef12";
        assert!(!is_update_available("dev", sha_a));
        assert!(!is_update_available("__BLUEORB_BUILD_COMMIT__", sha_a));
        assert!(!is_update_available(sha_a, sha_a));
        assert!(!is_update_available("abcdef1", sha_a));
        assert!(is_update_available(sha_a, sha_b));
    }

    #[test]
    fn sha256_verification_accepts_matching_and_rejects_tampered() {
        let data = b"BlueOrb legal launcher payload";
        let mut h = Sha256::new();
        h.update(data);
        let good = format!("{:x}", h.finalize());
        assert!(verify_sha256_bytes(data, &good).is_ok());
        assert!(verify_sha256_bytes(b"tampered", &good).is_err());
        assert!(verify_sha256_bytes(data, "short").is_err());
    }
}
