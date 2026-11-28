//! BlueOrb Co-op Campaign, Overworld Multiplayer & LAN Discovery Runtime.
//!
//! Features:
//! - Live 3D overworld multiplayer visibility (animated chibi trainer avatars, nameplates, arrows)
//! - Party Leader ("Chef") role with host auto-assignment & leadership transfer
//! - Automatic Guest teleportation (TP) when the Chef changes map/zone (`MapManager`)
//! - Cutscene synchronization barrier ("Waiting for player to finish cutscene...") so neither
//!   player gets interrupted or desynced by story events
//! - Safe 2v2 Double Battle Co-op that strictly preserves solo, tutorial, 1-Pokemon trainer,
//!   Sky, Multi/Ally, and scripted/boss encounters
//! - Strict real save & Pokémon preservation (`DataManager -> PokemonTeam` read-only)
//! - Post-tutorial automatic Co-op Campaign unlock (`party_count >= 1`)

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use egui::{
    Align, Color32, CornerRadius, FontId, Layout, Margin, Pos2, Rect, RichText, Shape, Stroke,
    StrokeKind, Vec2,
};
use zakuro_core::memory::Memory;
use zakuro_core::room::{
    Connection, CoopEvent, Mac, PartyMonSummary, PlayerPresence, RemotePlayer,
};
use zakuro_gpu::{layout, top_aspect_scale};

use crate::menus::{Action, RoomView};
use crate::settings::{Screens, Settings, TopShape};

const LAN_DISCOVERY_PORT: u16 = 24874;
const LAN_MAGIC: &str = "BLUEORB_LAN_V1";
const PRESENCE_SEND_INTERVAL: Duration = Duration::from_millis(50);
const PARTY_SYNC_INTERVAL: Duration = Duration::from_millis(900);
const CUTSCENE_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(900);
const CUTSCENE_SETTLE_DURATION: Duration = Duration::from_millis(1500);
const SOLO_SCRIPTED_BATTLE_WINDOW: Duration = Duration::from_secs(2);
const WARP_RETRY_COOLDOWN: Duration = Duration::from_secs(3);
const WARP_ACTIVE_TIMEOUT: Duration = Duration::from_secs(20);
const ORAS_BATTLE_MAIN_PROCESS: u32 = 0x08C6_9214;
const ORAS_MOVEMODEL_VTABLE: u32 = 0x005D_F370;
const ORAS_MOVEMODEL_STRIDE: u32 = 0xAB0;

#[derive(Clone, Copy)]
pub struct AvatarPalette {
    pub name: &'static str,
    pub cap: Color32,
    pub jacket: Color32,
    pub accent: Color32,
}

pub const AVATAR_PALETTES: [AvatarPalette; 8] = [
    AvatarPalette {
        name: "Sapphire",
        cap: Color32::from_rgb(38, 128, 240),
        jacket: Color32::from_rgb(28, 92, 188),
        accent: Color32::from_rgb(132, 212, 255),
    },
    AvatarPalette {
        name: "Ruby",
        cap: Color32::from_rgb(228, 58, 72),
        jacket: Color32::from_rgb(176, 42, 58),
        accent: Color32::from_rgb(255, 168, 168),
    },
    AvatarPalette {
        name: "Emerald",
        cap: Color32::from_rgb(34, 186, 118),
        jacket: Color32::from_rgb(24, 134, 86),
        accent: Color32::from_rgb(142, 244, 194),
    },
    AvatarPalette {
        name: "Gold",
        cap: Color32::from_rgb(236, 178, 36),
        jacket: Color32::from_rgb(184, 126, 22),
        accent: Color32::from_rgb(255, 232, 148),
    },
    AvatarPalette {
        name: "Amethyst",
        cap: Color32::from_rgb(148, 86, 236),
        jacket: Color32::from_rgb(104, 58, 178),
        accent: Color32::from_rgb(214, 180, 255),
    },
    AvatarPalette {
        name: "Coral",
        cap: Color32::from_rgb(244, 112, 68),
        jacket: Color32::from_rgb(188, 78, 48),
        accent: Color32::from_rgb(255, 198, 172),
    },
    AvatarPalette {
        name: "Cyan",
        cap: Color32::from_rgb(34, 192, 216),
        jacket: Color32::from_rgb(24, 138, 162),
        accent: Color32::from_rgb(168, 246, 255),
    },
    AvatarPalette {
        name: "Obsidian",
        cap: Color32::from_rgb(74, 84, 104),
        jacket: Color32::from_rgb(48, 54, 68),
        accent: Color32::from_rgb(178, 196, 224),
    },
];

pub fn avatar_palette(idx: u8) -> AvatarPalette {
    AVATAR_PALETTES[(idx as usize) % AVATAR_PALETTES.len()]
}

pub fn is_tutorial_unlocked(local_party_count: u8, force_unlock: bool) -> bool {
    local_party_count >= 1 || force_unlock
}

/// Returns true whenever zone warps / teleports must pause because either the local
/// player is in or settling from a cutscene, or another player is in a cutscene
/// while cutscene sync is enabled.
pub fn cutscene_blocks_sync(
    local_in_cutscene: bool,
    settling: bool,
    any_peer_in_cutscene: bool,
    sync_enabled: bool,
) -> bool {
    local_in_cutscene || settling || (sync_enabled && any_peer_in_cutscene)
}

/// Strictly guards 2v2 Double Battle upgrades so solo/scripted encounters
/// (starter Poochyena, Wally catching tutorial, 1-Pokemon trainers, Sky/Multi
/// battles, and scripted cutscene bosses) remain untouched and bug-free.
#[allow(clippy::too_many_arguments)]
pub fn should_upgrade_battle_to_double(
    healthy_party_count: u8,
    format: u8,
    is_against_trainer: bool,
    opp_count: u8,
    is_sky_battle: bool,
    is_capture_forced: bool,
    no_money: bool,
    has_native_ally_or_multi: bool,
    recent_scripted_event: bool,
) -> bool {
    if healthy_party_count < 2 {
        return false;
    }
    if format != 0 {
        return false;
    }
    if is_sky_battle || is_capture_forced || no_money || has_native_ally_or_multi || recent_scripted_event {
        return false;
    }
    if is_against_trainer && opp_count < 2 {
        return false;
    }
    true
}

#[derive(Debug, Clone)]
pub struct LanRoom {
    pub address: String,
    pub name: String,
    #[allow(dead_code)]
    pub port: u16,
    pub members: usize,
    pub last_seen: Instant,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorldCamera {
    view: [[f32; 4]; 3],
    proj: [[f32; 4]; 4],
    rotated_landscape: bool,
    right_sign: f32,
    up_sign: f32,
}

#[derive(Debug, Clone)]
pub struct SampledWorld {
    pub presence: PlayerPresence,
    pub in_overworld: bool,
    pub in_cutscene: bool,
    pub party_count: u8,
    pub party_mons: Vec<PartyMonSummary>,
    map_mgr: u32,
    player_slot: Option<u32>,
    team_ptr: Option<u32>,
    camera: Option<WorldCamera>,
    active_npcs: Vec<u32>,
}

#[derive(Debug, Clone)]
struct PendingWarp {
    zone_id: u16,
    x: f32,
    y: f32,
    z: f32,
    manual: bool,
    issued_at: Instant,
}

#[derive(Debug, Clone, Copy)]
struct HijackedNpc {
    addr: u32,
    obj_code: u16,
    orig_move_code: u16,
    orig_prev: [f32; 3],
    orig_next: [f32; 3],
    orig_now: [f32; 3],
    orig_draw: [f32; 3],
}

#[derive(Debug, Clone)]
struct SmoothedRemote {
    nickname: String,
    presence: PlayerPresence,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    last_packet: Instant,
}

#[derive(Debug, Clone)]
pub struct RenderPeer {
    pub mac: Mac,
    pub nickname: String,
    pub zone_id: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub speed: f32,
    pub avatar_style: u8,
    pub in_battle: bool,
    pub in_cutscene: bool,
    pub is_leader: bool,
    pub party_count: u8,
}

#[derive(Debug, Clone)]
pub struct CoopFrame {
    pub local_zone: Option<u16>,
    pub local_pos: [f32; 3],
    pub(crate) camera: Option<WorldCamera>,
    pub peers: Vec<RenderPeer>,
    pub aspect_scale: f32,
    pub local_is_leader: bool,
    pub tutorial_unlocked: bool,
    pub local_party_count: u8,
    pub in_overworld: bool,
    pub local_in_battle: bool,
    pub local_in_cutscene: bool,
    pub warp_paused_by_cutscene: bool,
    pub waiting_for_cutscene: Vec<String>,
    pub banner: Option<String>,
    pub local_party: Vec<PartyMonSummary>,
    pub partner_parties: Vec<(String, bool, Vec<PartyMonSummary>)>,
    pub room_connected: bool,
    pub room_name: Option<String>,
    pub room_members: Vec<(String, Mac, bool, bool)>,
    pub native_npc_peer: Option<Mac>,
}

pub struct CoopRuntime {
    pub preview_buddy: bool,
    pub native_3d_npc_sync: bool,
    pub lan_rooms: Vec<LanRoom>,
    pub local_ip: Option<String>,
    pub local_party_count: u8,
    pub local_is_leader: bool,
    pub leader_mac: Option<Mac>,
    pub leader_in_battle: bool,
    pub local_in_cutscene: bool,
    gm_slot_hint: u32,
    models_offset_hint: u32,
    models_base_hint: Option<u32>,
    proj_offset_hint: Option<u32>,
    last_gm_scan: Instant,
    last_model_scan: Instant,
    last_vtable_scan: Instant,
    last_known_zone: Option<u16>,
    hijacked_npc: Option<HijackedNpc>,
    native_npc_peer: Option<Mac>,
    pub last_diag: String,
    last_sample: Option<SampledWorld>,
    last_presence_sent: Instant,
    last_party_sent: Instant,
    last_cutscene_sent: Instant,
    last_cutscene_ended_at: Instant,
    last_tick: Instant,
    smoothed: HashMap<Mac, SmoothedRemote>,
    peer_parties: HashMap<Mac, (u8, Vec<PartyMonSummary>)>,
    peer_cutscenes: HashMap<Mac, (bool, Instant)>,
    last_leader_zone: Option<u16>,
    was_in_battle: bool,
    pending_warp: Option<PendingWarp>,
    warp_paused_by_cutscene: bool,
    last_warp_applied: Instant,
    battle_double_applied: bool,
    status_banner: Option<(String, Instant)>,
    lan_socket: Option<UdpSocket>,
    lan_recv_tried: bool,
    last_lan_beacon: Instant,
    started_at: Instant,
}

impl CoopRuntime {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            preview_buddy: false,
            native_3d_npc_sync: false,
            lan_rooms: Vec::new(),
            local_ip: detect_local_lan_ip(),
            local_party_count: 0,
            local_is_leader: false,
            leader_mac: None,
            leader_in_battle: false,
            local_in_cutscene: false,
            gm_slot_hint: 0x0062_F7C4,
            models_offset_hint: 0x369C,
            models_base_hint: None,
            proj_offset_hint: None,
            last_gm_scan: now - Duration::from_secs(5),
            last_model_scan: now - Duration::from_secs(5),
            last_vtable_scan: now - Duration::from_secs(5),
            last_known_zone: None,
            hijacked_npc: None,
            native_npc_peer: None,
            last_diag: "init".to_owned(),
            last_sample: None,
            last_presence_sent: now - Duration::from_secs(1),
            last_party_sent: now - Duration::from_secs(5),
            last_cutscene_sent: now - Duration::from_secs(5),
            last_cutscene_ended_at: now - Duration::from_secs(10),
            last_tick: now,
            smoothed: HashMap::new(),
            peer_parties: HashMap::new(),
            peer_cutscenes: HashMap::new(),
            last_leader_zone: None,
            was_in_battle: false,
            pending_warp: None,
            warp_paused_by_cutscene: false,
            last_warp_applied: now - Duration::from_secs(10),
            battle_double_applied: false,
            status_banner: None,
            lan_socket: None,
            lan_recv_tried: false,
            last_lan_beacon: now - Duration::from_secs(5),
            started_at: now,
        }
    }

    pub fn reset_game(&mut self) {
        self.last_sample = None;
        self.smoothed.clear();
        self.peer_parties.clear();
        self.peer_cutscenes.clear();
        self.proj_offset_hint = None;
        self.local_party_count = 0;
        self.local_in_cutscene = false;
        self.last_leader_zone = None;
        self.was_in_battle = false;
        self.models_offset_hint = 0x369C;
        self.models_base_hint = None;
        self.last_known_zone = None;
        self.hijacked_npc = None;
        self.native_npc_peer = None;
        self.last_diag = "reset".to_owned();
        self.pending_warp = None;
        self.warp_paused_by_cutscene = false;
        self.battle_double_applied = false;
        self.leader_in_battle = false;
        self.status_banner = None;
    }

    pub fn tutorial_unlocked(&self, settings: &Settings) -> bool {
        is_tutorial_unlocked(self.local_party_count, settings.coop_force_unlock)
    }

    /// Manually queues a teleport to the Chef/Leader (or to the teammate if you are the Chef).
    pub fn request_manual_warp(&mut self) {
        if let Some(l_mac) = self.leader_mac.filter(|_| !self.local_is_leader) {
            self.request_warp_to_peer(l_mac);
            return;
        }
        if let Some((&peer_mac, _)) = self.smoothed.iter().next() {
            self.request_warp_to_peer(peer_mac);
        } else {
            self.set_banner("No teammate connected with position data yet.");
        }
    }

    /// Manually queues a teleport directly to a specific room teammate (`target_mac`).
    pub fn request_warp_to_peer(&mut self, target_mac: Mac) {
        let Some(remote) = self.smoothed.get(&target_mac).cloned() else {
            self.set_banner("Waiting for teammate telemetry… Make sure both games are unpaused.");
            return;
        };
        if !(1..536).contains(&remote.presence.zone_id) {
            self.set_banner(format!(
                "{} is not in Hoenn overworld yet (Zone {}).",
                remote.nickname, remote.presence.zone_id
            ));
            return;
        }
        self.pending_warp = Some(PendingWarp {
            zone_id: remote.presence.zone_id,
            x: remote.presence.x,
            y: remote.presence.y,
            z: remote.presence.z,
            manual: true,
            issued_at: Instant::now(),
        });
        self.set_banner(format!(
            "Teleporting to {} (Zone {})…",
            remote.nickname, remote.presence.zone_id
        ));
    }

    fn set_banner(&mut self, msg: impl Into<String>) {
        self.status_banner = Some((msg.into(), Instant::now()));
    }

    /// Samples the local player's ORAS state, drives Co-op Campaign rules (Chef TP,
    /// Cutscene sync barrier, safe 2v2 Double Battle, real party sync), and broadcasts updates.
    pub fn sample_game(
        &mut self,
        memory: &mut Memory,
        room: Option<&Connection>,
        settings: &Settings,
    ) {
        let sampled = sample_oras_world(
            memory,
            &mut self.gm_slot_hint,
            &mut self.models_offset_hint,
            &mut self.models_base_hint,
            &mut self.proj_offset_hint,
            &mut self.last_gm_scan,
            &mut self.last_model_scan,
            &mut self.last_vtable_scan,
            &mut self.last_known_zone,
            &mut self.last_diag,
            settings.coop_avatar_style,
        );

        if let Some(conn) = room {
            self.leader_mac = conn.leader();
            self.local_is_leader = conn.is_leader();
            for (from, event) in conn.drain_coop() {
                match event {
                    CoopEvent::ZoneWarp { zone_id, x, y, z, .. } => {
                        let from_leader = self.leader_mac == Some(from);
                        if from_leader
                            && !self.local_is_leader
                            && settings.coop_follow_leader
                            && (1..536).contains(&zone_id)
                        {
                            self.pending_warp = Some(PendingWarp {
                                zone_id,
                                x,
                                y,
                                z,
                                manual: false,
                                issued_at: Instant::now(),
                            });
                            if !self.local_in_cutscene {
                                self.set_banner(format!(
                                    "Chef moved to Zone {zone_id} — Syncing beside Chef…"
                                ));
                            }
                        }
                    }
                    CoopEvent::BattleStart { zone_id, is_trainer } => {
                        if self.leader_mac == Some(from) {
                            self.leader_in_battle = true;
                            let kind = if is_trainer { "Trainer" } else { "Wild" };
                            self.set_banner(format!(
                                "Chef entered a {kind} Battle (Zone {zone_id})!"
                            ));
                            if !self.local_is_leader && settings.coop_follow_leader {
                                if let Some(remote) = self.smoothed.get(&from) {
                                    if self
                                        .last_sample
                                        .as_ref()
                                        .is_some_and(|s| s.presence.zone_id != zone_id)
                                        && (1..536).contains(&zone_id)
                                    {
                                        self.pending_warp = Some(PendingWarp {
                                            zone_id,
                                            x: remote.x,
                                            y: remote.y,
                                            z: remote.z,
                                            manual: false,
                                            issued_at: Instant::now(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    CoopEvent::BattleEnd => {
                        if self.leader_mac == Some(from) {
                            self.leader_in_battle = false;
                        }
                    }
                    CoopEvent::CutsceneState(active) => {
                        self.peer_cutscenes.insert(from, (active, Instant::now()));
                    }
                    CoopEvent::PartySync { party_count, mons } => {
                        self.peer_parties.insert(from, (party_count, mons));
                    }
                }
            }
        } else {
            self.leader_mac = None;
            self.local_is_leader = false;
            self.leader_in_battle = false;
            self.pending_warp = None;
            self.peer_cutscenes.clear();
        }

        self.warp_paused_by_cutscene = false;

        if let Some(world) = &sampled {
            self.local_party_count = world.party_count;
            let unlocked = self.tutorial_unlocked(settings);

            // Track local cutscene edges so warp & solo-battle guards know when an event just ended
            let cutscene_changed = self.local_in_cutscene != world.in_cutscene;
            if self.local_in_cutscene && !world.in_cutscene {
                self.last_cutscene_ended_at = Instant::now();
            }
            self.local_in_cutscene = world.in_cutscene;

            let settling =
                !world.in_cutscene && self.last_cutscene_ended_at.elapsed() < CUTSCENE_SETTLE_DURATION;
            let any_peer_cutscene = self
                .peer_cutscenes
                .values()
                .any(|(active, at)| *active && at.elapsed() < Duration::from_secs(6));
            let sync_blocked = cutscene_blocks_sync(
                world.in_cutscene,
                settling,
                any_peer_cutscene,
                settings.coop_sync_cutscenes,
            );

            if let Some(conn) = room {
                if self.last_presence_sent.elapsed() >= PRESENCE_SEND_INTERVAL {
                    conn.send_presence(world.presence.clone());
                    self.last_presence_sent = Instant::now();
                }

                if self.last_party_sent.elapsed() >= PARTY_SYNC_INTERVAL {
                    conn.send_coop(CoopEvent::PartySync {
                        party_count: world.party_count,
                        mons: world.party_mons.clone(),
                    });
                    self.last_party_sent = Instant::now();
                }

                if cutscene_changed || self.last_cutscene_sent.elapsed() >= CUTSCENE_HEARTBEAT_INTERVAL {
                    conn.send_coop(CoopEvent::CutsceneState(world.in_cutscene));
                    self.last_cutscene_sent = Instant::now();
                }

                if unlocked && self.local_is_leader {
                    if !self.was_in_battle && world.presence.in_battle {
                        conn.send_coop(CoopEvent::BattleStart {
                            zone_id: world.presence.zone_id,
                            is_trainer: false,
                        });
                    } else if self.was_in_battle && !world.presence.in_battle {
                        conn.send_coop(CoopEvent::BattleEnd);
                    }

                    // Only broadcast Chef zone changes when no cutscene is active or settling
                    if !sync_blocked && world.in_overworld && (1..536).contains(&world.presence.zone_id) {
                        if self.last_leader_zone != Some(world.presence.zone_id) {
                            if self.last_leader_zone.is_some() {
                                conn.send_coop(CoopEvent::ZoneWarp {
                                    zone_id: world.presence.zone_id,
                                    x: world.presence.x,
                                    y: world.presence.y,
                                    z: world.presence.z,
                                    yaw: world.presence.yaw,
                                });
                            }
                            self.last_leader_zone = Some(world.presence.zone_id);
                        }
                    }
                } else {
                    self.last_leader_zone = None;
                }
            }

            // Execute Auto-TP when unlocked & cutscenes clear; manual TP executes whenever local player is free in overworld
            if world.in_overworld {
                if let Some(mut warp) = self.pending_warp.clone() {
                    let allowed = warp.manual || unlocked;
                    let blocked = if warp.manual {
                        world.in_cutscene
                    } else {
                        sync_blocked
                    };
                    if !allowed {
                        self.pending_warp = None;
                        self.set_banner("Tutorial lock active — pick your starter or enable Override in F4.");
                    } else if blocked {
                        self.warp_paused_by_cutscene = true;
                        // Keep warp fresh until cutscenes complete
                        warp.issued_at = Instant::now();
                        self.pending_warp = Some(warp);
                    } else if warp.issued_at.elapsed() > WARP_ACTIVE_TIMEOUT {
                        self.pending_warp = None;
                    } else if world.presence.zone_id != warp.zone_id {
                        if self.last_warp_applied.elapsed() >= WARP_RETRY_COOLDOWN
                            && trigger_map_warp(memory, world.map_mgr, warp.zone_id)
                        {
                            self.last_warp_applied = Instant::now();
                        }
                    } else if let Some(player_slot) = world.player_slot {
                        if is_next_map_idle(memory, world.map_mgr) {
                            teleport_player_coords(memory, player_slot, [warp.x + 18.0, warp.y, warp.z]);
                            self.pending_warp = None;
                            self.set_banner(format!("Teleported to Zone {}!", warp.zone_id));
                        }
                    } else if warp.manual {
                        self.pending_warp = None;
                        self.set_banner("Already in the same zone — waiting for local player model telemetry.");
                    }
                }
            }

            // Battle transition & Solo/Scripted-Battle-Safe 2v2 Double Battle upgrade
            if !world.presence.in_battle {
                self.battle_double_applied = false;
            } else if unlocked
                && settings.coop_double_battle
                && room.is_some()
                && !self.battle_double_applied
            {
                if let Some(team_ptr) = world.team_ptr {
                    let healthy_count = world.party_mons.iter().filter(|m| m.hp > 0).count() as u8;
                    let recent_scripted = world.in_cutscene
                        || self.last_cutscene_ended_at.elapsed() < SOLO_SCRIPTED_BATTLE_WINDOW;
                    if try_upgrade_battle_to_double(memory, team_ptr, healthy_count, recent_scripted) {
                        self.battle_double_applied = true;
                        self.set_banner("Co-op Double Battle (2v2) Activated — Real Save Kept!");
                    }
                }
            }

            self.was_in_battle = world.presence.in_battle;

            // Optional real 3D in-game MoveModel NPC sync for Player 2
            let synced_peer = if self.native_3d_npc_sync && world.in_overworld && !world.in_cutscene && !world.active_npcs.is_empty() {
                self.smoothed
                    .iter()
                    .find(|(_, r)| r.presence.zone_id == world.presence.zone_id || r.presence.zone_id == 0)
                    .map(|(mac, r)| (*mac, [r.x, r.y, r.z], r.yaw))
                    .or_else(|| {
                        self.preview_buddy.then(|| {
                            let angle = self.started_at.elapsed().as_secs_f32() * 1.15;
                            let pos = [
                                world.presence.x + angle.cos() * 26.0,
                                world.presence.y,
                                world.presence.z + angle.sin() * 26.0,
                            ];
                            let yaw = (-angle.sin()).atan2(angle.cos());
                            ([0x02, 0x42, 0x4C, 0x55, 0x45, 0x01], pos, yaw)
                        })
                    })
            } else {
                None
            };
            if let Some((peer_mac, pos, yaw)) = synced_peer {
                let target_npc = self
                    .hijacked_npc
                    .map(|h| h.addr)
                    .filter(|addr| world.active_npcs.contains(addr))
                    .or_else(|| world.active_npcs.first().copied());
                if let Some(npc_addr) = target_npc {
                    if self.hijacked_npc.is_some_and(|h| h.addr != npc_addr) {
                        restore_hijacked_npc(memory, &mut self.hijacked_npc);
                    }
                    if self.hijacked_npc.is_none() {
                        if let (Some(obj_code), Some(orig_move_code), Some(orig_prev), Some(orig_next), Some(orig_now), Some(orig_draw)) = (
                            read_u16(memory, npc_addr.wrapping_add(0x0E)),
                            read_u16(memory, npc_addr.wrapping_add(0x10)),
                            read_vec3(memory, npc_addr.wrapping_add(0x94)),
                            read_vec3(memory, npc_addr.wrapping_add(0xC0)),
                            read_vec3(memory, npc_addr.wrapping_add(0xEC)),
                            read_vec3(memory, npc_addr.wrapping_add(0x114)),
                        ) {
                            self.hijacked_npc = Some(HijackedNpc {
                                addr: npc_addr,
                                obj_code,
                                orig_move_code,
                                orig_prev,
                                orig_next,
                                orig_now,
                                orig_draw,
                            });
                        }
                    }
                    write_u16_if_writable(memory, npc_addr.wrapping_add(0x10), 0);
                    set_model_all_positions(memory, npc_addr, pos);
                    write_model_facing(memory, npc_addr, yaw);
                    self.native_npc_peer = Some(peer_mac);
                } else {
                    restore_hijacked_npc(memory, &mut self.hijacked_npc);
                    self.native_npc_peer = None;
                }
            } else {
                restore_hijacked_npc(memory, &mut self.hijacked_npc);
                self.native_npc_peer = None;
            }
        } else {
            restore_hijacked_npc(memory, &mut self.hijacked_npc);
            self.native_npc_peer = None;
            if let (Some(conn), true) = (room, self.last_sample.is_none()) {
                if self.last_presence_sent.elapsed() >= Duration::from_millis(500) {
                    conn.send_presence(PlayerPresence {
                        zone_id: 0,
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        yaw: 0.0,
                        speed: 0.0,
                        avatar_style: settings.coop_avatar_style,
                        in_battle: false,
                    });
                    self.last_presence_sent = Instant::now();
                }
            }
        }

        if let Some((_, at)) = &self.status_banner {
            if at.elapsed() > Duration::from_secs(6) {
                self.status_banner = None;
            }
        }

        if sampled.is_some() {
            self.last_sample = sampled;
        }
    }

    /// Advances smooth peer interpolation and UDP LAN discovery/beacon traffic.
    pub fn tick(
        &mut self,
        room: Option<&Connection>,
        hosting_name: Option<(&str, u16, usize)>,
        discovery_active: bool,
    ) {
        let now = Instant::now();
        let dt = (now - self.last_tick).as_secs_f32().clamp(0.001, 0.05);
        self.last_tick = now;

        if let Some((room_name, tcp_port, members)) = hosting_name {
            if self.last_lan_beacon.elapsed() >= Duration::from_millis(1400) {
                self.last_lan_beacon = now;
                send_lan_beacon(room_name, tcp_port, members);
            }
        }

        if discovery_active {
            self.poll_lan_discovery();
        }

        if let Some(conn) = room {
            self.leader_mac = conn.leader();
            self.local_is_leader = conn.is_leader();

            // Keep presence alive while paused or on title screen so partners stay visible in HUD & room state
            if self.last_presence_sent.elapsed() >= Duration::from_millis(800) {
                let fallback_presence = self
                    .last_sample
                    .as_ref()
                    .map(|s| s.presence.clone())
                    .unwrap_or(PlayerPresence {
                        zone_id: 0,
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        yaw: 0.0,
                        speed: 0.0,
                        avatar_style: 0,
                        in_battle: false,
                    });
                conn.send_presence(fallback_presence);
                self.last_presence_sent = now;
            }

            let remotes = conn.remote_players();
            let mut active_macs = Vec::with_capacity(remotes.len());
            let alpha = 1.0 - (-dt * 14.0).exp();
            for RemotePlayer { mac, nickname, presence } in remotes {
                active_macs.push(mac);
                match self.smoothed.get_mut(&mac) {
                    Some(entry) => {
                        let dx = presence.x - entry.x;
                        let dy = presence.y - entry.y;
                        let dz = presence.z - entry.z;
                        let dist_sq = dx * dx + dy * dy + dz * dz;
                        let zone_changed = entry.presence.zone_id != presence.zone_id;
                        if zone_changed || dist_sq > 120.0 * 120.0 {
                            entry.x = presence.x;
                            entry.y = presence.y;
                            entry.z = presence.z;
                            entry.yaw = presence.yaw;
                        } else {
                            entry.x += dx * alpha;
                            entry.y += dy * alpha;
                            entry.z += dz * alpha;
                            let diff = wrap_angle(presence.yaw - entry.yaw);
                            entry.yaw = wrap_angle(entry.yaw + diff * alpha);
                        }
                        entry.nickname = nickname;
                        entry.presence = presence;
                        entry.last_packet = now;
                    }
                    None => {
                        self.smoothed.insert(
                            mac,
                            SmoothedRemote {
                                nickname,
                                x: presence.x,
                                y: presence.y,
                                z: presence.z,
                                yaw: presence.yaw,
                                presence,
                                last_packet: now,
                            },
                        );
                    }
                }
            }
            self.smoothed
                .retain(|mac, entry| active_macs.contains(mac) && entry.last_packet.elapsed() < Duration::from_secs(6));
            self.peer_parties.retain(|mac, _| active_macs.contains(mac));
            self.peer_cutscenes.retain(|mac, (_, at)| {
                active_macs.contains(mac) && at.elapsed() < Duration::from_secs(6)
            });
        } else {
            self.smoothed.clear();
            self.peer_parties.clear();
            self.peer_cutscenes.clear();
            self.leader_mac = None;
            self.local_is_leader = false;
        }
    }

    pub fn build_frame(&self, settings: &Settings, room: Option<&Connection>) -> CoopFrame {
        let mut waiting_for_cutscene = Vec::new();
        let mut peers: Vec<RenderPeer> = self
            .smoothed
            .iter()
            .map(|(mac, s)| {
                let p_count = self.peer_parties.get(mac).map(|(c, _)| *c).unwrap_or(0);
                let in_cutscene = self
                    .peer_cutscenes
                    .get(mac)
                    .is_some_and(|(active, at)| *active && at.elapsed() < Duration::from_secs(6));
                if settings.coop_sync_cutscenes && in_cutscene {
                    waiting_for_cutscene.push(s.nickname.clone());
                }
                RenderPeer {
                    mac: *mac,
                    nickname: s.nickname.clone(),
                    zone_id: s.presence.zone_id,
                    x: s.x,
                    y: s.y,
                    z: s.z,
                    yaw: s.yaw,
                    speed: s.presence.speed,
                    avatar_style: s.presence.avatar_style,
                    in_battle: s.presence.in_battle,
                    in_cutscene,
                    is_leader: self.leader_mac == Some(*mac),
                    party_count: p_count,
                }
            })
            .collect();

        let (local_zone, local_pos, camera, in_overworld, local_in_cutscene, local_party) =
            match &self.last_sample {
                Some(s) => (
                    Some(s.presence.zone_id),
                    [s.presence.x, s.presence.y, s.presence.z],
                    if s.in_overworld { s.camera } else { None },
                    s.in_overworld,
                    s.in_cutscene,
                    s.party_mons.clone(),
                ),
                None => (None, [0.0, 0.0, 0.0], None, true, false, Vec::new()),
            };

        if self.preview_buddy {
            let t = self.started_at.elapsed().as_secs_f32();
            let angle = t * 1.15;
            let radius = 26.0;
            let bx = local_pos[0] + angle.cos() * radius;
            let by = local_pos[1];
            let bz = local_pos[2] + angle.sin() * radius;
            let tangent_yaw = (-angle.sin()).atan2(angle.cos());
            peers.push(RenderPeer {
                mac: [0x02, 0x42, 0x4C, 0x55, 0x45, 0x01],
                nickname: "Sapphire Bot".to_owned(),
                zone_id: local_zone.unwrap_or(0),
                x: bx,
                y: by,
                z: bz,
                yaw: tangent_yaw,
                speed: 1.0,
                avatar_style: 2,
                in_battle: false,
                in_cutscene: false,
                is_leader: false,
                party_count: 2,
            });
        }

        let partner_parties = self
            .smoothed
            .iter()
            .filter_map(|(mac, s)| {
                let (_, mons) = self.peer_parties.get(mac)?;
                if mons.is_empty() {
                    return None;
                }
                Some((s.nickname.clone(), self.leader_mac == Some(*mac), mons.clone()))
            })
            .collect();

        let (room_connected, room_name, room_members) = match room {
            Some(conn) => {
                let local_mac = conn.mac();
                let leader = conn.leader();
                let list = conn
                    .members()
                    .into_iter()
                    .map(|m| (m.nickname, m.mac, leader == Some(m.mac), m.mac == local_mac))
                    .collect();
                (true, Some(conn.room().to_owned()), list)
            }
            None => (false, None, Vec::new()),
        };

        let local_in_battle = self
            .last_sample
            .as_ref()
            .is_some_and(|s| s.presence.in_battle);

        CoopFrame {
            local_zone,
            local_pos,
            camera,
            peers,
            aspect_scale: top_aspect_scale(),
            local_is_leader: self.local_is_leader,
            tutorial_unlocked: self.tutorial_unlocked(settings),
            local_party_count: self.local_party_count,
            in_overworld,
            local_in_battle,
            local_in_cutscene,
            warp_paused_by_cutscene: self.warp_paused_by_cutscene,
            waiting_for_cutscene,
            banner: self.status_banner.as_ref().map(|(s, _)| s.clone()),
            local_party,
            partner_parties,
            room_connected,
            room_name,
            room_members,
            native_npc_peer: self.native_npc_peer,
        }
    }

    fn poll_lan_discovery(&mut self) {
        if self.lan_socket.is_none() && !self.lan_recv_tried {
            self.lan_recv_tried = true;
            if let Ok(sock) = UdpSocket::bind(("0.0.0.0", LAN_DISCOVERY_PORT)) {
                let _ = sock.set_nonblocking(true);
                let _ = sock.set_broadcast(true);
                self.lan_socket = Some(sock);
            }
        }
        if let Some(sock) = &self.lan_socket {
            let mut buf = [0u8; 512];
            while let Ok((len, src)) = sock.recv_from(&mut buf) {
                let Ok(text) = std::str::from_utf8(&buf[..len]) else { continue };
                if let Some((port, members, name)) = decode_lan_beacon(text) {
                    let ip = src.ip();
                    if is_self_ip(ip, self.local_ip.as_deref()) {
                        continue;
                    }
                    let address = format!("{ip}:{port}");
                    let now = Instant::now();
                    if let Some(existing) = self.lan_rooms.iter_mut().find(|r| r.address == address) {
                        existing.name = name;
                        existing.members = members;
                        existing.last_seen = now;
                    } else {
                        self.lan_rooms.push(LanRoom {
                            address,
                            name,
                            port,
                            members,
                            last_seen: now,
                        });
                    }
                }
            }
        }
        self.lan_rooms.retain(|r| r.last_seen.elapsed() < Duration::from_secs(5));
    }
}

fn is_self_ip(ip: IpAddr, local_ip: Option<&str>) -> bool {
    if ip.is_loopback() {
        return true;
    }
    local_ip.is_some_and(|local| ip.to_string() == local)
}

pub fn encode_lan_beacon(name: &str, tcp_port: u16, members: usize) -> String {
    let clean: String = name
        .chars()
        .filter(|&c| c != '|' && c != '\n' && c != '\r')
        .take(32)
        .collect();
    let label = if clean.trim().is_empty() { "BlueOrb Room".to_owned() } else { clean };
    format!("{LAN_MAGIC}|{tcp_port}|{members}|{label}")
}

pub fn decode_lan_beacon(raw: &str) -> Option<(u16, usize, String)> {
    let mut parts = raw.trim().splitn(4, '|');
    if parts.next()? != LAN_MAGIC {
        return None;
    }
    let port: u16 = parts.next()?.parse().ok()?;
    let members: usize = parts.next()?.parse().ok()?;
    let name = parts.next()?.trim();
    if name.is_empty() {
        return None;
    }
    Some((port, members, name.to_owned()))
}

fn send_lan_beacon(name: &str, tcp_port: u16, members: usize) {
    let payload = encode_lan_beacon(name, tcp_port, members);
    if let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        let _ = sock.set_broadcast(true);
        let target = SocketAddr::from((Ipv4Addr::BROADCAST, LAN_DISCOVERY_PORT));
        let _ = sock.send_to(payload.as_bytes(), target);
    }
}

fn detect_local_lan_ip() -> Option<String> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip.to_string())
    }
}

pub fn top_screen_rect_points(
    window: &winit::window::Window,
    settings: &Settings,
    active_layout: Screens,
    top_shape: TopShape,
) -> Option<Rect> {
    let size = window.inner_size();
    let scale = (window.scale_factor() as f32).max(0.5);
    let (top, _) = layout(
        size.width,
        size.height,
        active_layout.screens(),
        settings.orientation.screen_orientation(),
        top_shape.width(),
        settings.integer_scale,
    );
    top.map(|vp| {
        Rect::from_min_size(
            Pos2::new(vp.x / scale, vp.y / scale),
            Vec2::new(vp.width / scale, vp.height / scale),
        )
    })
}

/// Renders other players in the 3D overworld, Chef crown tags, Co-op Campaign HUD,
/// synchronized Double Battle ally squad status, and cutscene wait notices.
pub fn render_overworld_coop(ctx: &egui::Context, top_rect: Rect, frame: &CoopFrame, settings: &Settings) {
    if !frame.room_connected
        && frame.peers.is_empty()
        && frame.local_zone.is_none()
        && frame.banner.is_none()
        && frame.waiting_for_cutscene.is_empty()
    {
        return;
    }
    if frame.room_connected
        || !frame.peers.is_empty()
        || frame.banner.is_some()
        || !frame.waiting_for_cutscene.is_empty()
        || frame.warp_paused_by_cutscene
    {
        ctx.request_repaint();
    }

    let time_s = ctx.input(|i| i.time) as f32;
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::new("blueorb_coop_overworld_layer"),
    ));
    let clipped = painter.with_clip_rect(top_rect);

    if let (Some(local_zone), Some(cam)) = (frame.local_zone, frame.camera) {
        let mut draw_list: Vec<(f32, &RenderPeer, Pos2, Option<Pos2>)> = Vec::new();
        let mut edge_list: Vec<(&RenderPeer, Vec2, f32)> = Vec::new();

        for peer in &frame.peers {
            if peer.zone_id != local_zone {
                continue;
            }
            let dist_tiles = ((peer.x - frame.local_pos[0]).hypot(peer.z - frame.local_pos[2])) / 18.0;
            if let Some((feet_px, cam_depth)) = project_point(&cam, [peer.x, peer.y, peer.z], top_rect, frame.aspect_scale) {
                let inside = top_rect.expand(-12.0).contains(feet_px);
                if inside {
                    let head_px = project_point(&cam, [peer.x, peer.y + 18.0, peer.z], top_rect, frame.aspect_scale)
                        .map(|(p, _)| p);
                    draw_list.push((cam_depth, peer, feet_px, head_px));
                } else {
                    edge_list.push((peer, feet_px - top_rect.center(), dist_tiles));
                }
            } else {
                let rel_world = world_to_camera(&cam.view, [peer.x, peer.y, peer.z]);
                edge_list.push((peer, Vec2::new(rel_world[0], -rel_world[1] + 24.0), dist_tiles));
            }
        }

        draw_list.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        for (cam_depth, peer, feet, head) in draw_list {
            let h_px = head
                .map(|hp| (feet.y - hp.y).abs())
                .unwrap_or_else(|| (top_rect.height() * 2.2 / (-cam_depth).max(20.0)).clamp(20.0, 96.0))
                .clamp(22.0, (top_rect.height() * 0.34).max(24.0));
            if frame.native_npc_peer == Some(peer.mac) {
                if settings.coop_show_names {
                    let head_top = head.unwrap_or_else(|| Pos2::new(feet.x, feet.y - h_px));
                    paint_nameplate(&clipped, Pos2::new(head_top.x, head_top.y - 10.0), peer);
                }
                continue;
            }
            let cam_yaw = cam.view[2][0].atan2(cam.view[2][2]);
            let rel_yaw = wrap_angle(peer.yaw - cam_yaw);
            paint_world_avatar(
                &clipped,
                feet,
                h_px,
                peer,
                rel_yaw,
                time_s,
                settings.coop_show_names,
            );
        }

        if settings.coop_show_names {
            for (peer, dir_vec, dist_tiles) in edge_list {
                paint_edge_indicator(&clipped, top_rect, peer, dir_vec, dist_tiles);
            }
        }
    }

    // Test bot fallback: when the game camera cannot be resolved yet, only draw the local Sapphire Bot
    // so real teammates never get stuck as a static screen-space placeholder.
    if frame.camera.is_none() || frame.local_zone.is_none() {
        let c = top_rect.center();
        let h_px = (top_rect.height() * 0.2).clamp(24.0, 60.0);
        for peer in frame.peers.iter().filter(|p| p.mac[..5] == [0x02, 0x42, 0x4C, 0x55, 0x45]) {
            let ang = time_s * 1.15;
            let feet = Pos2::new(
                c.x + ang.cos() * top_rect.width() * 0.22,
                c.y + top_rect.height() * 0.18 + ang.sin() * top_rect.height() * 0.06,
            );
            paint_world_avatar(&clipped, feet, h_px, peer, 0.0, time_s, settings.coop_show_names);
        }
    }

    // Cutscene wait card when local player finished first and partners are still in a cutscene
    if !frame.local_in_cutscene && !frame.waiting_for_cutscene.is_empty() {
        let names = frame.waiting_for_cutscene.join(", ");
        let dots = match (time_s * 2.0) as usize % 3 {
            0 => ".",
            1 => "..",
            _ => "...",
        };
        let bw = (top_rect.width() * 0.68).clamp(240.0, 440.0);
        let bpos = Pos2::new(top_rect.center().x - bw * 0.5, top_rect.top() + 14.0);
        egui::Area::new(egui::Id::new("blueorb_coop_cutscene_wait"))
            .order(egui::Order::Foreground)
            .fixed_pos(bpos)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(14, 22, 42, 232))
                    .stroke(Stroke::new(1.2, Color32::from_rgb(250, 204, 96)))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 9))
                    .show(ui, |ui| {
                        ui.set_width(bw - 28.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(format!("Waiting for {names} to finish cutscene{dots}"))
                                    .size(12.5)
                                    .strong()
                                    .color(Color32::from_rgb(255, 228, 138)),
                            );
                            ui.label(
                                RichText::new("Please wait — Co-op sync resumes automatically once both players are ready.")
                                    .size(10.5)
                                    .color(Color32::from_rgb(212, 226, 246)),
                            );
                        });
                    });
            });
    } else if frame.local_in_cutscene && frame.warp_paused_by_cutscene {
        let bw = (top_rect.width() * 0.62).clamp(210.0, 390.0);
        let bpos = Pos2::new(top_rect.center().x - bw * 0.5, top_rect.top() + 10.0);
        egui::Area::new(egui::Id::new("blueorb_coop_cutscene_local"))
            .order(egui::Order::Foreground)
            .fixed_pos(bpos)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(12, 22, 40, 214))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(108, 192, 255)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.set_width(bw - 20.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                RichText::new("Cutscene active — Teleport to Chef paused until cutscene ends")
                                    .size(10.8)
                                    .color(Color32::from_rgb(220, 238, 255)),
                            );
                        });
                    });
            });
    } else if let Some(banner_text) = &frame.banner {
        let bw = (top_rect.width() * 0.72).clamp(200.0, 420.0);
        let bpos = Pos2::new(top_rect.center().x - bw * 0.5, top_rect.top() + 10.0);
        egui::Area::new(egui::Id::new("blueorb_coop_banner"))
            .order(egui::Order::Foreground)
            .fixed_pos(bpos)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(12, 24, 46, 224))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(108, 192, 255)))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.set_width(bw - 24.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(RichText::new(banner_text).size(11.5).strong().color(Color32::from_rgb(232, 244, 255)));
                        });
                    });
            });
    }

    if settings.coop_show_hud && (frame.room_connected || !frame.peers.is_empty()) {
        let hud_w = 226.0_f32.min((top_rect.width() * 0.44).max(176.0));
        let hud_pos = Pos2::new(top_rect.right() - hud_w - 10.0, top_rect.top() + 10.0);
        let remote_members: Vec<_> = frame.room_members.iter().filter(|m| !m.3).collect();
        let has_teammate = !frame.peers.is_empty() || !remote_members.is_empty();
        egui::Area::new(egui::Id::new("blueorb_coop_hud"))
            .order(egui::Order::Foreground)
            .fixed_pos(hud_pos)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(10, 18, 34, 212))
                    .stroke(Stroke::new(
                        1.0,
                        if has_teammate {
                            Color32::from_rgba_unmultiplied(100, 228, 166, 128)
                        } else {
                            Color32::from_rgba_unmultiplied(245, 192, 92, 118)
                        },
                    ))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.set_width(hud_w - 20.0);
                        ui.horizontal(|ui| {
                            let (dot_r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                            ui.painter().circle_filled(
                                dot_r.center(),
                                3.5,
                                if has_teammate {
                                    Color32::from_rgb(64, 224, 148)
                                } else {
                                    Color32::from_rgb(245, 196, 86)
                                },
                            );
                            let total_count = frame.room_members.len().max(frame.peers.len() + usize::from(frame.room_connected));
                            let title = if frame.local_in_battle {
                                "Co-op Battle".to_owned()
                            } else if !has_teammate {
                                "Room Open (1/1 — Waiting)".to_owned()
                            } else if frame.local_is_leader {
                                format!("Co-op Chef ({total_count} online)")
                            } else {
                                format!("Co-op Linked ({total_count} online)")
                            };
                            ui.label(
                                RichText::new(title)
                                    .size(11.5)
                                    .strong()
                                    .color(Color32::from_rgb(232, 242, 255)),
                            );
                        });
                        if let Some(lz) = frame.local_zone {
                            if lz > 0 {
                                ui.label(
                                    RichText::new(format!("You: Zone {lz}"))
                                        .size(9.8)
                                        .monospace()
                                        .color(Color32::from_rgb(144, 178, 218)),
                                );
                            }
                        }
                        if !has_teammate {
                            ui.label(
                                RichText::new(if cfg!(target_os = "macos") {
                                    "Press ⌘4 to share IP or join mate"
                                } else {
                                    "Press F4 to share IP or join mate"
                                })
                                .size(10.0)
                                .color(Color32::from_rgb(196, 208, 228)),
                            );
                        }
                        if !frame.tutorial_unlocked {
                            ui.label(
                                RichText::new("Tutorial lock — pick starter or override")
                                    .size(10.0)
                                    .color(Color32::from_rgb(252, 204, 108)),
                            );
                        }
                        ui.add_space(3.0);
                        if !frame.peers.is_empty() {
                            for peer in frame.peers.iter().take(4) {
                                let pal = avatar_palette(peer.avatar_style);
                                let same_zone = frame.local_zone.is_some_and(|lz| lz > 0 && lz == peer.zone_id);
                                ui.horizontal(|ui| {
                                    let (sw, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                                    ui.painter().circle_filled(sw.center(), 3.5, pal.cap);
                                    let display = if peer.is_leader {
                                        format!("[Chef] {}", peer.nickname)
                                    } else {
                                        peer.nickname.clone()
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(display)
                                                .size(11.0)
                                                .color(if peer.is_leader {
                                                    Color32::from_rgb(255, 228, 138)
                                                } else {
                                                    Color32::from_rgb(220, 230, 246)
                                                }),
                                        )
                                        .truncate(),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let p_tag = if peer.party_count > 0 {
                                            format!(" · {}P", peer.party_count)
                                        } else {
                                            String::new()
                                        };
                                        let (tag, col) = if peer.in_cutscene {
                                            (format!("Cutscene{p_tag}"), Color32::from_rgb(255, 214, 102))
                                        } else if peer.in_battle {
                                            (format!("Battle{p_tag}"), Color32::from_rgb(252, 168, 112))
                                        } else if peer.zone_id == 0 {
                                            ("Title/Menu".to_owned(), Color32::from_rgb(176, 188, 210))
                                        } else if same_zone {
                                            (format!("Same Zone{p_tag}"), Color32::from_rgb(122, 238, 174))
                                        } else {
                                            (format!("Z{}{p_tag}", peer.zone_id), Color32::from_rgb(142, 184, 234))
                                        };
                                        ui.label(RichText::new(tag).size(10.0).monospace().color(col));
                                    });
                                });
                            }
                        } else {
                            for (nick, _, is_chef, _) in remote_members.into_iter().take(4) {
                                let label = if *is_chef {
                                    format!("[Chef] {nick}")
                                } else {
                                    nick.clone()
                                };
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(label).size(11.0).color(Color32::from_rgb(220, 230, 246)));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(RichText::new("Connected").size(10.0).monospace().color(Color32::from_rgb(122, 238, 174)));
                                    });
                                });
                            }
                        }
                        if frame.local_in_battle && (!frame.local_party.is_empty() || !frame.partner_parties.is_empty()) {
                            ui.separator();
                            if !frame.local_party.is_empty() {
                                let alive = frame.local_party.iter().filter(|m| m.hp > 0).count();
                                let top_lv = frame.local_party.first().map(|m| m.level).unwrap_or(1);
                                ui.label(
                                    RichText::new(format!("You: {alive}/{} ok (Lv.{top_lv})", frame.local_party_count))
                                        .size(10.0)
                                        .color(Color32::from_rgb(142, 244, 194)),
                                );
                            }
                            for (pname, is_chef, mons) in frame.partner_parties.iter().take(2) {
                                let prefix = if *is_chef { "[Chef] " } else { "" };
                                let alive = mons.iter().filter(|m| m.hp > 0).count();
                                let top_lv = mons.first().map(|m| m.level).unwrap_or(1);
                                ui.label(
                                    RichText::new(format!("{prefix}{pname}: {alive}/{} ok (Lv.{top_lv})", mons.len()))
                                        .size(10.0)
                                        .color(Color32::from_rgb(174, 222, 255)),
                                );
                            }
                        }
                    });
            });
    }
}

fn paint_world_avatar(
    painter: &egui::Painter,
    feet: Pos2,
    h: f32,
    peer: &RenderPeer,
    rel_yaw: f32,
    time_s: f32,
    show_name: bool,
) {
    let pal = avatar_palette(peer.avatar_style);
    let moving = peer.speed > 0.08;
    let phase = time_s * 10.5 + (peer.mac[5] as f32);
    let stride = if moving { phase.sin() * h * 0.09 } else { 0.0 };
    let bob = if moving { (phase * 2.0).sin().abs() * h * 0.04 } else { (time_s * 2.5).sin() * h * 0.01 };

    let w = h * 0.56;
    let body_feet = Pos2::new(feet.x, feet.y - bob);

    let shadow_rect = Rect::from_center_size(feet, Vec2::new(w * 0.98, h * 0.22));
    painter.rect_filled(
        shadow_rect,
        CornerRadius::same((h * 0.11) as u8),
        Color32::from_rgba_unmultiplied(4, 10, 22, 128),
    );
    let ring_col = if peer.is_leader {
        Color32::from_rgba_unmultiplied(255, 214, 96, 185)
    } else {
        Color32::from_rgba_unmultiplied(pal.accent.r(), pal.accent.g(), pal.accent.b(), 145)
    };
    painter.rect_stroke(
        shadow_rect.expand(2.0),
        CornerRadius::same((h * 0.13) as u8),
        Stroke::new(if peer.is_leader { 1.6 } else { 1.2 }, ring_col),
        StrokeKind::Outside,
    );

    let leg_w = (w * 0.24).max(2.5);
    let leg_h = (h * 0.26).max(4.0);
    let left_leg = Rect::from_min_size(
        Pos2::new(body_feet.x - w * 0.24 - leg_w * 0.5, body_feet.y - leg_h + stride),
        Vec2::new(leg_w, leg_h),
    );
    let right_leg = Rect::from_min_size(
        Pos2::new(body_feet.x + w * 0.24 - leg_w * 0.5, body_feet.y - leg_h - stride),
        Vec2::new(leg_w, leg_h),
    );
    let leg_col = Color32::from_rgb(38, 46, 62);
    painter.rect_filled(left_leg, CornerRadius::same(2), leg_col);
    painter.rect_filled(right_leg, CornerRadius::same(2), leg_col);
    painter.rect_filled(
        Rect::from_min_size(Pos2::new(left_leg.left() - 0.5, left_leg.bottom() - 3.0), Vec2::new(leg_w + 1.0, 3.0)),
        CornerRadius::same(1),
        pal.cap,
    );
    painter.rect_filled(
        Rect::from_min_size(Pos2::new(right_leg.left() - 0.5, right_leg.bottom() - 3.0), Vec2::new(leg_w + 1.0, 3.0)),
        CornerRadius::same(1),
        pal.cap,
    );

    let torso_h = h * 0.36;
    let torso_w = w * 0.78;
    let torso_rect = Rect::from_center_size(
        Pos2::new(body_feet.x, body_feet.y - leg_h - torso_h * 0.48),
        Vec2::new(torso_w, torso_h),
    );
    let outline = Color32::from_rgb(16, 22, 34);
    painter.rect_filled(torso_rect, CornerRadius::same(4), pal.jacket);
    let stripe = Rect::from_min_size(
        Pos2::new(torso_rect.center().x - torso_w * 0.14, torso_rect.top() + 2.0),
        Vec2::new(torso_w * 0.28, torso_h - 3.0),
    );
    painter.rect_filled(stripe, CornerRadius::same(2), Color32::from_rgb(236, 242, 250));
    painter.rect_stroke(torso_rect, CornerRadius::same(4), Stroke::new(1.0, outline), StrokeKind::Outside);

    let head_r = (h * 0.22).max(5.0);
    let head_c = Pos2::new(body_feet.x, torso_rect.top() - head_r * 0.78);
    painter.circle_filled(head_c, head_r, Color32::from_rgb(248, 214, 184));
    painter.circle_stroke(head_c, head_r, Stroke::new(1.0, outline));

    let cap_clip = Rect::from_min_max(
        Pos2::new(head_c.x - head_r - 2.0, head_c.y - head_r - 2.0),
        Pos2::new(head_c.x + head_r + 2.0, head_c.y + head_r * 0.08),
    );
    painter.with_clip_rect(cap_clip).circle_filled(head_c, head_r * 1.06, pal.cap);
    let look_x = rel_yaw.sin() * head_r * 0.42;
    painter.circle_filled(
        Pos2::new(head_c.x + look_x * 0.65, head_c.y - head_r * 0.42),
        (head_r * 0.28).max(2.0),
        pal.accent,
    );
    let brim_rect = Rect::from_center_size(
        Pos2::new(head_c.x + look_x * 0.9, head_c.y - head_r * 0.04),
        Vec2::new(head_r * 1.35, (head_r * 0.34).max(2.5)),
    );
    painter.rect_filled(brim_rect, CornerRadius::same(2), pal.accent);

    if rel_yaw.cos() > -0.25 {
        let eye_y = head_c.y + head_r * 0.25;
        let eye_sep = head_r * 0.34;
        let er = (head_r * 0.14).max(1.2);
        painter.circle_filled(Pos2::new(head_c.x + look_x * 0.6 - eye_sep, eye_y), er, outline);
        painter.circle_filled(Pos2::new(head_c.x + look_x * 0.6 + eye_sep, eye_y), er, outline);
    }

    if show_name {
        paint_nameplate(painter, Pos2::new(head_c.x, head_c.y - head_r - 16.0), peer);
    }
}

fn paint_nameplate(painter: &egui::Painter, pill_center: Pos2, peer: &RenderPeer) {
    let pal = avatar_palette(peer.avatar_style);
    let mut label = if peer.is_leader {
        format!("[Chef] {}", peer.nickname)
    } else {
        peer.nickname.clone()
    };
    if peer.in_cutscene {
        label.push_str(" [Cutscene]");
    } else if peer.in_battle {
        label.push_str(" [Battle]");
    }
    let font = FontId::proportional(11.5);
    let galley = painter.layout_no_wrap(label, font, Color32::WHITE);
    let g_sz = galley.size();
    let pill_sz = Vec2::new(g_sz.x + 18.0, 20.0);
    let pill = Rect::from_center_size(pill_center, pill_sz);
    painter.rect_filled(pill, CornerRadius::same(10), Color32::from_rgba_unmultiplied(10, 17, 30, 214));
    let border_col = if peer.is_leader {
        Color32::from_rgb(255, 214, 96)
    } else {
        Color32::from_rgba_unmultiplied(pal.accent.r(), pal.accent.g(), pal.accent.b(), 150)
    };
    painter.rect_stroke(pill, CornerRadius::same(10), Stroke::new(1.0, border_col), StrokeKind::Inside);
    painter.circle_filled(
        Pos2::new(pill.left() + 8.0, pill.center().y),
        3.0,
        if peer.is_leader { Color32::from_rgb(255, 214, 96) } else { pal.cap },
    );
    painter.galley(
        Pos2::new(pill.left() + 14.0, pill.center().y - g_sz.y * 0.5),
        galley,
        Color32::WHITE,
    );
}

fn paint_edge_indicator(
    painter: &egui::Painter,
    bounds: Rect,
    peer: &RenderPeer,
    dir: Vec2,
    dist_tiles: f32,
) {
    let norm = dir.length();
    if norm < 1e-4 {
        return;
    }
    let u = dir / norm;
    let margin = 24.0;
    let half_w = (bounds.width() * 0.5 - margin).max(16.0);
    let half_h = (bounds.height() * 0.5 - margin).max(16.0);
    let tx = if u.x.abs() > 1e-4 { half_w / u.x.abs() } else { f32::MAX };
    let ty = if u.y.abs() > 1e-4 { half_h / u.y.abs() } else { f32::MAX };
    let t = tx.min(ty);
    let anchor = bounds.center() + u * t;
    let pal = avatar_palette(peer.avatar_style);

    let tip = anchor + u * 8.0;
    let perp = Vec2::new(-u.y, u.x);
    let p1 = anchor - u * 4.0 + perp * 6.0;
    let p2 = anchor - u * 4.0 - perp * 6.0;
    painter.add(Shape::convex_polygon(
        vec![tip, p1, p2],
        if peer.is_leader { Color32::from_rgb(255, 214, 96) } else { pal.cap },
        Stroke::new(1.0, Color32::WHITE),
    ));

    let tag = if peer.is_leader {
        format!("[Chef] {} ({:.0}m)", peer.nickname, dist_tiles.max(1.0))
    } else {
        format!("{} ({:.0}m)", peer.nickname, dist_tiles.max(1.0))
    };
    let font = FontId::proportional(10.5);
    let galley = painter.layout_no_wrap(tag, font, Color32::WHITE);
    let g_sz = galley.size();
    let lbl_pos = Pos2::new(
        (anchor.x - g_sz.x * 0.5).clamp(bounds.left() + 6.0, bounds.right() - g_sz.x - 6.0),
        (anchor.y + 10.0).clamp(bounds.top() + 4.0, bounds.bottom() - g_sz.y - 4.0),
    );
    let bg = Rect::from_min_size(lbl_pos - Vec2::new(4.0, 2.0), g_sz + Vec2::new(8.0, 4.0));
    painter.rect_filled(bg, CornerRadius::same(4), Color32::from_rgba_unmultiplied(10, 17, 30, 196));
    painter.galley(lbl_pos, galley, Color32::from_rgb(230, 240, 255));
}

pub fn paint_mini_avatar(painter: &egui::Painter, center: Pos2, style_idx: u8) {
    let dummy = RenderPeer {
        mac: [0; 6],
        nickname: String::new(),
        zone_id: 0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        yaw: 0.0,
        speed: 0.0,
        avatar_style: style_idx,
        in_battle: false,
        in_cutscene: false,
        is_leader: false,
        party_count: 0,
    };
    paint_world_avatar(
        painter,
        Pos2::new(center.x, center.y + 15.0),
        32.0,
        &dummy,
        0.0,
        0.0,
        false,
    );
}

pub fn draw_multiplayer_window(
    ctx: &egui::Context,
    open: &mut bool,
    password: &mut String,
    settings: &mut Settings,
    room: Option<&RoomView>,
    room_conn: Option<&Connection>,
    coop: &CoopRuntime,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut win_open = *open;
    let before = settings.clone();

    egui::Window::new("Multiplayer & Co-op Campaign")
        .open(&mut win_open)
        .default_width(450.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.label(
                RichText::new(
                    "Explore Hoenn together! Cutscenes wait cleanly for all players, standard encounters can upgrade to 2v2 Doubles while solo/scripted battles stay intact, and your real save stays 100% yours.",
                )
                .size(12.0)
                .color(Color32::from_rgb(166, 178, 198)),
            );
            ui.add_space(6.0);

            // Post-tutorial unlock & real save status badge
            let unlocked = coop.tutorial_unlocked(settings);
            let (badge_fill, badge_stroke, badge_title, badge_sub) = if unlocked {
                (
                    Color32::from_rgb(20, 42, 34),
                    Color32::from_rgb(56, 168, 114),
                    format!(
                        "Co-op Campaign Unlocked ({} Pokémon in your real party)",
                        coop.local_party_count
                    ),
                    "Your real save file & Pokémon stay 100% yours.".to_owned(),
                )
            } else {
                (
                    Color32::from_rgb(44, 34, 18),
                    Color32::from_rgb(212, 152, 56),
                    "Tutorial Lock — Pick your starter Pokémon to unlock Co-op".to_owned(),
                    "Waiting for party_count >= 1 (post-tutorial). You can still connect to a room now.".to_owned(),
                )
            };
            egui::Frame::NONE
                .fill(badge_fill)
                .stroke(Stroke::new(1.0, badge_stroke))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.label(RichText::new(badge_title).size(12.5).strong().color(Color32::WHITE));
                    ui.label(RichText::new(badge_sub).size(11.0).color(Color32::from_rgb(204, 214, 228)));
                    let telem = match &coop.last_sample {
                        Some(w) if w.in_overworld => format!(
                            "Telemetry: Overworld · Zone {} · Pos ({:.0}, {:.0}, {:.0})",
                            w.presence.zone_id, w.presence.x, w.presence.y, w.presence.z
                        ),
                        Some(w) if w.presence.in_battle => {
                            format!("Telemetry: In Battle · Zone {}", w.presence.zone_id)
                        }
                        Some(w) => format!(
                            "Telemetry: Menu / Cutscene · Zone {}",
                            w.presence.zone_id
                        ),
                        None => "Telemetry: Waiting for ORAS world state…".to_owned(),
                    };
                    ui.label(RichText::new(telem).size(10.5).monospace().color(Color32::from_rgb(154, 206, 255)));
                    ui.label(
                        RichText::new(format!("Diag: {}", coop.last_diag))
                            .size(9.5)
                            .monospace()
                            .color(Color32::from_rgb(142, 162, 192)),
                    );
                });
            ui.add_space(8.0);

            // Profile + Avatar color picker
            egui::Frame::NONE
                .fill(Color32::from_rgb(26, 29, 36))
                .stroke(Stroke::new(1.0, Color32::from_rgb(48, 52, 62)))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (av_rect, _) = ui.allocate_exact_size(Vec2::new(40.0, 42.0), egui::Sense::hover());
                        ui.painter().rect_filled(av_rect, CornerRadius::same(8), Color32::from_rgb(16, 22, 34));
                        paint_mini_avatar(ui.painter(), av_rect.center(), settings.coop_avatar_style);

                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Trainer Name").size(12.5).strong());
                                ui.add(
                                    egui::TextEdit::singleline(&mut settings.nickname)
                                        .char_limit(10)
                                        .desired_width(140.0)
                                        .hint_text("Player"),
                                );
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Avatar Color").size(12.0).color(Color32::from_rgb(172, 180, 194)));
                                for (idx, pal) in AVATAR_PALETTES.iter().enumerate() {
                                    let selected = (settings.coop_avatar_style as usize % AVATAR_PALETTES.len()) == idx;
                                    let (sw_rect, sw_resp) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::click());
                                    ui.painter().circle_filled(sw_rect.center(), if selected { 8.0 } else { 6.5 }, pal.cap);
                                    if selected {
                                        ui.painter().circle_stroke(sw_rect.center(), 9.0, Stroke::new(1.5, Color32::WHITE));
                                    }
                                    if sw_resp.clicked() {
                                        settings.coop_avatar_style = idx as u8;
                                    }
                                    sw_resp.on_hover_text(pal.name);
                                }
                            });
                        });
                    });
                });

            ui.add_space(8.0);

            match room {
                Some(room) => {
                    egui::Frame::NONE
                        .fill(Color32::from_rgb(22, 32, 46))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(66, 114, 178)))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("Room: {}", room.name)).size(14.5).strong().color(Color32::WHITE));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    let leave = if room.port.is_some() { "Close Room" } else { "Leave Room" };
                                    if ui.button(leave).clicked() {
                                        actions.push(Action::LeaveRoom);
                                    }
                                    if room_conn.is_some_and(|c| c.members().len() > 1) {
                                        let lbl = if room_conn.is_some_and(|c| !c.is_leader()) {
                                            "TP to Chef"
                                        } else {
                                            "TP to Mate"
                                        };
                                        if ui.button(lbl).clicked() {
                                            actions.push(Action::TeleportToLeader);
                                        }
                                    }
                                });
                            });
                            if let Some(port) = room.port {
                                let share_addr = match &coop.local_ip {
                                    Some(ip) => format!("{ip}:{port}"),
                                    None => format!("<Your IP>:{port}"),
                                };
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("LAN / Direct IP:").size(12.0).color(Color32::from_rgb(176, 198, 224)));
                                    ui.label(RichText::new(&share_addr).size(12.5).strong().monospace().color(Color32::from_rgb(132, 212, 255)));
                                    if ui.small_button("Copy").clicked() {
                                        ctx.copy_text(share_addr);
                                    }
                                });
                            }
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(4.0);

                            if let Some(conn) = room_conn {
                                let members = conn.members();
                                let leader_mac = conn.leader();
                                let local_mac = conn.mac();
                                let i_am_leader = conn.is_leader();
                                let others_count = members.iter().filter(|m| m.mac != local_mac).count();
                                let conn_txt = if others_count > 0 {
                                    format!("CONNECTED — You + {others_count} teammate(s) are in this room!")
                                } else {
                                    "ALONE IN ROOM (1/1) — Waiting for teammate to join your IP:Port".to_owned()
                                };
                                let conn_col = if others_count > 0 {
                                    Color32::from_rgb(112, 240, 168)
                                } else {
                                    Color32::from_rgb(252, 206, 108)
                                };
                                ui.label(RichText::new(conn_txt).size(12.0).strong().color(conn_col));
                                ui.add_space(4.0);
                                let local_zone = coop.last_sample.as_ref().map(|s| s.presence.zone_id);
                                ui.label(RichText::new(format!("Party Members ({})", members.len())).size(12.5).strong());
                                for m in members {
                                    let is_chef = leader_mac == Some(m.mac);
                                    let is_self = local_mac == m.mac;
                                    ui.horizontal(|ui| {
                                        let role = if is_chef { "[Chef] " } else { "" };
                                        let self_tag = if is_self { " (You)" } else { "" };
                                        let col = if is_chef {
                                            Color32::from_rgb(255, 226, 134)
                                        } else {
                                            Color32::from_rgb(220, 230, 244)
                                        };
                                        ui.label(
                                            RichText::new(format!("• {role}{}{self_tag}", m.nickname))
                                                .size(12.0)
                                                .strong()
                                                .color(col),
                                        );
                                        if !is_self {
                                            if let Some(remote) = coop.smoothed.get(&m.mac) {
                                                let z = remote.presence.zone_id;
                                                let (status_s, status_c) = if z == 0 {
                                                    ("Title / Syncing".to_owned(), Color32::from_rgb(176, 188, 210))
                                                } else if local_zone == Some(z) {
                                                    (format!("Same Zone ({z})"), Color32::from_rgb(122, 238, 174))
                                                } else {
                                                    (format!("Zone {z}"), Color32::from_rgb(142, 198, 255))
                                                };
                                                ui.label(RichText::new(format!("· {status_s}")).size(11.0).monospace().color(status_c));
                                            } else {
                                                ui.label(RichText::new("· Connected").size(11.0).monospace().color(Color32::from_rgb(122, 238, 174)));
                                            }
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                if ui.small_button("TP").clicked() {
                                                    actions.push(Action::TeleportToPeer(m.mac));
                                                }
                                                if i_am_leader && !is_chef {
                                                    if ui.small_button("Make Chef").clicked() {
                                                        actions.push(Action::SetCoopLeader(m.mac));
                                                    }
                                                }
                                            });
                                        }
                                    });
                                }
                            } else {
                                ui.label(RichText::new(format!("Connected Players ({})", room.members.len())).size(12.5).strong());
                                for (nick, game) in &room.members {
                                    let label = if game.is_empty() {
                                        format!("• {nick}")
                                    } else {
                                        format!("• {nick}  —  {game}")
                                    };
                                    ui.label(RichText::new(label).size(12.0).color(Color32::from_rgb(220, 230, 244)));
                                }
                            }

                            if let Some(download_play) = &room.download_play {
                                ui.add_space(6.0);
                                ui.separator();
                                ui.label(RichText::new("Download Play").size(12.5).strong());
                                match &download_play.doing {
                                    Some((doing, progress)) => {
                                        ui.label(doing);
                                        if let Some(progress) = progress {
                                            ui.add(egui::ProgressBar::new(*progress).show_percentage());
                                        }
                                        if ui.button("Leave").clicked() {
                                            actions.push(Action::DlpLeave);
                                        }
                                    }
                                    None if download_play.offers.is_empty() => {
                                        ui.label(RichText::new("No active Download Play offers in this room.").size(11.5).weak());
                                    }
                                    None => {
                                        for (index, offer) in download_play.offers.iter().enumerate() {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(&offer.title).strong());
                                                ui.label(RichText::new(format!("from {}", offer.host)).weak());
                                                if ui.add_enabled(offer.blocked.is_none(), egui::Button::new("Join")).clicked() {
                                                    actions.push(Action::DlpJoin(index));
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                        });
                }
                None => {
                    egui::Frame::NONE
                        .fill(Color32::from_rgb(24, 28, 36))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(48, 52, 62)))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.label(RichText::new("Nearby LAN Rooms (Auto-Detected)").size(13.0).strong());
                            ui.add_space(4.0);
                            if coop.lan_rooms.is_empty() {
                                ui.label(
                                    RichText::new("Scanning local network on UDP 24874… Host a room on another PC/Mac to see it here.")
                                        .size(11.5)
                                        .weak(),
                                );
                            } else {
                                for r in &coop.lan_rooms {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(&r.name).strong());
                                        ui.label(RichText::new(format!("({})  ·  {} players", r.address, r.members)).size(11.5).weak());
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if ui.button("Join").clicked() {
                                                actions.push(Action::JoinLanRoom(r.address.clone()));
                                            }
                                        });
                                    });
                                }
                            }
                        });

                    ui.add_space(8.0);

                    ui.columns(2, |cols| {
                        cols[0].group(|ui| {
                            ui.label(RichText::new("Join by IP:Port").size(13.0).strong());
                            ui.add_space(4.0);
                            ui.label(RichText::new("Address").size(11.5).weak());
                            ui.add(egui::TextEdit::singleline(&mut settings.room_address).hint_text("192.168.1.10:24873"));
                            ui.label(RichText::new("Password").size(11.5).weak());
                            ui.add(egui::TextEdit::singleline(password).password(true).hint_text("optional"));
                            ui.add_space(6.0);
                            if ui.add_enabled(!settings.room_address.trim().is_empty(), egui::Button::new("Join Room")).clicked() {
                                actions.push(Action::JoinRoom);
                            }
                        });

                        cols[1].group(|ui| {
                            ui.label(RichText::new("Host a Room (Chef)").size(13.0).strong());
                            ui.add_space(4.0);
                            ui.label(RichText::new("Room Name").size(11.5).weak());
                            ui.add(egui::TextEdit::singleline(&mut settings.room_name).hint_text("BlueOrb Hoenn"));
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Port").size(11.5).weak());
                                ui.add(egui::DragValue::new(&mut settings.room_port).range(1024..=65535));
                            });
                            if let Some(ip) = &coop.local_ip {
                                ui.label(RichText::new(format!("Your LAN IP: {ip}")).size(11.0).monospace().weak());
                            }
                            ui.add_space(4.0);
                            if ui.button("Host Room").clicked() {
                                actions.push(Action::HostRoom);
                            }
                        });
                    });
                }
            }

            ui.add_space(8.0);

            // Co-op Campaign & Overworld options
            egui::Frame::NONE
                .fill(Color32::from_rgb(24, 27, 34))
                .stroke(Stroke::new(1.0, Color32::from_rgb(48, 52, 62)))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.label(RichText::new("Co-op Campaign Settings").size(13.0).strong());
                    ui.add_space(4.0);
                    ui.checkbox(&mut settings.coop_enabled, "Show other players live in the 3D overworld");
                    ui.checkbox(&mut settings.coop_follow_leader, "Auto-TP guests when the Chef changes zone");
                    ui.checkbox(&mut settings.coop_sync_cutscenes, "Sync cutscenes (wait for all players with a notice before warping)");
                    ui.checkbox(&mut settings.coop_double_battle, "Co-op Double Battles (2v2) on standard battles (keeps solo/story fights intact)");
                    ui.checkbox(&mut settings.coop_show_names, "Show trainer nameplates, [Chef] badge & arrows");
                    ui.checkbox(&mut settings.coop_show_hud, "Show Co-op HUD & live partner Pokémon squad");
                    ui.checkbox(&mut settings.coop_force_unlock, "Override tutorial lock (allow Co-op before first Pokémon)");
                    ui.separator();
                    let mut preview = coop.preview_buddy;
                    if ui.checkbox(&mut preview, "Solo Preview Buddy (test animated co-op trainer near you)").changed() {
                        actions.push(Action::ToggleCoopPreview);
                    }
                    let mut native_npc = coop.native_3d_npc_sync;
                    if ui.checkbox(&mut native_npc, "Real 3D In-Game NPC Model (uses a map NPC as Player 2, Experimental)").changed() {
                        actions.push(Action::ToggleNativeNpcSync);
                    }
                });
        });

    *open = win_open;
    if *settings != before {
        actions.push(Action::Settings);
    }
    actions
}

fn wrap_angle(a: f32) -> f32 {
    let mut x = a;
    while x > std::f32::consts::PI {
        x -= std::f32::consts::TAU;
    }
    while x < -std::f32::consts::PI {
        x += std::f32::consts::TAU;
    }
    x
}

#[inline]
fn read_u8(mem: &mut Memory, vaddr: u32) -> Option<u8> {
    if !mem.is_readable(vaddr, 1) {
        return None;
    }
    let mut buf = [0u8; 1];
    mem.read_bytes(vaddr, &mut buf);
    Some(buf[0])
}

#[inline]
fn read_u16(mem: &mut Memory, vaddr: u32) -> Option<u16> {
    if !mem.is_readable(vaddr, 2) {
        return None;
    }
    let mut buf = [0u8; 2];
    mem.read_bytes(vaddr, &mut buf);
    Some(u16::from_le_bytes(buf))
}

#[inline]
fn read_u32(mem: &mut Memory, vaddr: u32) -> Option<u32> {
    if vaddr == 0 || !mem.is_readable(vaddr, 4) {
        return None;
    }
    let mut buf = [0u8; 4];
    mem.read_bytes(vaddr, &mut buf);
    Some(u32::from_le_bytes(buf))
}

#[inline]
fn read_f32(mem: &mut Memory, vaddr: u32) -> Option<f32> {
    let bits = read_u32(mem, vaddr)?;
    let v = f32::from_bits(bits);
    v.is_finite().then_some(v)
}

#[inline]
fn is_heap_ptr(addr: u32) -> bool {
    (addr & 3) == 0
        && ((0x0800_0000..0x1000_0000).contains(&addr)
            || (0x1400_0000..0x1C00_0000).contains(&addr)
            || (0x3000_0000..0x4000_0000).contains(&addr))
}

fn is_battle_active(mem: &mut Memory, team_ptr: Option<u32>) -> bool {
    let Some(tp) = team_ptr else {
        return false;
    };
    let Some(cfg) = read_u32(mem, ORAS_BATTLE_MAIN_PROCESS.wrapping_add(0x38)).filter(|&p| is_heap_ptr(p)) else {
        return false;
    };
    let Some(mgr) = read_u32(mem, ORAS_BATTLE_MAIN_PROCESS.wrapping_add(0x3C)).filter(|&p| is_heap_ptr(p)) else {
        return false;
    };
    let _ = mgr;
    read_u32(mem, cfg.wrapping_add(0x4C)) == Some(tp)
}

fn check_candidate_player_model(mem: &mut Memory, m: u32) -> Option<u32> {
    let flags0 = read_u32(mem, m.wrapping_add(0x04))?;
    if (flags0 & 1) == 0 {
        return None;
    }
    let id = read_u16(mem, m.wrapping_add(0x0C))?;
    if id != 0xFF {
        return None;
    }
    let px = read_f32(mem, m.wrapping_add(0x114))?;
    let py = read_f32(mem, m.wrapping_add(0x118))?;
    let pz = read_f32(mem, m.wrapping_add(0x11C))?;
    if px.abs() < 100_000.0 && py.abs() < 100_000.0 && pz.abs() < 100_000.0 {
        Some(m)
    } else {
        None
    }
}

fn try_models_offset(mem: &mut Memory, model_mgr: u32, off: u32) -> Option<u32> {
    let models_base = read_u32(mem, model_mgr.wrapping_add(off)).filter(|&p| is_heap_ptr(p))?;
    // Check struct array layout (`models_base + i * 0xAB0`)
    for i in 0..32u32 {
        let m = models_base.wrapping_add(i * 0xAB0);
        if !mem.is_readable(m, 0x120) {
            break;
        }
        if let Some(found) = check_candidate_player_model(mem, m) {
            return Some(found);
        }
    }
    // Check pointer table layout (`*(models_base + i * 4)`)
    for i in 0..32u32 {
        let Some(ptr) = read_u32(mem, models_base.wrapping_add(i * 4)).filter(|&p| is_heap_ptr(p)) else {
            continue;
        };
        if let Some(found) = check_candidate_player_model(mem, ptr) {
            return Some(found);
        }
    }
    None
}

fn find_player_model(
    mem: &mut Memory,
    model_mgr: u32,
    offset_hint: &mut u32,
    last_model_scan: &mut Instant,
) -> Option<u32> {
    if let Some(m) = try_models_offset(mem, model_mgr, *offset_hint) {
        return Some(m);
    }
    for off in (0x3600u32..=0x3780).step_by(4).chain((0x1450u32..=0x1540).step_by(4)) {
        if let Some(m) = try_models_offset(mem, model_mgr, off) {
            *offset_hint = off;
            return Some(m);
        }
    }
    if last_model_scan.elapsed() >= Duration::from_secs(1) {
        *last_model_scan = Instant::now();
        for off in (0x0000u32..=0x4000).step_by(4) {
            if let Some(m) = try_models_offset(mem, model_mgr, off) {
                *offset_hint = off;
                return Some(m);
            }
        }
    }
    None
}

fn find_models_base_by_vtable(
    mem: &mut Memory,
    hint: &mut Option<u32>,
    last_scan: &mut Instant,
) -> Option<u32> {
    if let Some(base) = *hint {
        if read_u32(mem, base) == Some(ORAS_MOVEMODEL_VTABLE)
            && read_u32(mem, base.wrapping_add(ORAS_MOVEMODEL_STRIDE)) == Some(ORAS_MOVEMODEL_VTABLE)
        {
            return Some(base);
        }
    }
    if last_scan.elapsed() < Duration::from_secs(1) {
        return None;
    }
    *last_scan = Instant::now();

    let regions: Vec<(u32, u32)> = mem
        .mappings()
        .filter(|m| is_heap_ptr(m.base) && m.size >= ORAS_MOVEMODEL_STRIDE * 2)
        .map(|m| (m.base, m.size))
        .collect();
    let vtable_bytes = ORAS_MOVEMODEL_VTABLE.to_le_bytes();
    let mut chunk = vec![0u8; 64 * 1024];
    for (r_base, r_size) in regions {
        let mut off = 0u32;
        while off + ORAS_MOVEMODEL_STRIDE < r_size {
            let to_read = (r_size - off).min(chunk.len() as u32) & !3;
            if to_read < 8 {
                break;
            }
            let addr = r_base.wrapping_add(off);
            if mem.is_readable(addr, to_read) {
                mem.read_bytes(addr, &mut chunk[..to_read as usize]);
                for rel in (0..to_read as usize).step_by(4) {
                    if chunk[rel..rel + 4] == vtable_bytes {
                        let cand = addr.wrapping_add(rel as u32);
                        if read_u32(mem, cand.wrapping_add(ORAS_MOVEMODEL_STRIDE))
                            == Some(ORAS_MOVEMODEL_VTABLE)
                        {
                            let mut slot0 = cand;
                            for _ in 0..32 {
                                let prev = slot0.wrapping_sub(ORAS_MOVEMODEL_STRIDE);
                                if read_u32(mem, prev) == Some(ORAS_MOVEMODEL_VTABLE) {
                                    slot0 = prev;
                                } else {
                                    break;
                                }
                            }
                            *hint = Some(slot0);
                            return Some(slot0);
                        }
                    }
                }
            }
            off += to_read;
        }
    }
    None
}

fn scan_models_array(mem: &mut Memory, models_base: u32) -> (Option<u32>, Vec<u32>) {
    let mut player = None;
    let mut npcs: Vec<(bool, u32)> = Vec::new();
    for i in 0..32u32 {
        let m = models_base.wrapping_add(i * ORAS_MOVEMODEL_STRIDE);
        if !mem.is_readable(m, 0x120) {
            break;
        }
        let Some(flags0) = read_u32(mem, m.wrapping_add(0x04)) else { continue };
        if (flags0 & 1) == 0 {
            continue;
        }
        let Some(id) = read_u16(mem, m.wrapping_add(0x0C)) else { continue };
        if id == 0xFF {
            if check_candidate_player_model(mem, m).is_some() {
                player = Some(m);
            }
        } else if read_u32(mem, m) == Some(ORAS_MOVEMODEL_VTABLE) {
            let obj_code = read_u16(mem, m.wrapping_add(0x0E)).unwrap_or(0);
            let move_code = read_u16(mem, m.wrapping_add(0x10)).unwrap_or(0);
            if (1..=260).contains(&obj_code) {
                npcs.push((move_code != 0, m));
            }
        }
    }
    // Prioritize living NPCs with active movement AI over static objects
    npcs.sort_by_key(|&(has_ai, _)| !has_ai);
    (player, npcs.into_iter().map(|(_, m)| m).collect())
}

fn fallback_overworld_camera(player: [f32; 3]) -> WorldCamera {
    let eye = [player[0], player[1] + 150.0, player[2] + 170.0];
    let target = [player[0], player[1] + 8.0, player[2]];
    let f = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let flen = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt().max(1e-4);
    let fwd = [f[0] / flen, f[1] / flen, f[2] / flen];
    // Right = normalize(fwd x up(0,1,0)) = [-fwd.z, 0, fwd.x] normalized
    let r_raw = [-fwd[2], 0.0, fwd[0]];
    let rlen = (r_raw[0] * r_raw[0] + r_raw[2] * r_raw[2]).sqrt().max(1e-4);
    let right = [r_raw[0] / rlen, 0.0, r_raw[2] / rlen];
    let up = [
        right[1] * fwd[2] - right[2] * fwd[1],
        right[2] * fwd[0] - right[0] * fwd[2],
        right[0] * fwd[1] - right[1] * fwd[0],
    ];
    let tx = -(right[0] * eye[0] + right[1] * eye[1] + right[2] * eye[2]);
    let ty = -(up[0] * eye[0] + up[1] * eye[1] + up[2] * eye[2]);
    let tz = fwd[0] * eye[0] + fwd[1] * eye[1] + fwd[2] * eye[2];
    WorldCamera {
        view: [
            [right[0], right[1], right[2], tx],
            [up[0], up[1], up[2], ty],
            [-fwd[0], -fwd[1], -fwd[2], tz],
        ],
        proj: default_perspective_mtx(),
        rotated_landscape: false,
        right_sign: 1.0,
        up_sign: 1.0,
    }
}

/// Checks `GameManager -> EventManager (+0x18)` to detect active ORAS cutscenes / scripted events.
fn is_event_or_cutscene_active(mem: &mut Memory, gm: u32) -> bool {
    let Some(event_mgr) = read_u32(mem, gm.wrapping_add(0x18)).filter(|&p| is_heap_ptr(p)) else {
        return false;
    };
    if read_u32(mem, event_mgr.wrapping_add(0x04)) != Some(gm) {
        return false;
    }
    let current = read_u32(mem, event_mgr).is_some_and(is_heap_ptr);
    let pending = read_u32(mem, event_mgr.wrapping_add(0x08)).is_some_and(is_heap_ptr);
    current || pending
}

fn read_party_summary(mem: &mut Memory, data_mgr: u32) -> (Option<u32>, u8, Vec<PartyMonSummary>) {
    let Some(team_ptr) = read_u32(mem, data_mgr.wrapping_add(0x14)).filter(|&p| is_heap_ptr(p)) else {
        return (None, 0, Vec::new());
    };
    let count = read_u8(mem, team_ptr.wrapping_add(0x18)).unwrap_or(0).min(6);
    let mut mons = Vec::with_capacity(count as usize);
    for slot in 0..count as u32 {
        let Some(param) = read_u32(mem, team_ptr.wrapping_add(slot * 4)).filter(|&p| is_heap_ptr(p)) else {
            continue;
        };
        let Some(rt) = read_u32(mem, param.wrapping_add(0x08)).filter(|&p| is_heap_ptr(p)) else {
            continue;
        };
        let level = read_u8(mem, rt.wrapping_add(0x04)).unwrap_or(0);
        let hp = read_u16(mem, rt.wrapping_add(0x08)).unwrap_or(0);
        let max_hp = read_u16(mem, rt.wrapping_add(0x0A)).unwrap_or(0);
        if (1..=100).contains(&level) && (1..=999).contains(&max_hp) && hp <= max_hp {
            mons.push(PartyMonSummary { level, hp, max_hp });
        }
    }
    let effective_count = if mons.is_empty() && count > 0 { 0 } else { count.max(mons.len() as u8) };
    (Some(team_ptr), effective_count, mons)
}

fn is_next_map_idle(mem: &mut Memory, map_mgr: u32) -> bool {
    is_heap_ptr(map_mgr) && read_u32(mem, map_mgr.wrapping_add(0xF8)) == Some(0xFFFF)
}

fn trigger_map_warp(mem: &mut Memory, map_mgr: u32, target_zone: u16) -> bool {
    if !is_heap_ptr(map_mgr) || !(1..536).contains(&target_zone) {
        return false;
    }
    if read_u32(mem, map_mgr.wrapping_add(0xF8)) != Some(0xFFFF) {
        return false;
    }
    let next_id_addr = map_mgr.wrapping_add(0xF8);
    if !mem.is_writable(next_id_addr, 8) {
        return false;
    }
    mem.write_bytes(next_id_addr, &(target_zone as u32).to_le_bytes());
    mem.write_bytes(map_mgr.wrapping_add(0xFC), &0u32.to_le_bytes());
    true
}

fn teleport_player_coords(mem: &mut Memory, player_slot: u32, pos: [f32; 3]) -> bool {
    if !is_heap_ptr(player_slot) {
        return false;
    }
    if !pos.iter().all(|v| v.is_finite() && v.abs() < 100_000.0) {
        return false;
    }
    let mut raw = [0u8; 12];
    raw[0..4].copy_from_slice(&pos[0].to_le_bytes());
    raw[4..8].copy_from_slice(&pos[1].to_le_bytes());
    raw[8..12].copy_from_slice(&pos[2].to_le_bytes());
    let world_addr = player_slot.wrapping_add(0xEC);
    let draw_addr = player_slot.wrapping_add(0x114);
    if !mem.is_writable(world_addr, 12) || !mem.is_writable(draw_addr, 12) {
        return false;
    }
    mem.write_bytes(world_addr, &raw);
    mem.write_bytes(draw_addr, &raw);
    true
}

fn read_vec3(mem: &mut Memory, addr: u32) -> Option<[f32; 3]> {
    let x = read_f32(mem, addr)?;
    let y = read_f32(mem, addr.wrapping_add(4))?;
    let z = read_f32(mem, addr.wrapping_add(8))?;
    if x.abs() < 100_000.0 && y.abs() < 100_000.0 && z.abs() < 100_000.0 {
        Some([x, y, z])
    } else {
        None
    }
}

fn write_model_facing(mem: &mut Memory, model_addr: u32, yaw: f32) {
    let facing_addr = model_addr.wrapping_add(0x1C);
    if !mem.is_writable(facing_addr, 12) {
        return;
    }
    let mut raw = [0u8; 12];
    raw[0..4].copy_from_slice(&yaw.sin().to_le_bytes());
    raw[4..8].copy_from_slice(&0.0f32.to_le_bytes());
    raw[8..12].copy_from_slice(&yaw.cos().to_le_bytes());
    mem.write_bytes(facing_addr, &raw);
}

fn write_vec3_if_writable(mem: &mut Memory, addr: u32, pos: [f32; 3]) -> bool {
    if !pos.iter().all(|v| v.is_finite() && v.abs() < 100_000.0) {
        return false;
    }
    if !mem.is_writable(addr, 12) {
        return false;
    }
    let mut raw = [0u8; 12];
    raw[0..4].copy_from_slice(&pos[0].to_le_bytes());
    raw[4..8].copy_from_slice(&pos[1].to_le_bytes());
    raw[8..12].copy_from_slice(&pos[2].to_le_bytes());
    mem.write_bytes(addr, &raw);
    true
}

fn write_u16_if_writable(mem: &mut Memory, addr: u32, val: u16) {
    if mem.is_writable(addr, 2) {
        mem.write_bytes(addr, &val.to_le_bytes());
    }
}

fn set_model_all_positions(mem: &mut Memory, addr: u32, pos: [f32; 3]) {
    for off in [0x94u32, 0xC0, 0xEC, 0x114] {
        write_vec3_if_writable(mem, addr.wrapping_add(off), pos);
    }
}

fn restore_hijacked_npc(mem: &mut Memory, hijacked: &mut Option<HijackedNpc>) {
    let Some(saved) = hijacked.take() else {
        return;
    };
    if !is_heap_ptr(saved.addr) {
        return;
    }
    if read_u16(mem, saved.addr.wrapping_add(0x0E)) != Some(saved.obj_code) {
        return;
    }
    write_u16_if_writable(mem, saved.addr.wrapping_add(0x10), saved.orig_move_code);
    write_vec3_if_writable(mem, saved.addr.wrapping_add(0x94), saved.orig_prev);
    write_vec3_if_writable(mem, saved.addr.wrapping_add(0xC0), saved.orig_next);
    write_vec3_if_writable(mem, saved.addr.wrapping_add(0xEC), saved.orig_now);
    write_vec3_if_writable(mem, saved.addr.wrapping_add(0x114), saved.orig_draw);
}

/// Safely upgrades eligible standard battles (`Format::kSingle`) to `Format::kDouble` (2v2)
/// via `battle::Process` (`0x08C69214`). Strictly preserves solo, scripted, 1-mon trainer,
/// Sky, Multi, and forced-capture battles as well as player's `SaveData`.
fn try_upgrade_battle_to_double(
    mem: &mut Memory,
    local_team_ptr: u32,
    healthy_party_count: u8,
    recent_scripted_event: bool,
) -> bool {
    let mut candidates = [0u32; 2];
    candidates[0] = read_u32(mem, ORAS_BATTLE_MAIN_PROCESS.wrapping_add(0x38)).unwrap_or(0);
    if let Some(mgr) = read_u32(mem, ORAS_BATTLE_MAIN_PROCESS.wrapping_add(0x3C)).filter(|&p| is_heap_ptr(p)) {
        candidates[1] = read_u32(mem, mgr.wrapping_add(0x10)).unwrap_or(0);
    }

    for cfg in candidates {
        if !is_heap_ptr(cfg) {
            continue;
        }
        if read_u32(mem, cfg.wrapping_add(0x4C)) != Some(local_team_ptr) {
            continue;
        }
        let Some(opp_team) = read_u32(mem, cfg.wrapping_add(0x50)).filter(|&p| is_heap_ptr(p)) else {
            continue;
        };
        let is_against_trainer = read_u8(mem, cfg.wrapping_add(0x00)).unwrap_or(0) != 0;
        let Some(fmt) = read_u8(mem, cfg.wrapping_add(0x01)) else { continue };
        let opp_count = read_u8(mem, opp_team.wrapping_add(0x18)).unwrap_or(0);
        let has_native_ally_or_multi = read_u32(mem, cfg.wrapping_add(0x54)).is_some_and(is_heap_ptr)
            || read_u32(mem, cfg.wrapping_add(0x58)).is_some_and(is_heap_ptr);
        let is_sky_battle = read_u8(mem, cfg.wrapping_add(0xB0)).unwrap_or(0) != 0;
        let is_capture_forced = read_u8(mem, cfg.wrapping_add(0xB2)).unwrap_or(0) != 0;
        let no_money = read_u8(mem, cfg.wrapping_add(0xB3)).unwrap_or(0) != 0;

        if !should_upgrade_battle_to_double(
            healthy_party_count,
            fmt,
            is_against_trainer,
            opp_count,
            is_sky_battle,
            is_capture_forced,
            no_money,
            has_native_ally_or_multi,
            recent_scripted_event,
        ) {
            continue;
        }

        if !mem.is_writable(cfg.wrapping_add(0x01), 1) {
            continue;
        }

        // Duplicate wild encounter foe slot 0 -> slot 1 only for standard non-scripted wild battles
        if !is_against_trainer && opp_count == 1 {
            if let (Some(mon0), Some(mon1)) = (
                read_u32(mem, opp_team).filter(|&p| is_heap_ptr(p)),
                read_u32(mem, opp_team.wrapping_add(4)).filter(|&p| is_heap_ptr(p)),
            ) {
                if let (Some(core0), Some(rt0), Some(core1), Some(rt1)) = (
                    read_u32(mem, mon0.wrapping_add(4)).filter(|&p| is_heap_ptr(p)),
                    read_u32(mem, mon0.wrapping_add(8)).filter(|&p| is_heap_ptr(p)),
                    read_u32(mem, mon1.wrapping_add(4)).filter(|&p| is_heap_ptr(p)),
                    read_u32(mem, mon1.wrapping_add(8)).filter(|&p| is_heap_ptr(p)),
                ) {
                    if mem.is_readable(core0, 232)
                        && mem.is_writable(core1, 232)
                        && mem.is_readable(rt0, 28)
                        && mem.is_writable(rt1, 28)
                        && mem.is_writable(opp_team.wrapping_add(0x18), 1)
                    {
                        let mut core_buf = [0u8; 232];
                        let mut rt_buf = [0u8; 28];
                        mem.read_bytes(core0, &mut core_buf);
                        mem.read_bytes(rt0, &mut rt_buf);
                        mem.write_bytes(core1, &core_buf);
                        mem.write_bytes(rt1, &rt_buf);
                        mem.write_bytes(opp_team.wrapping_add(0x18), &[2u8]);
                    }
                }
            }
        }

        mem.write_bytes(cfg.wrapping_add(0x01), &[1u8]);
        return true;
    }

    false
}

fn read_mtx34(mem: &mut Memory, vaddr: u32) -> Option<[[f32; 4]; 3]> {
    if !mem.is_readable(vaddr, 48) {
        return None;
    }
    let mut raw = [0u8; 48];
    mem.read_bytes(vaddr, &mut raw);
    let mut m = [[0.0f32; 4]; 3];
    for r in 0..3 {
        for c in 0..4 {
            let idx = (r * 4 + c) * 4;
            let v = f32::from_le_bytes([raw[idx], raw[idx + 1], raw[idx + 2], raw[idx + 3]]);
            if !v.is_finite() {
                return None;
            }
            m[r][c] = v;
        }
    }
    Some(m)
}

fn read_mtx44(mem: &mut Memory, vaddr: u32) -> Option<[[f32; 4]; 4]> {
    if !mem.is_readable(vaddr, 64) {
        return None;
    }
    let mut raw = [0u8; 64];
    mem.read_bytes(vaddr, &mut raw);
    let mut m = [[0.0f32; 4]; 4];
    for r in 0..4 {
        for c in 0..4 {
            let idx = (r * 4 + c) * 4;
            let v = f32::from_le_bytes([raw[idx], raw[idx + 1], raw[idx + 2], raw[idx + 3]]);
            if !v.is_finite() {
                return None;
            }
            m[r][c] = v;
        }
    }
    Some(m)
}

fn world_to_camera(view: &[[f32; 4]; 3], p: [f32; 3]) -> [f32; 3] {
    [
        view[0][0] * p[0] + view[0][1] * p[1] + view[0][2] * p[2] + view[0][3],
        view[1][0] * p[0] + view[1][1] * p[1] + view[1][2] * p[2] + view[1][3],
        view[2][0] * p[0] + view[2][1] * p[1] + view[2][2] * p[2] + view[2][3],
    ]
}

fn validate_view_matrix(view: &[[f32; 4]; 3], player: [f32; 3]) -> bool {
    for row in view {
        let len = (row[0] * row[0] + row[1] * row[1] + row[2] * row[2]).sqrt();
        if (len - 1.0).abs() > 0.12 {
            return false;
        }
    }
    let cam = world_to_camera(view, player);
    cam[2] < -4.0 && cam[2] > -20_000.0
}

fn default_perspective_mtx() -> [[f32; 4]; 4] {
    let fov_y = 26.0_f32.to_radians();
    let f = 1.0 / (fov_y * 0.5).tan();
    let aspect = 400.0 / 240.0;
    let near = 1.0_f32;
    let far = 10_000.0_f32;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (far + near) / (near - far), (2.0 * far * near) / (near - far)],
        [0.0, 0.0, -1.0, 0.0],
    ]
}

fn validate_proj_matrix(proj: &[[f32; 4]; 4], cam_p: [f32; 3]) -> Option<(bool, f32, f32)> {
    let w = proj[3][0] * cam_p[0] + proj[3][1] * cam_p[1] + proj[3][2] * cam_p[2] + proj[3][3];
    if w.abs() < 1e-4 {
        return None;
    }
    let x = (proj[0][0] * cam_p[0] + proj[0][1] * cam_p[1] + proj[0][2] * cam_p[2] + proj[0][3]) / w;
    let y = (proj[1][0] * cam_p[0] + proj[1][1] * cam_p[1] + proj[1][2] * cam_p[2] + proj[1][3]) / w;
    if !x.is_finite() || !y.is_finite() || x.abs() > 0.45 || y.abs() > 0.45 {
        return None;
    }
    let w_sign = w.signum();
    if proj[0][0].abs() > 0.25 && proj[1][1].abs() > 0.25 && proj[0][1].abs() < 0.05 && proj[1][0].abs() < 0.05 {
        let rs = (proj[0][0] * w_sign).signum();
        let us = (proj[1][1] * w_sign).signum();
        return Some((false, rs, us));
    }
    if proj[0][1].abs() > 0.25 && proj[1][0].abs() > 0.25 && proj[0][0].abs() < 0.05 && proj[1][1].abs() < 0.05 {
        let rs = (proj[1][0] * w_sign).signum();
        let us = (proj[0][1] * w_sign).signum();
        return Some((true, rs, us));
    }
    None
}

fn project_point(
    cam: &WorldCamera,
    world: [f32; 3],
    top_rect: Rect,
    aspect_scale: f32,
) -> Option<(Pos2, f32)> {
    let c = world_to_camera(&cam.view, world);
    if c[2] >= -1.0 {
        return None;
    }
    let w = cam.proj[3][0] * c[0] + cam.proj[3][1] * c[1] + cam.proj[3][2] * c[2] + cam.proj[3][3];
    if w.abs() < 1e-4 {
        return None;
    }
    let px = (cam.proj[0][0] * c[0] + cam.proj[0][1] * c[1] + cam.proj[0][2] * c[2] + cam.proj[0][3]) / w;
    let py = (cam.proj[1][0] * c[0] + cam.proj[1][1] * c[1] + cam.proj[1][2] * c[2] + cam.proj[1][3]) / w;
    let (ndc_x, ndc_y) = if cam.rotated_landscape {
        (py * cam.right_sign * aspect_scale, px * cam.up_sign)
    } else {
        (px * cam.right_sign * aspect_scale, py * cam.up_sign)
    };
    let sx = top_rect.left() + (ndc_x * 0.5 + 0.5) * top_rect.width();
    let sy = top_rect.top() + (0.5 - ndc_y * 0.5) * top_rect.height();
    Some((Pos2::new(sx, sy), c[2]))
}

fn validate_gm_slot(mem: &mut Memory, slot_addr: u32) -> Option<(u32, u32, u32, u16)> {
    validate_gm_slot_mode(mem, slot_addr, true)
}

fn validate_gm_slot_mode(mem: &mut Memory, slot_addr: u32, strict: bool) -> Option<(u32, u32, u32, u16)> {
    let gm = read_u32(mem, slot_addr)?;
    if !is_heap_ptr(gm) {
        return None;
    }
    let data_mgr = read_u32(mem, gm.wrapping_add(0x1C))?;
    let map_mgr = read_u32(mem, gm.wrapping_add(0x48))?;
    if !is_heap_ptr(data_mgr) || !is_heap_ptr(map_mgr) {
        return None;
    }
    let zone_a = read_u16(mem, data_mgr.wrapping_add(0x124))?;
    let zone_b = read_u32(mem, map_mgr.wrapping_add(0xF0))? as u16;
    if zone_a >= 2048 {
        return None;
    }
    if zone_a != zone_b {
        if strict {
            return None;
        }
        if zone_a == 0 {
            if (1..2048).contains(&zone_b) {
                return Some((gm, data_mgr, map_mgr, zone_b));
            }
            return None;
        }
    }
    Some((gm, data_mgr, map_mgr, zone_a))
}

fn sample_oras_world(
    mem: &mut Memory,
    gm_slot_hint: &mut u32,
    models_offset_hint: &mut u32,
    models_base_hint: &mut Option<u32>,
    proj_offset_hint: &mut Option<u32>,
    last_gm_scan: &mut Instant,
    last_model_scan: &mut Instant,
    last_vtable_scan: &mut Instant,
    last_known_zone: &mut Option<u16>,
    diag: &mut String,
    avatar_style: u8,
) -> Option<SampledWorld> {
    let gm_info = match validate_gm_slot_mode(mem, *gm_slot_hint, false) {
        Some(v) => Some(v),
        None => {
            if last_gm_scan.elapsed() >= Duration::from_secs(1) {
                *last_gm_scan = Instant::now();
                let mut found = None;
                for strict in [true, false] {
                    for addr in (0x0058_0000..=0x0078_0000).step_by(4) {
                        if let Some(v) = validate_gm_slot_mode(mem, addr, strict) {
                            *gm_slot_hint = addr;
                            found = Some(v);
                            break;
                        }
                    }
                    if found.is_some() {
                        break;
                    }
                }
                found
            } else {
                None
            }
        }
    };

    let (has_gm, in_cutscene, team_ptr, party_count, party_mons, map_mgr, gm_zone, model_mgr) =
        match gm_info {
            Some((gm, data_mgr, map_mgr, zone)) => {
                let (tp, pc, pm) = read_party_summary(mem, data_mgr);
                let mm = read_u32(mem, data_mgr.wrapping_add(0x20)).filter(|&p| is_heap_ptr(p));
                (true, is_event_or_cutscene_active(mem, gm), tp, pc, pm, map_mgr, zone, mm)
            }
            None => (false, false, None, 0, Vec::new(), 0, 0, None),
        };

    let mut player_slot = model_mgr.and_then(|mm| find_player_model(mem, mm, models_offset_hint, last_model_scan));
    let mut active_npcs = Vec::new();
    let mut vm_ok = false;

    if let Some(base) = find_models_base_by_vtable(mem, models_base_hint, last_vtable_scan) {
        vm_ok = true;
        let (vp, npcs) = scan_models_array(mem, base);
        active_npcs = npcs;
        if player_slot.is_none() {
            player_slot = vp;
        }
    }

    let Some(player_base) = player_slot else {
        if !has_gm {
            *diag = "GM:none VM:none".to_owned();
            return None;
        }
        let in_battle = is_battle_active(mem, team_ptr);
        *diag = format!(
            "GM:ok Z:{gm_zone} MM:{} VM:{} P:none Cam:none",
            if model_mgr.is_some() { "yes" } else { "no" },
            if vm_ok { "ok" } else { "none" },
        );
        return Some(SampledWorld {
            presence: PlayerPresence {
                zone_id: gm_zone,
                x: 0.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                speed: 0.0,
                avatar_style,
                in_battle,
            },
            in_overworld: false,
            in_cutscene,
            party_count,
            party_mons,
            map_mgr,
            player_slot: None,
            team_ptr,
            camera: None,
            active_npcs,
        });
    };

    let mut px = read_f32(mem, player_base.wrapping_add(0x114))?;
    let mut py = read_f32(mem, player_base.wrapping_add(0x118))?;
    let mut pz = read_f32(mem, player_base.wrapping_add(0x11C))?;
    if px == 0.0 && py == 0.0 && pz == 0.0 {
        if let Some([wx, wy, wz]) = read_vec3(mem, player_base.wrapping_add(0xEC)) {
            px = wx;
            py = wy;
            pz = wz;
        }
    }
    if px.abs() > 100_000.0 || py.abs() > 100_000.0 || pz.abs() > 100_000.0 {
        return None;
    }

    let zone_id = if gm_zone > 0 {
        *last_known_zone = Some(gm_zone);
        gm_zone
    } else {
        last_known_zone.unwrap_or(1)
    };

    let fx = read_f32(mem, player_base.wrapping_add(0x1C)).unwrap_or(0.0);
    let fz = read_f32(mem, player_base.wrapping_add(0x24)).unwrap_or(1.0);
    let mx = read_f32(mem, player_base.wrapping_add(0x28)).unwrap_or(0.0);
    let mz = read_f32(mem, player_base.wrapping_add(0x30)).unwrap_or(0.0);
    let yaw = fx.atan2(fz);
    let speed = mx.hypot(mz);

    let renderer = if is_heap_ptr(map_mgr) {
        read_u32(mem, map_mgr.wrapping_add(0x100)).filter(|&p| is_heap_ptr(p))
    } else {
        None
    };
    let stereo_cam = renderer.and_then(|r| {
        read_u32(mem, r.wrapping_add(0xAA4))
            .filter(|&p| is_heap_ptr(p))
            .or_else(|| read_u32(mem, r.wrapping_add(0xAB8)).filter(|&p| is_heap_ptr(p)))
    });

    let real_cam = stereo_cam.and_then(|sc| {
        let view = read_mtx34(mem, sc.wrapping_add(0x34)).filter(|v| validate_view_matrix(v, [px, py + 9.0, pz]))?;
        let cam_p = world_to_camera(&view, [px, py + 9.0, pz]);
        let left_cam = read_u32(mem, sc.wrapping_add(0x98)).filter(|&p| is_heap_ptr(p));

        let mut resolved_proj = None;
        if let Some(lc) = left_cam {
            if let Some(off) = *proj_offset_hint {
                if let Some(pm) = read_mtx44(mem, lc.wrapping_add(off)) {
                    if let Some((rot, rs, us)) = validate_proj_matrix(&pm, cam_p) {
                        resolved_proj = Some((pm, rot, rs, us));
                    }
                }
            }
            if resolved_proj.is_none() {
                for off in (0x00u32..=0x180).step_by(4) {
                    if let Some(pm) = read_mtx44(mem, lc.wrapping_add(off)) {
                        if let Some((rot, rs, us)) = validate_proj_matrix(&pm, cam_p) {
                            *proj_offset_hint = Some(off);
                            resolved_proj = Some((pm, rot, rs, us));
                            break;
                        }
                    }
                }
            }
        }

        let (proj, rotated_landscape, right_sign, up_sign) =
            resolved_proj.unwrap_or_else(|| (default_perspective_mtx(), false, 1.0, 1.0));
        Some(WorldCamera {
            view,
            proj,
            rotated_landscape,
            right_sign,
            up_sign,
        })
    });
    let used_real_cam = real_cam.is_some();
    let camera = real_cam.or_else(|| Some(fallback_overworld_camera([px, py, pz])));

    let in_battle = is_battle_active(mem, team_ptr);
    *diag = format!(
        "GM:{} VM:{}({}) Z:{} P:({:.0},{:.0},{:.0}) Cam:{}",
        if has_gm { "ok" } else { "no" },
        if vm_ok { "ok" } else { "mm" },
        active_npcs.len() + 1,
        zone_id,
        px,
        py,
        pz,
        if used_real_cam { "3D" } else { "follow" },
    );
    Some(SampledWorld {
        presence: PlayerPresence {
            zone_id,
            x: px,
            y: py,
            z: pz,
            yaw,
            speed,
            avatar_style,
            in_battle,
        },
        in_overworld: !in_battle,
        in_cutscene,
        party_count,
        party_mons,
        map_mgr,
        player_slot: Some(player_base),
        team_ptr,
        camera,
        active_npcs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_beacon_roundtrip() {
        let pkt = encode_lan_beacon("Hoenn|Co-op", 24873, 3);
        let (port, members, name) = decode_lan_beacon(&pkt).unwrap();
        assert_eq!(port, 24873);
        assert_eq!(members, 3);
        assert_eq!(name, "HoennCo-op");
        assert!(decode_lan_beacon("OTHER_PROTO|24873|1|Test").is_none());
    }

    #[test]
    fn tutorial_unlock_requires_starter_unless_overridden() {
        assert!(!is_tutorial_unlocked(0, false));
        assert!(is_tutorial_unlocked(1, false));
        assert!(is_tutorial_unlocked(6, false));
        assert!(is_tutorial_unlocked(0, true));
    }

    #[test]
    fn cutscene_barrier_pauses_sync_until_both_players_finish() {
        assert!(!cutscene_blocks_sync(false, false, false, true));
        assert!(cutscene_blocks_sync(true, false, false, true));
        assert!(cutscene_blocks_sync(false, true, false, true));
        assert!(cutscene_blocks_sync(false, false, true, true));
        // When sync_enabled is false, only local cutscene/settle blocks warps
        assert!(!cutscene_blocks_sync(false, false, true, false));
    }

    #[test]
    fn solo_and_scripted_battles_are_never_forced_to_double() {
        // Need at least 2 healthy Pokémon
        assert!(!should_upgrade_battle_to_double(1, 0, false, 1, false, false, false, false, false));
        // Standard wild encounter with 2+ healthy Pokémon upgrades cleanly
        assert!(should_upgrade_battle_to_double(2, 0, false, 1, false, false, false, false, false));
        // 1-Pokémon trainer duels remain 1v1
        assert!(!should_upgrade_battle_to_double(3, 0, true, 1, false, false, false, false, false));
        // Trainers with 2+ Pokémon can cleanly upgrade to Double Battle
        assert!(should_upgrade_battle_to_double(3, 0, true, 2, false, false, false, false, false));
        // Sky, Capture-forced (Wally), No-money (Birch Poochyena), Multi-Ally, and Scripted Bosses stay solo
        assert!(!should_upgrade_battle_to_double(3, 0, false, 1, true, false, false, false, false));
        assert!(!should_upgrade_battle_to_double(3, 0, false, 1, false, true, false, false, false));
        assert!(!should_upgrade_battle_to_double(3, 0, false, 1, false, false, true, false, false));
        assert!(!should_upgrade_battle_to_double(3, 0, true, 2, false, false, false, true, false));
        assert!(!should_upgrade_battle_to_double(3, 0, false, 1, false, false, false, false, true));
        // Native non-single formats (Double/Triple/Rotation/Horde) stay untouched
        assert!(!should_upgrade_battle_to_double(3, 1, true, 2, false, false, false, false, false));
    }

    #[test]
    fn heap_ptr_accepts_app_and_new3ds_linear_heaps() {
        assert!(is_heap_ptr(0x0800_0000));
        assert!(is_heap_ptr(0x0C12_3450));
        assert!(is_heap_ptr(0x1400_0000));
        assert!(is_heap_ptr(0x1A00_1200));
        assert!(is_heap_ptr(0x3000_0000));
        assert!(is_heap_ptr(0x38AB_CDEF & !3));
        assert!(!is_heap_ptr(0));
        assert!(!is_heap_ptr(0x0800_0002));
        assert!(!is_heap_ptr(0x0010_0000));
    }

    #[test]
    fn fallback_camera_projects_nearby_friend() {
        let local = [100.0, 0.0, 100.0];
        let friend = [120.0, 0.0, 100.0];
        let cam = fallback_overworld_camera(local);
        let top = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 480.0));
        let (pt, depth) = project_point(&cam, friend, top, 1.0).unwrap();
        assert!(top.contains(pt));
        assert!(pt.x > top.center().x);
        assert!(depth < -50.0);
    }

    #[test]
    fn overworld_projection_projects_front_and_rejects_behind() {
        let cam = WorldCamera {
            view: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, -100.0],
            ],
            proj: default_perspective_mtx(),
            rotated_landscape: false,
            right_sign: 1.0,
            up_sign: 1.0,
        };
        let top = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 480.0));
        let (p_center, depth) = project_point(&cam, [0.0, 0.0, 0.0], top, 1.0).unwrap();
        assert!((p_center.x - 400.0).abs() < 1e-2);
        assert!((p_center.y - 240.0).abs() < 1e-2);
        assert!((depth + 100.0).abs() < 1e-2);

        let (p_uw, _) = project_point(&cam, [20.0, 0.0, 0.0], top, 0.75).unwrap();
        let (p_std, _) = project_point(&cam, [20.0, 0.0, 0.0], top, 1.0).unwrap();
        assert!(((p_uw.x - 400.0) / (p_std.x - 400.0) - 0.75).abs() < 1e-3);

        assert!(project_point(&cam, [0.0, 0.0, 150.0], top, 1.0).is_none());
    }

    #[test]
    fn build_frame_preview_buddy_and_not_in_battle() {
        let mut coop = CoopRuntime::new();
        coop.preview_buddy = true;
        let settings = Settings::default();
        let frame = coop.build_frame(&settings, None);
        assert!(!frame.local_in_battle);
        assert_eq!(frame.peers.len(), 1);
        assert_eq!(frame.peers[0].nickname, "Sapphire Bot");
        assert!(frame.native_npc_peer.is_none());
        assert!(!coop.native_3d_npc_sync);
    }
}
