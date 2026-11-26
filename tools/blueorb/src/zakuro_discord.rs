//! BlueOrb Discord Rich Presence (Local IPC — zero external dependencies).
//!
//! Connects directly to the local Discord desktop client's IPC pipe/socket,
//! displays the official Pokémon Alpha Sapphire boxart/logo + Blue Orb badge,
//! live Hoenn Co-op / Battle / Cutscene activity, and a clickable button
//! linking to BlueOrb on GitHub.

use std::io::{Read, Write};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{Color32, RichText};

pub const ALPHA_SAPPHIRE_LOGO_URL: &str = "https://art.gametdb.com/3ds/coverHQ/US/ECLA.jpg";
pub const BLUE_ORB_BADGE_URL: &str =
    "https://raw.githubusercontent.com/PokeAPI/sprites/master/sprites/items/blue-orb.png";
pub const GITHUB_REPO_URL: &str = "https://github.com/Louka2b/BlueOrb";
/// Optional baked-in Discord Application ID ("Pokémon Alpha Sapphire").
/// Can also be injected at build time via `BLUEORB_DISCORD_CLIENT_ID`.
pub const DEFAULT_DISCORD_CLIENT_ID: &str = "";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RpcSnapshot {
    pub in_game: bool,
    pub zone_id: Option<u16>,
    pub in_battle: bool,
    pub in_cutscene: bool,
    pub party_count: u8,
    pub coop_members: usize,
    pub is_leader: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RpcConnectionState {
    #[default]
    Disabled,
    NeedsClientId,
    Connecting,
    Connected,
    Unavailable,
}

enum RpcMsg {
    Config {
        enabled: bool,
        client_id: String,
    },
    Snapshot(RpcSnapshot),
    Stop,
}

pub struct DiscordRpcRuntime {
    tx: Sender<RpcMsg>,
    state: Arc<Mutex<RpcConnectionState>>,
    last_enabled: Option<bool>,
    last_client_id: String,
    last_snapshot: Option<RpcSnapshot>,
    last_sent: Instant,
}

impl DiscordRpcRuntime {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(RpcConnectionState::Disabled));
        let worker_state = state.clone();
        let (tx, rx) = channel::<RpcMsg>();

        std::thread::Builder::new()
            .name("blueorb-discord-rpc".into())
            .spawn(move || rpc_worker_loop(rx, worker_state))
            .ok();

        let now = Instant::now();
        Self {
            tx,
            state,
            last_enabled: None,
            last_client_id: String::new(),
            last_snapshot: None,
            last_sent: now - Duration::from_secs(10),
        }
    }

    pub fn state(&self) -> RpcConnectionState {
        self.state.lock().map(|s| *s).unwrap_or(RpcConnectionState::Unavailable)
    }

    pub fn update(&mut self, enabled: bool, client_id: &str, snap: RpcSnapshot) {
        let effective_id = resolved_client_id(client_id);
        let cfg_changed =
            self.last_enabled != Some(enabled) || self.last_client_id != effective_id;
        if cfg_changed {
            self.last_enabled = Some(enabled);
            self.last_client_id = effective_id.clone();
            let _ = self.tx.send(RpcMsg::Config {
                enabled,
                client_id: effective_id,
            });
        }

        if self.last_snapshot.as_ref() != Some(&snap)
            && self.last_sent.elapsed() >= Duration::from_secs(4)
        {
            self.last_snapshot = Some(snap.clone());
            self.last_sent = Instant::now();
            let _ = self.tx.send(RpcMsg::Snapshot(snap));
        }
    }
}

impl Drop for DiscordRpcRuntime {
    fn drop(&mut self) {
        let _ = self.tx.send(RpcMsg::Stop);
    }
}

fn builtin_client_id() -> &'static str {
    let compile_time = option_env!("BLUEORB_DISCORD_CLIENT_ID").unwrap_or("").trim();
    if !compile_time.is_empty() {
        return compile_time;
    }
    DEFAULT_DISCORD_CLIENT_ID.trim()
}

fn resolved_client_id(configured: &str) -> String {
    let trimmed = configured.trim();
    if !trimmed.is_empty() {
        return trimmed.to_owned();
    }
    if let Ok(runtime_env) = std::env::var("BLUEORB_DISCORD_CLIENT_ID") {
        let rt = runtime_env.trim();
        if !rt.is_empty() {
            return rt.to_owned();
        }
    }
    builtin_client_id().to_owned()
}

pub fn encode_ipc_frame(opcode: u32, payload: &str) -> Vec<u8> {
    let body = payload.as_bytes();
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&opcode.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

pub fn build_activity_payload(pid: u32, start_epoch: u64, snap: &RpcSnapshot) -> String {
    let details = if !snap.in_game {
        "In BlueOrb Launcher".to_owned()
    } else if snap.in_battle {
        "In Battle — Pokémon Alpha Sapphire".to_owned()
    } else if snap.in_cutscene {
        "Story Cutscene — Hoenn".to_owned()
    } else if let Some(z) = snap.zone_id {
        format!("Exploring Hoenn (Zone {z})")
    } else {
        "Playing Pokémon Alpha Sapphire".to_owned()
    };

    let state = if snap.coop_members >= 2 {
        let role = if snap.is_leader { "Chef" } else { "Co-op" };
        if snap.party_count > 0 {
            format!("{role} · {} Party Pokémon", snap.party_count)
        } else {
            format!("{role} · {} Players", snap.coop_members)
        }
    } else if snap.in_game && snap.party_count > 0 {
        format!("Solo Adventure · {} Party Pokémon", snap.party_count)
    } else {
        "Native AOT Recompilation".to_owned()
    };

    let party_json = if snap.coop_members >= 2 {
        format!(
            ",\"party\":{{\"id\":\"blueorb-coop\",\"size\":[{},16]}}",
            snap.coop_members.clamp(2, 16)
        )
    } else {
        String::new()
    };

    format!(
        "{{\"cmd\":\"SET_ACTIVITY\",\"args\":{{\"pid\":{pid},\"activity\":{{\"details\":\"{det}\",\"state\":\"{st}\",\"timestamps\":{{\"start\":{start_epoch}}},\"assets\":{{\"large_image\":\"{logo}\",\"large_text\":\"Pokémon Alpha Sapphire — BlueOrb\",\"small_image\":\"{badge}\",\"small_text\":\"BlueOrb Native AOT\"}},\"buttons\":[{{\"label\":\"BlueOrb on GitHub\",\"url\":\"{repo}\"}}]{party}}}}},\"nonce\":\"{pid}-{start_epoch}\"}}",
        det = json_escape(&details),
        st = json_escape(&state),
        logo = json_escape(ALPHA_SAPPHIRE_LOGO_URL),
        badge = json_escape(BLUE_ORB_BADGE_URL),
        repo = json_escape(GITHUB_REPO_URL),
        party = party_json,
    )
}

#[cfg(unix)]
type IpcStream = std::os::unix::net::UnixStream;
#[cfg(windows)]
type IpcStream = std::fs::File;

#[cfg(unix)]
fn connect_discord_ipc() -> Option<IpcStream> {
    let mut dirs: Vec<String> = Vec::new();
    for var in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                dirs.push(v);
            }
        }
    }
    dirs.push("/tmp".to_owned());

    for dir in dirs {
        let base = dir.trim_end_matches('/');
        for sub in ["", "/app/com.discordapp.Discord", "/snap.discord"] {
            for idx in 0..10 {
                let candidate = format!("{base}{sub}/discord-ipc-{idx}");
                if let Ok(stream) = std::os::unix::net::UnixStream::connect(&candidate) {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(800)));
                    return Some(stream);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn connect_discord_ipc() -> Option<IpcStream> {
    for idx in 0..10 {
        let pipe = format!(r"\\.\pipe\discord-ipc-{idx}");
        if let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&pipe) {
            return Some(file);
        }
    }
    None
}

fn ipc_send_and_drain(stream: &mut IpcStream, opcode: u32, payload: &str) -> bool {
    let frame = encode_ipc_frame(opcode, payload);
    if stream.write_all(&frame).is_err() || stream.flush().is_err() {
        return false;
    }
    let mut hdr = [0u8; 8];
    if stream.read_exact(&mut hdr).is_err() {
        return false;
    }
    let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    if len > 0 && len <= 256 * 1024 {
        let mut discard = vec![0u8; len];
        if stream.read_exact(&mut discard).is_err() {
            return false;
        }
    }
    true
}

fn rpc_worker_loop(rx: std::sync::mpsc::Receiver<RpcMsg>, state: Arc<Mutex<RpcConnectionState>>) {
    let pid = std::process::id();
    let start_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut enabled = false;
    let mut client_id = String::new();
    let mut latest_snap = RpcSnapshot::default();
    let mut conn: Option<IpcStream> = None;
    let mut dirty = false;

    loop {
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(RpcMsg::Config {
                enabled: en,
                client_id: id,
            }) => {
                if enabled != en || client_id != id {
                    conn = None;
                }
                enabled = en;
                client_id = id;
                dirty = true;
            }
            Ok(RpcMsg::Snapshot(snap)) => {
                latest_snap = snap;
                dirty = true;
            }
            Ok(RpcMsg::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }

        while let Ok(msg) = rx.try_recv() {
            match msg {
                RpcMsg::Config {
                    enabled: en,
                    client_id: id,
                } => {
                    if enabled != en || client_id != id {
                        conn = None;
                    }
                    enabled = en;
                    client_id = id;
                    dirty = true;
                }
                RpcMsg::Snapshot(snap) => {
                    latest_snap = snap;
                    dirty = true;
                }
                RpcMsg::Stop => return,
            }
        }

        if !enabled {
            conn = None;
            if let Ok(mut s) = state.lock() {
                *s = RpcConnectionState::Disabled;
            }
            continue;
        }

        if client_id.is_empty() {
            conn = None;
            if let Ok(mut s) = state.lock() {
                *s = RpcConnectionState::NeedsClientId;
            }
            continue;
        }

        if conn.is_none() {
            if let Ok(mut s) = state.lock() {
                *s = RpcConnectionState::Connecting;
            }
            if let Some(mut stream) = connect_discord_ipc() {
                let handshake = format!("{{\"v\":1,\"client_id\":\"{}\"}}", json_escape(&client_id));
                if ipc_send_and_drain(&mut stream, 0, &handshake) {
                    conn = Some(stream);
                    dirty = true;
                }
            }
        }

        if let Some(stream) = &mut conn {
            if dirty {
                let activity = build_activity_payload(pid, start_epoch, &latest_snap);
                if ipc_send_and_drain(stream, 1, &activity) {
                    dirty = false;
                    if let Ok(mut s) = state.lock() {
                        *s = RpcConnectionState::Connected;
                    }
                } else {
                    conn = None;
                    if let Ok(mut s) = state.lock() {
                        *s = RpcConnectionState::Unavailable;
                    }
                }
            } else if let Ok(mut s) = state.lock() {
                *s = RpcConnectionState::Connected;
            }
        } else if let Ok(mut s) = state.lock() {
            *s = RpcConnectionState::Unavailable;
        }
    }
}

/// Renders the Discord Rich Presence configuration card inside `Esc -> System & Cheats`.
pub fn draw_discord_rpc_section(
    ui: &mut egui::Ui,
    enabled: &mut bool,
    client_id: &mut String,
    state: RpcConnectionState,
) {
    let has_builtin = !builtin_client_id().is_empty();
    let (status_txt, status_col) = match state {
        RpcConnectionState::Connected => ("Connected", Color32::from_rgb(122, 238, 174)),
        RpcConnectionState::Connecting => ("Connecting…", Color32::from_rgb(154, 204, 255)),
        RpcConnectionState::NeedsClientId => ("1-time App ID needed", Color32::from_rgb(255, 214, 102)),
        RpcConnectionState::Unavailable => ("Discord not running", Color32::from_rgb(176, 184, 198)),
        RpcConnectionState::Disabled => ("Disabled", Color32::from_rgb(142, 146, 158)),
    };

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Discord Rich Presence")
                .size(14.0)
                .strong()
                .color(Color32::from_rgb(236, 236, 240)),
        );
        ui.label(RichText::new(format!("·  {status_txt}")).size(11.5).color(status_col));
    });
    ui.add_space(4.0);
    ui.checkbox(enabled, "Show Pokémon Alpha Sapphire cover, Hoenn status & GitHub link on Discord");
    if has_builtin {
        ui.collapsing("Advanced: Override built-in Discord App ID", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Custom App ID").size(11.5).color(Color32::from_rgb(184, 192, 208)));
                ui.add(
                    egui::TextEdit::singleline(client_id)
                        .desired_width(180.0)
                        .hint_text("Leave empty to use BlueOrb default"),
                );
            });
        });
    } else {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Discord App ID").size(12.0).color(Color32::from_rgb(184, 192, 208)));
            ui.add(
                egui::TextEdit::singleline(client_id)
                    .desired_width(180.0)
                    .hint_text("Paste Application ID"),
            );
            if ui.button("Open Portal").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab("https://discord.com/developers/applications"));
            }
        });
        ui.label(
            RichText::new("Discord requires a registered App ID (no assets needed — create a free app named \"Pokémon Alpha Sapphire\" once and paste its ID).")
                .size(10.5)
                .color(Color32::from_rgb(142, 152, 170)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_frame_encodes_opcode_length_and_payload() {
        let frame = encode_ipc_frame(1, r#"{"ok":true}"#);
        assert_eq!(&frame[0..4], &1u32.to_le_bytes());
        assert_eq!(&frame[4..8], &(11u32).to_le_bytes());
        assert_eq!(&frame[8..], br#"{"ok":true}"#);
    }

    #[test]
    fn activity_payload_includes_alpha_sapphire_logo_github_button_and_coop_state() {
        let snap = RpcSnapshot {
            in_game: true,
            zone_id: Some(102),
            in_battle: false,
            in_cutscene: false,
            party_count: 3,
            coop_members: 2,
            is_leader: true,
        };
        let payload = build_activity_payload(4242, 1_700_000_000, &snap);
        assert!(payload.contains(ALPHA_SAPPHIRE_LOGO_URL));
        assert!(payload.contains(GITHUB_REPO_URL));
        assert!(payload.contains("BlueOrb on GitHub"));
        assert!(payload.contains("Exploring Hoenn (Zone 102)"));
        assert!(payload.contains("Chef · 3 Party Pokémon"));
    }
}
