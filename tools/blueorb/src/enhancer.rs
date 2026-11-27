// Patchs BlueOrb :
// 1. Recompilation incrémentale C (`3dsrecomp`) + compilation pleine vitesse multi-cœurs
// 2. Améliorations Remaster HD Pokémon Saphir Alpha (Upscaling 4x Smooth-xBR, Anisotropie 16x, No Outlines HD)
// 3. Correction complète du clignotement 60 Hz / 60 FPS (désactivation par défaut de l'interpolation + protection des UV yeux/bouches)
// 4. Support Ultra-Wide Hor+ (16:9, 21:9, 32:9) & Double Fenêtre détachable (F7)
// 5. Assistant Premier Lancement ultra-simple (glisser-déposer / choisir la ROM .3ds -> recompilation auto -> lancement direct aux fois suivantes)

use std::fs;
use std::path::Path;

const LAUNCHER_BG_JPG: &[u8] = include_bytes!("launcher_bg.jpg");
const ZAKURO_COOP_RS: &str = include_str!("zakuro_coop.rs");
const ZAKURO_UPDATER_RS: &str = include_str!("zakuro_updater.rs");
const ZAKURO_DISCORD_RS: &str = include_str!("zakuro_discord.rs");

fn read_lf(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|s| s.replace("\r\n", "\n"))
        .map_err(|e| e.to_string())
}

pub fn apply_sdk_enhancements(recomp_repo: &Path, zakuro_repo: &Path) -> Result<(), String> {
    patch_recomp_incremental(recomp_repo)?;
    patch_zakuro_workspace_local_recomp(zakuro_repo, recomp_repo)?;
    patch_zakuro_oras_enhancements(zakuro_repo)?;
    patch_zakuro_core_coop_room(zakuro_repo)?;
    patch_zakuro_gpu_ultrawide_upscaler_antiflicker_and_dual_vulkan(zakuro_repo)?;
    patch_zakuro_frontend_onboarding_console_ui_and_dual_window(zakuro_repo)?;
    Ok(())
}

/// Ensures `zakuro/Cargo.toml` compiles against our locally patched `../3dsrecomp` (incremental + no Windows cmd popups).
fn patch_zakuro_workspace_local_recomp(zakuro_repo: &Path, recomp_repo: &Path) -> Result<(), String> {
    if !recomp_repo.join("Cargo.toml").exists() {
        return Ok(());
    }
    let cargo_toml = zakuro_repo.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Ok(());
    }
    let mut src = read_lf(&cargo_toml).map_err(|e| e.to_string())?;
    if src.contains("BLUEORB_PATCH_LOCAL_RECOMP") {
        return Ok(());
    }
    src.push_str(
        "\n# BLUEORB_PATCH_LOCAL_RECOMP\n[patch.\"https://github.com/fearkov/3dsrecomp\"]\nrecomp3ds = { path = \"../3dsrecomp\" }\nrecomp-abi = { path = \"../3dsrecomp/abi\" }\n",
    );
    fs::write(&cargo_toml, src).map_err(|e| e.to_string())
}

/// Extends `zakuro-core/src/room.rs` with live overworld `PlayerPresence` synchronization alongside local wireless UDS packets.
fn patch_zakuro_core_coop_room(zakuro_repo: &Path) -> Result<(), String> {
    let room_rs = zakuro_repo.join("crates").join("zakuro-core").join("src").join("room.rs");
    if !room_rs.exists() {
        return Ok(());
    }
    let mut src = read_lf(&room_rs).map_err(|e| e.to_string())?;
    if src.contains("BLUEORB_COOP_ROOM_V3") {
        return Ok(());
    }

    if !src.contains("BLUEORB_COOP_ROOM_V1") {
        src = src.replace(
            "use std::collections::VecDeque;",
            "// BLUEORB_COOP_ROOM_V1\nuse std::collections::{HashMap, VecDeque};",
        );
        src = src.replace(
            "#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct Member {",
            "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct PlayerPresence {\n    pub zone_id: u16,\n    pub x: f32,\n    pub y: f32,\n    pub z: f32,\n    pub yaw: f32,\n    pub speed: f32,\n    pub avatar_style: u8,\n    pub in_battle: bool,\n}\n\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct RemotePlayer {\n    pub mac: Mac,\n    pub nickname: String,\n    pub presence: PlayerPresence,\n}\n\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct Member {",
        );
        src = src.replace(
            "    Packet { to: Mac, body: Vec<u8> },\n}",
            "    Packet { to: Mac, body: Vec<u8> },\n    Presence(PlayerPresence),\n}",
        );
        src = src.replace(
            "    Packet { from: Mac, body: Vec<u8> },\n}",
            "    Packet { from: Mac, body: Vec<u8> },\n    PresenceUpdate(RemotePlayer),\n    PresenceLeft(Mac),\n}",
        );
        src = src.replace(
            "    /// last part of the next MAC handed out\n    next: u32,\n}",
            "    /// last part of the next MAC handed out\n    next: u32,\n    presences: HashMap<Mac, PlayerPresence>,\n}",
        );
        src = src.replace(
            "            taken: Vec::new(),\n            next: 1,",
            "            taken: Vec::new(),\n            next: 1,\n            presences: HashMap::new(),",
        );
        src = src.replace(
            "        room.taken.push((Member { nickname: nickname.clone(), mac, game: String::new() }, writer));\n        room.announce();\n        mac",
            "        let existing_presences: Vec<FromRoom> = room\n            .taken\n            .iter()\n            .filter_map(|(m, _)| {\n                room.presences.get(&m.mac).map(|p| FromRoom::PresenceUpdate(RemotePlayer {\n                    mac: m.mac,\n                    nickname: m.nickname.clone(),\n                    presence: p.clone(),\n                }))\n            })\n            .collect();\n        for pmsg in existing_presences {\n            let _ = write(&mut stream, &pmsg);\n        }\n        room.taken.push((Member { nickname: nickname.clone(), mac, game: String::new() }, writer));\n        room.announce();\n        mac",
        );
        src = src.replace(
            "            ToRoom::Hello { .. } => {}\n        }",
            "            ToRoom::Presence(presence) => {\n                room.presences.insert(mac, presence.clone());\n                let sender_nick = room\n                    .taken\n                    .iter()\n                    .find(|(m, _)| m.mac == mac)\n                    .map(|(m, _)| m.nickname.clone())\n                    .unwrap_or_default();\n                let update = FromRoom::PresenceUpdate(RemotePlayer {\n                    mac,\n                    nickname: sender_nick,\n                    presence,\n                });\n                for (member, stream) in &room.taken {\n                    if member.mac != mac {\n                        if let Ok(mut stream) = stream.lock() {\n                            let _ = write(&mut *stream, &update);\n                        }\n                    }\n                }\n            }\n            ToRoom::Hello { .. } => {}\n        }",
        );
        src = src.replace(
            "    if let Ok(mut room) = seats.lock() {\n        room.taken.retain(|(member, _)| member.mac != mac);\n        room.announce();\n    }",
            "    if let Ok(mut room) = seats.lock() {\n        room.taken.retain(|(member, _)| member.mac != mac);\n        room.presences.remove(&mac);\n        let left_msg = FromRoom::PresenceLeft(mac);\n        for (_, stream) in &room.taken {\n            if let Ok(mut stream) = stream.lock() {\n                let _ = write(&mut *stream, &left_msg);\n            }\n        }\n        room.announce();\n    }",
        );
        src = src.replace(
            "    inbox: Mutex<VecDeque<(Mac, Vec<u8>)>>,\n}",
            "    inbox: Mutex<VecDeque<(Mac, Vec<u8>)>>,\n    presences: Mutex<HashMap<Mac, (RemotePlayer, std::time::Instant)>>,\n}",
        );
        src = src.replace(
            "            inbox: Mutex::new(VecDeque::new()),\n        });",
            "            inbox: Mutex::new(VecDeque::new()),\n            presences: Mutex::new(HashMap::new()),\n        });",
        );
        src = src.replace(
            "                inbox: Mutex::new(VecDeque::new()),\n            })",
            "                inbox: Mutex::new(VecDeque::new()),\n                presences: Mutex::new(HashMap::new()),\n            })",
        );
        src = src.replace(
            "                        FromRoom::Welcome { .. } | FromRoom::Refused(_) => {}",
            "                        FromRoom::PresenceUpdate(rp) => {\n                            if let Ok(mut map) = receiving.presences.lock() {\n                                map.insert(rp.mac, (rp, std::time::Instant::now()));\n                            }\n                        }\n                        FromRoom::PresenceLeft(mac) => {\n                            if let Ok(mut map) = receiving.presences.lock() {\n                                map.remove(&mac);\n                            }\n                        }\n                        FromRoom::Welcome { .. } | FromRoom::Refused(_) => {}",
        );
        src = src.replace(
            "    /// the console's way into the room, leaves what came b4 it",
            "    pub fn send_presence(&self, presence: PlayerPresence) {\n        let _ = self.outbox.send(ToRoom::Presence(presence));\n    }\n\n    pub fn remote_players(&self) -> Vec<RemotePlayer> {\n        let Ok(mut map) = self.shared.presences.lock() else {\n            return Vec::new();\n        };\n        map.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(6));\n        map.values().map(|(rp, _)| rp.clone()).collect()\n    }\n\n    /// the console's way into the room, leaves what came b4 it",
        );
        src = src.replace(
            "        assert_eq!(server.members().len(), 1);\n    }\n}",
            "        assert_eq!(server.members().len(), 1);\n    }\n\n    #[test]\n    fn a_room_syncs_overworld_presences() {\n        let server = Server::start(0, \"coop\", \"\", 4).unwrap();\n        let address = format!(\"127.0.0.1:{}\", server.port());\n        let alice = Connection::join(&address, \"Alice\", \"\").unwrap();\n        let bob = Connection::join(&address, \"Bob\", \"\").unwrap();\n\n        alice.send_presence(PlayerPresence {\n            zone_id: 102,\n            x: 150.0,\n            y: 0.0,\n            z: -240.0,\n            yaw: 1.5,\n            speed: 1.0,\n            avatar_style: 3,\n            in_battle: false,\n        });\n        let mut seen = Vec::new();\n        for _ in 0..200 {\n            seen = bob.remote_players();\n            if !seen.is_empty() {\n                break;\n            }\n            std::thread::sleep(Duration::from_millis(5));\n        }\n        assert_eq!(seen.len(), 1);\n        assert_eq!(seen[0].nickname, \"Alice\");\n        assert_eq!(seen[0].presence.zone_id, 102);\n        assert!(alice.remote_players().is_empty());\n    }\n}",
        );
    }

    // Upgrade V1 -> V2: Party Leader (Chef), Zone Warp TP, Battle Sync, Real Party Summary
    if !src.contains("BLUEORB_COOP_ROOM_V2") {
        src = src.replace(
            "// BLUEORB_COOP_ROOM_V1",
            "// BLUEORB_COOP_ROOM_V1\n// BLUEORB_COOP_ROOM_V2",
        );
        src = src.replace(
            "#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct Member {",
            "#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub struct PartyMonSummary {\n    pub level: u8,\n    pub hp: u16,\n    pub max_hp: u16,\n}\n\n#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]\npub enum CoopEvent {\n    ZoneWarp { zone_id: u16, x: f32, y: f32, z: f32, yaw: f32 },\n    BattleStart { zone_id: u16, is_trainer: bool },\n    BattleEnd,\n    PartySync { party_count: u8, mons: Vec<PartyMonSummary> },\n}\n\n#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]\npub struct Member {",
        );
        src = src.replace(
            "    Presence(PlayerPresence),\n}",
            "    Presence(PlayerPresence),\n    SetLeader(Mac),\n    Coop(CoopEvent),\n}",
        );
        src = src.replace(
            "    PresenceLeft(Mac),\n}",
            "    PresenceLeft(Mac),\n    Leader(Option<Mac>),\n    Coop { from: Mac, event: CoopEvent },\n}",
        );
        src = src.replace(
            "    presences: HashMap<Mac, PlayerPresence>,\n}",
            "    presences: HashMap<Mac, PlayerPresence>,\n    leader: Option<Mac>,\n}",
        );
        src = src.replace(
            "            presences: HashMap::new(),",
            "            presences: HashMap::new(),\n            leader: None,",
        );
        src = src.replace(
            "        room.taken.push((Member { nickname: nickname.clone(), mac, game: String::new() }, writer));\n        room.announce();",
            "        let leader_changed = if room.leader.is_none() {\n            room.leader = Some(mac);\n            true\n        } else {\n            false\n        };\n        let _ = write(&mut stream, &FromRoom::Leader(room.leader));\n        room.taken.push((Member { nickname: nickname.clone(), mac, game: String::new() }, writer));\n        if leader_changed {\n            let lmsg = FromRoom::Leader(room.leader);\n            for (_, s) in &room.taken {\n                if let Ok(mut s) = s.lock() {\n                    let _ = write(&mut *s, &lmsg);\n                }\n            }\n        }\n        room.announce();",
        );
        src = src.replace(
            "            ToRoom::Hello { .. } => {}\n        }",
            "            ToRoom::SetLeader(target) => {\n                if room.leader == Some(mac) && room.taken.iter().any(|(m, _)| m.mac == target) {\n                    room.leader = Some(target);\n                    let lmsg = FromRoom::Leader(room.leader);\n                    for (_, stream) in &room.taken {\n                        if let Ok(mut stream) = stream.lock() {\n                            let _ = write(&mut *stream, &lmsg);\n                        }\n                    }\n                }\n            }\n            ToRoom::Coop(event) => {\n                let cmsg = FromRoom::Coop { from: mac, event };\n                for (member, stream) in &room.taken {\n                    if member.mac != mac {\n                        if let Ok(mut stream) = stream.lock() {\n                            let _ = write(&mut *stream, &cmsg);\n                        }\n                    }\n                }\n            }\n            ToRoom::Hello { .. } => {}\n        }",
        );
        src = src.replace(
            "        room.taken.retain(|(member, _)| member.mac != mac);\n        room.presences.remove(&mac);",
            "        room.taken.retain(|(member, _)| member.mac != mac);\n        room.presences.remove(&mac);\n        if room.leader == Some(mac) {\n            room.leader = room.taken.first().map(|(m, _)| m.mac);\n            let lmsg = FromRoom::Leader(room.leader);\n            for (_, stream) in &room.taken {\n                if let Ok(mut stream) = stream.lock() {\n                    let _ = write(&mut *stream, &lmsg);\n                }\n            }\n        }",
        );
        src = src.replace(
            "    presences: Mutex<HashMap<Mac, (RemotePlayer, std::time::Instant)>>,\n}",
            "    presences: Mutex<HashMap<Mac, (RemotePlayer, std::time::Instant)>>,\n    leader: Mutex<Option<Mac>>,\n    coop_events: Mutex<VecDeque<(Mac, CoopEvent)>>,\n}",
        );
        src = src.replace(
            "            presences: Mutex::new(HashMap::new()),\n        });",
            "            presences: Mutex::new(HashMap::new()),\n            leader: Mutex::new(None),\n            coop_events: Mutex::new(VecDeque::new()),\n        });",
        );
        src = src.replace(
            "                presences: Mutex::new(HashMap::new()),\n            })",
            "                presences: Mutex::new(HashMap::new()),\n                leader: Mutex::new(None),\n                coop_events: Mutex::new(VecDeque::new()),\n            })",
        );
        src = src.replace(
            "                        FromRoom::Welcome { .. } | FromRoom::Refused(_) => {}",
            "                        FromRoom::Leader(leader_mac) => {\n                            if let Ok(mut l) = receiving.leader.lock() {\n                                *l = leader_mac;\n                            }\n                        }\n                        FromRoom::Coop { from, event } => {\n                            if let Ok(mut q) = receiving.coop_events.lock() {\n                                if q.len() >= 256 {\n                                    q.pop_front();\n                                }\n                                q.push_back((from, event));\n                            }\n                        }\n                        FromRoom::Welcome { .. } | FromRoom::Refused(_) => {}",
        );
        src = src.replace(
            "    /// the console's way into the room, leaves what came b4 it",
            "    pub fn leader(&self) -> Option<Mac> {\n        self.shared.leader.lock().ok().and_then(|l| *l)\n    }\n\n    pub fn is_leader(&self) -> bool {\n        self.leader() == Some(self.shared.mac)\n    }\n\n    pub fn set_leader(&self, mac: Mac) {\n        let _ = self.outbox.send(ToRoom::SetLeader(mac));\n    }\n\n    pub fn send_coop(&self, event: CoopEvent) {\n        let _ = self.outbox.send(ToRoom::Coop(event));\n    }\n\n    pub fn drain_coop(&self) -> Vec<(Mac, CoopEvent)> {\n        self.shared\n            .coop_events\n            .lock()\n            .map(|mut q| q.drain(..).collect())\n            .unwrap_or_default()\n    }\n\n    /// the console's way into the room, leaves what came b4 it",
        );
        src = src.replace(
            "        assert!(alice.remote_players().is_empty());\n    }\n}",
            "        assert!(alice.remote_players().is_empty());\n    }\n\n    #[test]\n    fn a_room_tracks_leader_and_routes_coop_events() {\n        let server = Server::start(0, \"coop2\", \"\", 4).unwrap();\n        let address = format!(\"127.0.0.1:{}\", server.port());\n        let alice = Connection::join(&address, \"Alice\", \"\").unwrap();\n        let bob = Connection::join(&address, \"Bob\", \"\").unwrap();\n\n        for _ in 0..200 {\n            if alice.is_leader() && bob.leader() == Some(alice.mac()) {\n                break;\n            }\n            std::thread::sleep(Duration::from_millis(5));\n        }\n        assert!(alice.is_leader());\n        assert!(!bob.is_leader());\n\n        alice.send_coop(CoopEvent::ZoneWarp { zone_id: 42, x: 10.0, y: 0.0, z: -20.0, yaw: 0.5 });\n        let mut evs = Vec::new();\n        for _ in 0..200 {\n            evs = bob.drain_coop();\n            if !evs.is_empty() {\n                break;\n            }\n            std::thread::sleep(Duration::from_millis(5));\n        }\n        assert_eq!(evs.len(), 1);\n        assert!(matches!(evs[0].1, CoopEvent::ZoneWarp { zone_id: 42, .. }));\n\n        alice.set_leader(bob.mac());\n        for _ in 0..200 {\n            if bob.is_leader() && !alice.is_leader() {\n                break;\n            }\n            std::thread::sleep(Duration::from_millis(5));\n        }\n        assert!(bob.is_leader());\n\n        drop(bob);\n        for _ in 0..200 {\n            if alice.is_leader() {\n                break;\n            }\n            std::thread::sleep(Duration::from_millis(5));\n        }\n        assert!(alice.is_leader());\n    }\n}",
        );
    }

    // Upgrade V2 -> V3: Cutscene synchronization barrier (`CutsceneState(bool)`)
    if !src.contains("BLUEORB_COOP_ROOM_V3") {
        src = src.replace(
            "// BLUEORB_COOP_ROOM_V2",
            "// BLUEORB_COOP_ROOM_V2\n// BLUEORB_COOP_ROOM_V3",
        );
        src = src.replace(
            "    BattleEnd,\n    PartySync { party_count: u8, mons: Vec<PartyMonSummary> },",
            "    BattleEnd,\n    CutsceneState(bool),\n    PartySync { party_count: u8, mons: Vec<PartyMonSummary> },",
        );
        src = src.replace(
            "        assert_eq!(evs.len(), 1);\n        assert!(matches!(evs[0].1, CoopEvent::ZoneWarp { zone_id: 42, .. }));",
            "        assert_eq!(evs.len(), 1);\n        assert!(matches!(evs[0].1, CoopEvent::ZoneWarp { zone_id: 42, .. }));\n\n        alice.send_coop(CoopEvent::CutsceneState(true));\n        let mut cut_evs = Vec::new();\n        for _ in 0..200 {\n            cut_evs = bob.drain_coop();\n            if !cut_evs.is_empty() {\n                break;\n            }\n            std::thread::sleep(Duration::from_millis(5));\n        }\n        assert_eq!(cut_evs.len(), 1);\n        assert!(matches!(cut_evs[0].1, CoopEvent::CutsceneState(true)));",
        );
    }

    fs::write(&room_rs, src).map_err(|e| e.to_string())
}

/// 1. Rend la compilation C de `3dsrecomp` incrémentale et masque les fenêtres console cmd sous Windows.
fn patch_recomp_incremental(recomp_repo: &Path) -> Result<(), String> {
    let compile_rs = recomp_repo.join("src").join("compile.rs");
    if !compile_rs.exists() {
        return Ok(());
    }
    let mut src = read_lf(&compile_rs).map_err(|e| e.to_string())?;
    if !src.contains("BLUEORB_INCREMENTAL_OBJ") {
        let needle = "                let path = dir.join(source);\n                let mut command = compiler.command();";
        let replacement = r#"                let path = dir.join(source);
                // BLUEORB_INCREMENTAL_OBJ: réutiliser l'objet .o déjà compilé s'il est à jour
                let obj_path = path.with_extension("o");
                let up_to_date = std::fs::metadata(&obj_path)
                    .and_then(|om| Ok((om.len(), om.modified()?)))
                    .and_then(|(olen, otime)| Ok(olen > 64 && otime >= std::fs::metadata(&path)?.modified()?))
                    .unwrap_or(false);
                if up_to_date {
                    if !progress(done.fetch_add(1, Ordering::Relaxed) + 1, sources.len()) {
                        stopped.store(true, Ordering::Relaxed);
                    }
                    let mut state = state.lock().unwrap();
                    state.1 -= needs[index];
                    state.2 -= 1;
                    freed.notify_all();
                    continue;
                }
                let mut command = compiler.command();"#;

        if src.contains(needle) {
            src = src.replace(needle, replacement);
            fs::write(&compile_rs, &src).map_err(|e| e.to_string())?;
        }
    }

    if !src.contains("BLUEORB_WIN_NO_WINDOW") {
        src = src.replace(
            "        let mut command = Command::new(&self.program);\n        command.args(&self.leading);",
            "        // BLUEORB_WIN_NO_WINDOW\n        let mut command = Command::new(&self.program);\n        #[cfg(windows)]\n        {\n            use std::os::windows::process::CommandExt;\n            command.creation_flags(0x0800_0000);\n        }\n        command.args(&self.leading);",
        );
        src = src.replace(
            "    command.creation_flags(BELOW_NORMAL_PRIORITY_CLASS);",
            "    command.creation_flags(BELOW_NORMAL_PRIORITY_CLASS | 0x0800_0000);",
        );
        src = src.replace(
            "    let mut command = Command::new(&ar);",
            "    let mut command = Command::new(&ar);\n    #[cfg(windows)]\n    {\n        use std::os::windows::process::CommandExt;\n        command.creation_flags(0x0800_0000);\n    }",
        );
        fs::write(&compile_rs, src).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 2. Ajoute l'amélioration vérifiée "No Outlines (HD)" pour Pokémon Saphir Alpha USA Rev 2 (`rev3`) et Omega Ruby.
fn patch_zakuro_oras_enhancements(zakuro_repo: &Path) -> Result<(), String> {
    let enh_path = zakuro_repo
        .join("crates")
        .join("zakuro-core")
        .join("src")
        .join("enhancements.txt");
    if !enh_path.exists() {
        return Ok(());
    }
    let src = read_lf(&enh_path).map_err(|e| e.to_string())?;
    if src.contains("No Outlines (HD)") {
        return Ok(());
    }

    let target = "= 000400000011C500 Pokémon Alpha Sapphire (USA)\n60 FPS | rev3 | Reshiban | The whole game runs twice as fast with it.\n28C650C1 00000000";
    let updated = "= 000400000011C500 Pokémon Alpha Sapphire (USA)\n\
No Outlines (HD) | rev3 | BlueOrb | Removes black cel-shading outlines around 3D models for clean HD rendering.\n\
D3000000 00000000\n\
00379EB4 00000000\n\
00379EB8 00000000\n\
D2000000 00000000\n\
60 FPS | rev3 | Reshiban | Runs game logic at 2x speed.\n\
28C650C1 00000000";

    if src.contains(target) {
        fs::write(&enh_path, src.replace(target, updated)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 3. Patchs `zakuro-gpu` :
///    - Correction anti-clignotement 60 FPS dans `interpolation.rs` (ne jamais interpoler les uniformes d'atlas UV yeux/bouches/eau)
///    - Projection 3D Hor+ Ultra-Wide (`16:9`, `21:9`, `32:9`) sans étirement
///    - Upscaler de textures 4x Smooth-xBR/EPX + Mipmaps trilinéaires + Anisotropie 16x
///    - `VulkanPresenter::new_secondary` zéro-copie pour l'écran tactile détaché
fn patch_zakuro_gpu_ultrawide_upscaler_antiflicker_and_dual_vulkan(zakuro_repo: &Path) -> Result<(), String> {
    // 3a. `zakuro-gpu/src/interpolation.rs` : suppression du clignotement 60 Hz sur les textures/yeux/transitions
    let interp_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("interpolation.rs");
    if interp_rs.exists() {
        let mut src = read_lf(&interp_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_ANTIFLICKER_60HZ") {
            // Détecter aussi les pas d'atlas 1/3, 1/6, 1/12, 1/24 (yeux, bouches et sprites Pokémon 3DS) en plus des 1/64 !
            src = src.replace(
                "fn steps(before: f32, now: f32) -> bool {\n    let round = |value: f32| (value * 64.0).fract() == 0.0;\n    round(before) && round(now)\n}",
                "// BLUEORB_ANTIFLICKER_60HZ\nfn steps(before: f32, now: f32) -> bool {\n    let round = |value: f32| {\n        let s64 = value * 64.0;\n        let s24 = value * 24.0;\n        (s64 - s64.round()).abs() < 1e-5 || (s24 - s24.round()).abs() < 1e-5\n    };\n    round(before) && round(now)\n}",
            );
            src = src.replace(
                "        self.moved > 0 && self.matched * 2 >= self.draws && self.jumped * 4 <= self.matched",
                "        self.moved > 0 && self.matched * 4 >= self.draws * 3 && self.jumped * 8 <= self.matched",
            );
            fs::write(&interp_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 3b. `zakuro-gpu/src/lib.rs`
    let gpu_lib_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("lib.rs");
    if gpu_lib_rs.exists() {
        let src = read_lf(&gpu_lib_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_TOP_ASPECT_SCALE") {
            let addition = r#"
static BLUEORB_TOP_ASPECT_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);

pub fn set_top_aspect_scale(top_width: f32) {
    let scale = if top_width > 400.5 { (400.0 / top_width).clamp(0.25, 1.0) } else { 1.0 };
    BLUEORB_TOP_ASPECT_SCALE.store(scale.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub fn top_aspect_scale() -> f32 {
    f32::from_bits(BLUEORB_TOP_ASPECT_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}
"#;
            fs::write(&gpu_lib_rs, format!("{}{}", src, addition)).map_err(|e| e.to_string())?;
        }
    }

    // 3c. `zakuro-gpu/src/raster.rs`
    let raster_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("raster.rs");
    if raster_rs.exists() {
        let src = read_lf(&raster_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_ULTRAWIDE_PLACE") {
            let needle = "fn place(registers: &[u32], shaded: &[Vertex], order: Option<&[usize]>, viewport: (f32, f32, f32, f32)) -> Placement {";
            let replacement = r#"fn place(registers: &[u32], shaded_in: &[Vertex], order: Option<&[usize]>, viewport: (f32, f32, f32, f32)) -> Placement {
    // BLUEORB_ULTRAWIDE_PLACE
    let aspect_scale = crate::top_aspect_scale();
    let uw_storage: Vec<Vertex>;
    let shaded: &[Vertex] = if aspect_scale < 0.999 && (viewport.2 - 240.0).abs() < 1.0 && (viewport.3 - 400.0).abs() < 1.0 {
        let is_fullscreen_2d_quad = shaded_in.len() <= 6
            && shaded_in.iter().all(|v| (v.clip[3] - 1.0).abs() < 1e-3 && v.clip[1].abs() >= 0.98);
        if is_fullscreen_2d_quad {
            shaded_in
        } else {
            uw_storage = shaded_in
                .iter()
                .copied()
                .map(|mut v| {
                    v.clip[1] *= aspect_scale;
                    v
                })
                .collect();
            &uw_storage
        }
    } else {
        shaded_in
    };"#;
            if src.contains(needle) {
                fs::write(&raster_rs, src.replace(needle, replacement)).map_err(|e| e.to_string())?;
            }
        }
    }

    // 3d. `zakuro-gpu/src/shader/glsl.rs`
    let glsl_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("shader").join("glsl.rs");
    if glsl_rs.exists() {
        let src = read_lf(&glsl_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_GLSL_ULTRAWIDE") {
            let needle = "const EPILOGUE: &str = r#\"    if (position.w > 1e-5) {";
            let replacement = r#"const EPILOGUE: &str = r#"    // BLUEORB_GLSL_ULTRAWIDE
    float aspect_scale = uintBitsToFloat(pad[0]);
    if (aspect_scale > 0.1 && aspect_scale < 0.999 && abs(viewport.z - 240.0) < 1.0 && abs(viewport.w - 400.0) < 1.0) {
        if (!(abs(position.w - 1.0) < 1e-3 && abs(position.y) >= 0.98)) {
            position.y *= aspect_scale;
        }
    }
    if (position.w > 1e-5) {"#;
            if src.contains(needle) {
                fs::write(&glsl_rs, src.replace(needle, replacement)).map_err(|e| e.to_string())?;
            }
        }
    }

    // 3e. `zakuro-gpu/src/raster/hardware.rs`
    let hw_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("raster").join("hardware.rs");
    if hw_rs.exists() {
        let mut src = read_lf(&hw_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_HD_UPSCALE") {
            src = src.replace(
                "    max_bias: f32,\n",
                "    max_bias: f32,\n    max_anisotropy: f32,\n",
            );
            src = src.replace(
                "                max_bias: properties.limits.max_sampler_lod_bias,\n",
                "                max_bias: properties.limits.max_sampler_lod_bias,\n                max_anisotropy: if instance.get_physical_device_features(physical).sampler_anisotropy == vk::TRUE { properties.limits.max_sampler_anisotropy.min(16.0) } else { 1.0 },\n",
            );
            src = src.replace(
                "    let features = vk::PhysicalDeviceFeatures::default()\n        .depth_clamp(true)\n        .logic_op(logic_ops)\n        .pipeline_statistics_query(statistics);",
                "    let features = vk::PhysicalDeviceFeatures::default()\n        .depth_clamp(true)\n        .logic_op(logic_ops)\n        .sampler_anisotropy(supported.sampler_anisotropy == vk::TRUE)\n        .pipeline_statistics_query(statistics);",
            );
            src = src.replace(
                "            .min_lod(filtering.levels.0 as f32)\n            .max_lod(filtering.levels.1 as f32);",
                "            .anisotropy_enable(self.max_anisotropy > 1.0 && filtering.magnify && filtering.minify)\n            .max_anisotropy(if self.max_anisotropy > 1.0 && filtering.magnify && filtering.minify { self.max_anisotropy } else { 1.0 })\n            .min_lod(filtering.levels.0 as f32)\n            .max_lod(filtering.levels.1 as f32);",
            );
            src = src.replace(
                "                        None => {\n                            let (view, picture) = self.texture(bound)?;",
                "                        None => {\n                            let (view, picture) = self.texture_hd(bound)?;",
            );
            let target_fn = "    fn texture(&mut self, bound: &BoundTexture) -> Result<(vk::ImageView, Option<u32>), String> {";
            let hd_fn_and_orig = r#"    // BLUEORB_HD_UPSCALE: Upscaling temps-réel 4x Smooth-xBR/EPX + Mipmaps trilinéaires
    fn texture_hd(&mut self, bound: &BoundTexture) -> Result<(vk::ImageView, Option<u32>), String> {
        let size = (bound.width, bound.height);
        if let Some((view, smallest)) = bound.replacement.as_ref().and_then(|material| self.replacement(material, size)) {
            return Ok((view, Some(smallest)));
        }
        let can_upscale = self.scale >= 2
            && bound.levels == 1
            && bound.width >= 8
            && bound.height >= 8
            && bound.width <= 512
            && bound.height <= 512
            && (bound.width * 4) <= self.max_image_size
            && (bound.height * 4) <= self.max_image_size
            && std::env::var_os("BLUEORB_NO_TEX_UPSCALE").is_none();

        if can_upscale {
            let hd_key = (Arc::as_ptr(&bound.texels) as *const u8 as usize) | 1usize;
            if let Some(texture) = self.textures.get_mut(&hd_key) {
                texture.used = self.batch;
                return Ok((texture.image.view, Some(2)));
            }
            let pic = blueorb_upscale_4x_mipmapped(bound.width, bound.height, &bound.texels);
            if let Ok(image) = self.upload_picture(&pic) {
                let view = image.view;
                self.textures.insert(hd_key, Texture { image, _texels: bound.texels.clone(), used: self.batch });
                return Ok((view, Some(2)));
            }
        }
        self.texture(bound)
    }

    fn texture(&mut self, bound: &BoundTexture) -> Result<(vk::ImageView, Option<u32>), String> {"#;
            src = src.replace(target_fn, hd_fn_and_orig);
            src.push_str(UPSCALE_HELPERS_RS);
        }
        if !src.contains("crate::top_aspect_scale().to_bits()") {
            src = src.replace(
                "words.extend([unit.bool_uniforms as u32, unit.entry_point, 0, 0]);",
                "words.extend([unit.bool_uniforms as u32, unit.entry_point, crate::top_aspect_scale().to_bits(), 0]);",
            );
        }
        fs::write(&hw_rs, src).map_err(|e| e.to_string())?;
    }

    // 3f. `zakuro-gpu/src/backend/vulkan.rs`
    let vk_rs = zakuro_repo.join("crates").join("zakuro-gpu").join("src").join("backend").join("vulkan.rs");
    if vk_rs.exists() {
        let mut src = read_lf(&vk_rs).map_err(|e| e.to_string())?;
        if !src.contains("pub fn new_secondary(") {
            src = src.replace(
                "    stale: bool,\n}",
                "    stale: bool,\n    secondary: bool,\n}",
            );
            src = src.replace(
                "            stale: false,\n        };",
                "            stale: false,\n            secondary: false,\n        };",
            );
            src = src.replace(
                "_ => vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO],",
                "_ if self.secondary => vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::FIFO],\n            _ => vec![vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::FIFO],",
            );

            let sec_ctor = r#"    pub fn new_secondary(
        window: &(impl HasDisplayHandle + HasWindowHandle),
        size: (u32, u32),
        shared_dev: Arc<SharedDevice>,
    ) -> Result<VulkanPresenter, PresentError> {
        let entry = shared_dev._entry.clone();
        let instance = shared_dev.instance.clone();
        let physical_device = shared_dev.physical;
        let queue_family = shared_dev.family;
        let device = shared_dev.device.clone();
        let queue = shared_dev.queue;

        let display = window.display_handle().map_err(|e| fail(format!("no display handle: {e}")))?;
        let window_handle = window.window_handle().map_err(|e| fail(format!("no window handle: {e}")))?;
        let surface_instance = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display.as_raw(), window_handle.as_raw(), None)
        }.map_err(vk_fail("creating secondary surface"))?;
        // BLUEORB_DUAL_WIN_VK_SUPPORT_V1
        let supported = unsafe {
            surface_instance.get_physical_device_surface_support(physical_device, queue_family, surface)
        }.map_err(vk_fail("querying secondary surface support"))?;
        if !supported {
            unsafe { surface_instance.destroy_surface(surface, None) };
            return Err(fail("secondary surface is not supported by the active Vulkan queue"));
        }

        let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };
        let swapchain_device = ash::khr::swapchain::Device::new(&instance, &device);
        let surface_format = choose_format(&surface_instance, physical_device, surface)?;
        let render_pass = create_render_pass(&device, surface_format.format)?;
        let descriptor_layout = create_descriptor_layout(&device, 1)?;
        let (pipeline_layout, pipeline) = create_pipeline(&device, render_pass, descriptor_layout, FRAGMENT_SPIRV, CONSTANTS)?;
        let stereo_layout = create_descriptor_layout(&device, 2)?;
        let (stereo_pipeline_layout, stereo_pipeline) = create_pipeline(&device, render_pass, stereo_layout, STEREO_FRAGMENT_SPIRV, STEREO_CONSTANTS)?;
        let srgb = matches!(surface_format.format, vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB);
        let overlay = OverlayPainter::new(&device, render_pass, FRAMES_IN_FLIGHT, srgb)?;

        let sampler = unsafe {
            device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )
        }.map_err(vk_fail("creating secondary sampler"))?;

        let descriptor_pool = unsafe {
            let sets = 6 + 4 * FRAMES_IN_FLIGHT as u32;
            let sizes = [vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                .descriptor_count(6 + 5 * FRAMES_IN_FLIGHT as u32)];
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                    .pool_sizes(&sizes)
                    .max_sets(sets),
                None,
            )
        }.map_err(vk_fail("creating secondary descriptor pool"))?;

        let gpu_sets = unsafe {
            let layouts = [descriptor_layout; 3 * FRAMES_IN_FLIGHT];
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default().descriptor_pool(descriptor_pool).set_layouts(&layouts),
            )
        }.map_err(vk_fail("allocating secondary descriptor sets"))?
        .chunks(3)
        .map(|s| [s[0], s[1], s[2]])
        .collect();

        let stereo_sets = unsafe {
            let layouts = [stereo_layout; FRAMES_IN_FLIGHT];
            device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default().descriptor_pool(descriptor_pool).set_layouts(&layouts),
            )
        }.map_err(vk_fail("allocating secondary stereo sets"))?;

        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }.map_err(vk_fail("creating secondary command pool"))?;

        let command_buffers = unsafe {
            device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(FRAMES_IN_FLIGHT as u32),
            )
        }.map_err(vk_fail("allocating secondary command buffers"))?;

        let mut image_available = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let mut render_finished = Vec::with_capacity(FRAMES_IN_FLIGHT);
        let mut in_flight = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            let sem_info = vk::SemaphoreCreateInfo::default();
            image_available.push(unsafe { device.create_semaphore(&sem_info, None) }.map_err(vk_fail("creating semaphore"))?);
            render_finished.push(unsafe { device.create_semaphore(&sem_info, None) }.map_err(vk_fail("creating semaphore"))?);
            in_flight.push(unsafe {
                device.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED), None)
            }.map_err(vk_fail("creating fence"))?);
        }

        let screens = [
            Screen::new(&device, &memory_properties, descriptor_pool, descriptor_layout, sampler, 400, 240)?,
            Screen::new(&device, &memory_properties, descriptor_pool, descriptor_layout, sampler, 320, 240)?,
            Screen::new(&device, &memory_properties, descriptor_pool, descriptor_layout, sampler, 400, 240)?,
        ];

        let mut presenter = VulkanPresenter {
            _entry: entry,
            instance,
            surface_instance,
            surface,
            physical_device,
            memory_properties,
            device,
            queue_family,
            queue,
            swapchain_device,
            swapchain: vk::SwapchainKHR::null(),
            surface_format,
            extent: vk::Extent2D { width: size.0, height: size.1 },
            image_views: Vec::new(),
            framebuffers: Vec::new(),
            render_pass,
            descriptor_layout,
            descriptor_pool,
            pipeline_layout,
            pipeline,
            stereo_layout,
            stereo_pipeline_layout,
            stereo_pipeline,
            stereo_sets,
            sampler,
            gpu_sets,
            shared: Some(shared_dev),
            waits: false,
            command_pool,
            command_buffers,
            image_available,
            render_finished,
            in_flight,
            frame: 0,
            screens,
            overlay: Some(overlay),
            window: size,
            arrangement: ScreenLayout::BottomOnly,
            orientation: ScreenOrientation::default(),
            top_width: super::TOP_WIDTH,
            filter: ScreenFilter::default(),
            integer: false,
            stereo: Stereo::default(),
            stale: false,
            secondary: true,
        };
        presenter.build_swapchain()?;
        Ok(presenter)
    }

    pub fn new("#;

            src = src.replace("    pub fn new(", sec_ctor);
            fs::write(&vk_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if src.contains("pub fn new_secondary(") && !src.contains("BLUEORB_DUAL_WIN_VK_SUPPORT_V1") {
            src = src.replace(
                "        let surface = unsafe {\n            ash_window::create_surface(&entry, &instance, display.as_raw(), window_handle.as_raw(), None)\n        }.map_err(vk_fail(\"creating secondary surface\"))?;",
                "        let surface = unsafe {\n            ash_window::create_surface(&entry, &instance, display.as_raw(), window_handle.as_raw(), None)\n        }.map_err(vk_fail(\"creating secondary surface\"))?;\n        // BLUEORB_DUAL_WIN_VK_SUPPORT_V1\n        let supported = unsafe {\n            surface_instance.get_physical_device_surface_support(physical_device, queue_family, surface)\n        }.map_err(vk_fail(\"querying secondary surface support\"))?;\n        if !supported {\n            unsafe { surface_instance.destroy_surface(surface, None) };\n            return Err(fail(\"secondary surface is not supported by the active Vulkan queue\"));\n        }",
            );
            fs::write(&vk_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // Preserve overlay DPI / coordinate space when dragging between displays (Retina 2x <-> 1080p 1x)
    let zakuro_gpu = zakuro_repo.join("crates").join("zakuro-gpu");
    let gpu_mod_rs = zakuro_gpu.join("src").join("backend").join("mod.rs");
    if gpu_mod_rs.exists() {
        let mut src = read_lf(&gpu_mod_rs).map_err(|e| e.to_string())?;
        if !src.contains("pub screen_size: [u32; 2]") {
            src = src.replace(
                "    pub free: Vec<u64>,\n}",
                "    pub free: Vec<u64>,\n    pub screen_size: [u32; 2],\n}",
            );
            fs::write(&gpu_mod_rs, src).map_err(|e| e.to_string())?;
        }
    }

    let vk_overlay_rs = zakuro_gpu.join("src").join("backend").join("vulkan_overlay.rs");
    if vk_overlay_rs.exists() {
        let mut src = read_lf(&vk_overlay_rs).map_err(|e| e.to_string())?;
        if !src.contains("overlay.screen_size") {
            src = src.replace(
                "let push = Push { size: [extent.width as f32, extent.height as f32], srgb: self.srgb as u32, _pad: 0 };",
                "let (src_w, src_h) = if overlay.screen_size[0] > 0 && overlay.screen_size[1] > 0 {\n            (overlay.screen_size[0] as f32, overlay.screen_size[1] as f32)\n        } else {\n            (extent.width as f32, extent.height as f32)\n        };\n        let sx = extent.width as f32 / src_w;\n        let sy = extent.height as f32 / src_h;\n        let push = Push { size: [src_w, src_h], srgb: self.srgb as u32, _pad: 0 };",
            );
            src = src.replace(
                "            let [left, top, right, bottom] = mesh.clip;\n            let (right, bottom) = (right.min(extent.width), bottom.min(extent.height));",
                "            let [raw_l, raw_t, raw_r, raw_b] = mesh.clip;\n            let left = ((raw_l as f32 * sx).floor().max(0.0) as u32).min(extent.width);\n            let top = ((raw_t as f32 * sy).floor().max(0.0) as u32).min(extent.height);\n            let right = ((raw_r as f32 * sx).ceil().max(0.0) as u32).min(extent.width);\n            let bottom = ((raw_b as f32 * sy).ceil().max(0.0) as u32).min(extent.height);",
            );
            fs::write(&vk_overlay_rs, src).map_err(|e| e.to_string())?;
        }
    }

    let gl_rs = zakuro_gpu.join("src").join("backend").join("gl.rs");
    if gl_rs.exists() {
        let mut src = read_lf(&gl_rs).map_err(|e| e.to_string())?;
        if !src.contains("overlay.screen_size") {
            src = src.replace(
                "                if let Some(location) = gl.get_uniform_location(self.program, \"size\") {\n                    gl.uniform_2_f32(Some(&location), window.0 as f32, window.1 as f32);\n                }",
                "                let (src_w, src_h) = if overlay.screen_size[0] > 0 && overlay.screen_size[1] > 0 {\n                    (overlay.screen_size[0] as f32, overlay.screen_size[1] as f32)\n                } else {\n                    (window.0 as f32, window.1 as f32)\n                };\n                let sx = window.0 as f32 / src_w;\n                let sy = window.1 as f32 / src_h;\n                if let Some(location) = gl.get_uniform_location(self.program, \"size\") {\n                    gl.uniform_2_f32(Some(&location), src_w, src_h);\n                }",
            );
            src = src.replace(
                "                    let [left, top, right, bottom] = mesh.clip;\n                    let (right, bottom) = (right.min(window.0), bottom.min(window.1));",
                "                    let [raw_l, raw_t, raw_r, raw_b] = mesh.clip;\n                    let left = ((raw_l as f32 * sx).floor().max(0.0) as u32).min(window.0);\n                    let top = ((raw_t as f32 * sy).floor().max(0.0) as u32).min(window.1);\n                    let right = ((raw_r as f32 * sx).ceil().max(0.0) as u32).min(window.0);\n                    let bottom = ((raw_b as f32 * sy).ceil().max(0.0) as u32).min(window.1);",
            );
            fs::write(&gl_rs, src).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// 4. Assistant Premier Lancement ultra-intuitif + Interface Console Large + Double Fenêtre
fn patch_zakuro_frontend_onboarding_console_ui_and_dual_window(zakuro_repo: &Path) -> Result<(), String> {
    let zakuro_crate = zakuro_repo.join("crates").join("zakuro");
    let zakuro_src = zakuro_crate.join("src");

    // 4a-bis. `build.rs` : embed AppIcon.ico into BlueOrb.exe on Windows MSVC when llvm-rc / rc is available
    let build_rs = zakuro_crate.join("build.rs");
    if build_rs.exists() {
        let src = read_lf(&build_rs).map_err(|e| e.to_string())?;
        if !src.contains("C:/Program Files/LLVM/bin/llvm-rc.exe") {
            let win_icon_hook = r#"
    // BLUEORB_WIN_ICON
    if windows && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
        let ico = manifest_dir.join("AppIcon.ico");
        println!("cargo:rerun-if-changed={}", ico.display());
        if ico.is_file() {
            if let Ok(out_dir) = std::env::var("OUT_DIR") {
                let out = std::path::PathBuf::from(out_dir);
                let rc_file = out.join("blueorb_icon.rc");
                let res_file = out.join("blueorb_icon.res");
                let ico_escaped = ico.display().to_string().replace('\\', "/");
                let _ = std::fs::write(&rc_file, format!("1 ICON \"{ico_escaped}\"\n"));
                let try_llvm_rc = |bin: &str| {
                    std::process::Command::new(bin)
                        .args(["/FO"])
                        .arg(&res_file)
                        .arg(&rc_file)
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                };
                let compiled = try_llvm_rc("llvm-rc")
                    || try_llvm_rc("C:/Program Files/LLVM/bin/llvm-rc.exe")
                    || std::process::Command::new("rc")
                        .args(["/nologo", "/fo"])
                        .arg(&res_file)
                        .arg(&rc_file)
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                if compiled && res_file.is_file() {
                    println!("cargo:rustc-link-arg-bins={}", res_file.display());
                }
            }
        }
    }
}"#;
            let insert_at = src
                .find("\n    // BLUEORB_WIN_ICON")
                .or_else(|| src.rfind('}'));
            if let Some(pos) = insert_at {
                let mut patched = src[..pos].to_string();
                patched.push_str(win_icon_hook);
                patched.push('\n');
                fs::write(&build_rs, patched).map_err(|e| e.to_string())?;
            }
        }
    }

    // 4a. `main.rs` : no black console window on Windows + offer to move into /Applications on macOS
    let main_rs = zakuro_src.join("main.rs");
    if main_rs.exists() {
        let mut src = read_lf(&main_rs).map_err(|e| e.to_string())?;
        if !src.contains("windows_subsystem") {
            src = format!("#![cfg_attr(target_os = \"windows\", windows_subsystem = \"windows\")]\n{src}");
        }
        if !src.contains("BLUEORB_MOVE_TO_APPLICATIONS") {
            src = src.replace(
                "    zakuro::run(None);",
                "    #[cfg(target_os = \"macos\")]\n    blueorb_offer_move_to_applications();\n    zakuro::run(None);",
            );
            src.push_str(MOVE_TO_APPLICATIONS_RS);
        }
        fs::write(&main_rs, src).map_err(|e| e.to_string())?;
    }

    // 4b. `recompile.rs` : compiler à pleine vitesse sur les cœurs Performance (4x-5x plus rapide au 1er lancement !)
    let recompile_rs = zakuro_src.join("recompile.rs");
    if recompile_rs.exists() {
        let src = read_lf(&recompile_rs).map_err(|e| e.to_string())?;
        if src.contains("background: true") {
            fs::write(&recompile_rs, src.replace("background: true", "background: false"))
                .map_err(|e| e.to_string())?;
        }
    }

    // 4c. `gui.rs` : Thème sombre sobre & lisible + synchro DPI multi-écran
    let gui_rs = zakuro_src.join("gui.rs");
    if gui_rs.exists() {
        let mut src = read_lf(&gui_rs).map_err(|e| e.to_string())?;
        if !src.contains("BLUEORB_CONSOLE_THEME") {
            src = src.replace(
                "        ctx.set_fonts(fonts(system_fonts()));",
                "        ctx.set_fonts(fonts(system_fonts()));\n        apply_blueorb_console_theme(&ctx);",
            );
            src.push_str(CONSOLE_THEME_FN_RS);
            fs::write(&gui_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_OVERLAY_DPI_SYNC") {
            src = src.replace(
                "        let input = self.state.take_egui_input(window);\n        let mut output = self.ctx.run_ui(input, ui);\n        self.state.handle_platform_output(window, output.platform_output);\n        let pixels_per_point = output.pixels_per_point;\n        let size = window.inner_size();",
                "        // BLUEORB_OVERLAY_DPI_SYNC\n        let mut input = self.state.take_egui_input(window);\n        let scale = window.scale_factor() as f32;\n        let size = window.inner_size();\n        if scale > 0.0 && size.width > 0 && size.height > 0 {\n            if let Some(vp) = input.viewports.get_mut(&ViewportId::ROOT) {\n                vp.native_pixels_per_point = Some(scale);\n            }\n            input.screen_rect = Some(egui::Rect::from_min_size(\n                egui::Pos2::ZERO,\n                egui::vec2(size.width as f32 / scale, size.height as f32 / scale),\n            ));\n        }\n        let mut output = self.ctx.run_ui(input, ui);\n        self.state.handle_platform_output(window, output.platform_output);\n        let pixels_per_point = output.pixels_per_point;",
            );
            src = src.replace(
                "Overlay { textures, meshes, free }",
                "Overlay { textures, meshes, free, screen_size: [size.width, size.height] }",
            );
            fs::write(&gui_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4d. `settings.rs` : `blueorb_rom`, valeurs par défaut optimales (Français, 5x 1440p, No Outlines HD, 0 clignotement)
    let settings_rs = zakuro_src.join("settings.rs");
    if settings_rs.exists() {
        let mut src = read_lf(&settings_rs).map_err(|e| e.to_string())?;
        if !src.contains("ZAKURO_SETTINGS_PATH") {
            src = src.replace(
                "fn path() -> Option<PathBuf> {",
                "fn path() -> Option<PathBuf> {\n    if let Some(custom) = std::env::var_os(\"ZAKURO_SETTINGS_PATH\").filter(|v| !v.is_empty()) {\n        return Some(PathBuf::from(custom));\n    }",
            );
        }
        if !src.contains("blueorb_rom") {
            src = src.replace(
                "    pub games: Option<PathBuf>,",
                "    pub games: Option<PathBuf>,\n    pub blueorb_rom: Option<PathBuf>,",
            );
            src = src.replace(
                "            games: None,\n            language: Language::English,\n            renderer: Renderer::Vulkan,\n            hardware_rasterizer: true,\n            resolution: 3,",
                "            games: None,\n            blueorb_rom: None,\n            language: Language::English,\n            renderer: Renderer::Vulkan,\n            hardware_rasterizer: true,\n            resolution: 5,",
            );
            src = src.replace(
                "            filter: Filter::Smooth,",
                "            filter: Filter::Sharp,",
            );
            src = src.replace(
                "            enhancements: std::collections::BTreeMap::new(),",
                "            enhancements: std::collections::BTreeMap::from([(\"000400000011C500\".to_owned(), vec![\"No Outlines (HD)\".to_owned()])]),",
            );
        }
        if !src.contains("SeparateWindows") {
            src = src.replace(
                "    LargeTop,\n}",
                "    LargeTop,\n    SeparateWindows,\n}",
            );
            src = src.replace(
                "pub const ALL: [Screens; 5] = [Screens::Stacked, Screens::SideBySide, Screens::LargeTop, Screens::TopOnly, Screens::BottomOnly];",
                "pub const ALL: [Screens; 6] = [Screens::Stacked, Screens::SideBySide, Screens::LargeTop, Screens::SeparateWindows, Screens::TopOnly, Screens::BottomOnly];",
            );
            src = src.replace(
                "Screens::LargeTop => \"Big top screen, small bottom one\",",
                "Screens::LargeTop => \"Big top screen, small bottom one\",\n            Screens::SeparateWindows => \"Separate windows\",",
            );
            src = src.replace(
                "Screens::LargeTop => ScreenLayout::LargeTop,",
                "Screens::LargeTop => ScreenLayout::LargeTop,\n            Screens::SeparateWindows => ScreenLayout::TopOnly,",
            );
        }
        if !src.contains("Wide32x9") {
            src = src.replace(
                "    Wide21x9,\n}",
                "    Wide21x9,\n    Wide32x9,\n}",
            );
            src = src.replace(
                "pub const ALL: [TopShape; 3] = [TopShape::Console, TopShape::Wide16x9, TopShape::Wide21x9];",
                "pub const ALL: [TopShape; 4] = [TopShape::Console, TopShape::Wide16x9, TopShape::Wide21x9, TopShape::Wide32x9];",
            );
            src = src.replace(
                "TopShape::Wide21x9 => \"21:9\",",
                "TopShape::Wide21x9 => \"21:9 Ultrawide\",\n            TopShape::Wide32x9 => \"32:9 Super Ultrawide\",",
            );
            src = src.replace(
                "TopShape::Wide21x9 => 560.0,",
                "TopShape::Wide21x9 => 560.0,\n            TopShape::Wide32x9 => 240.0 * 32.0 / 9.0,",
            );
        }
        if !src.contains("coop_enabled") {
            src = src.replace(
                "    pub room_port: u16,\n}",
                "    pub room_port: u16,\n    #[serde(default = \"blueorb_true\")]\n    pub coop_enabled: bool,\n    #[serde(default = \"blueorb_true\")]\n    pub coop_show_hud: bool,\n    #[serde(default = \"blueorb_true\")]\n    pub coop_show_names: bool,\n    #[serde(default)]\n    pub coop_avatar_style: u8,\n}",
            );
            src = src.replace(
                "            room_port: zakuro_core::room::DEFAULT_PORT,\n        }",
                "            room_port: zakuro_core::room::DEFAULT_PORT,\n            coop_enabled: true,\n            coop_show_hud: true,\n            coop_show_names: true,\n            coop_avatar_style: 0,\n        }",
            );
            src = src.replace(
                "fn or_default<'de, D, T>(deserializer: D)",
                "fn blueorb_true() -> bool { true }\n\nfn or_default<'de, D, T>(deserializer: D)",
            );
        }
        if !src.contains("coop_follow_leader") {
            src = src.replace(
                "    #[serde(default)]\n    pub coop_avatar_style: u8,\n}",
                "    #[serde(default)]\n    pub coop_avatar_style: u8,\n    #[serde(default = \"blueorb_true\")]\n    pub coop_follow_leader: bool,\n    #[serde(default = \"blueorb_true\")]\n    pub coop_double_battle: bool,\n    #[serde(default)]\n    pub coop_force_unlock: bool,\n}",
            );
            src = src.replace(
                "            coop_avatar_style: 0,\n        }",
                "            coop_avatar_style: 0,\n            coop_follow_leader: true,\n            coop_double_battle: true,\n            coop_force_unlock: false,\n        }",
            );
        }
        if !src.contains("coop_sync_cutscenes") {
            src = src.replace(
                "    #[serde(default)]\n    pub coop_force_unlock: bool,\n}",
                "    #[serde(default)]\n    pub coop_force_unlock: bool,\n    #[serde(default = \"blueorb_true\")]\n    pub coop_sync_cutscenes: bool,\n}",
            );
            src = src.replace(
                "            coop_force_unlock: false,\n        }",
                "            coop_force_unlock: false,\n            coop_sync_cutscenes: true,\n        }",
            );
        }
        if !src.contains("auto_update") {
            src = src.replace(
                "    #[serde(default = \"blueorb_true\")]\n    pub coop_sync_cutscenes: bool,\n}",
                "    #[serde(default = \"blueorb_true\")]\n    pub coop_sync_cutscenes: bool,\n    #[serde(default = \"blueorb_true\")]\n    pub auto_update: bool,\n}",
            );
            src = src.replace(
                "            coop_sync_cutscenes: true,\n        }",
                "            coop_sync_cutscenes: true,\n            auto_update: true,\n        }",
            );
        }
        if !src.contains("discord_rpc") {
            src = src.replace(
                "    #[serde(default = \"blueorb_true\")]\n    pub auto_update: bool,\n}",
                "    #[serde(default = \"blueorb_true\")]\n    pub auto_update: bool,\n    #[serde(default = \"blueorb_true\")]\n    pub discord_rpc: bool,\n    #[serde(default)]\n    pub discord_client_id: String,\n}",
            );
            src = src.replace(
                "            auto_update: true,\n        }",
                "            auto_update: true,\n            discord_rpc: true,\n            discord_client_id: String::new(),\n        }",
            );
        }
        if !src.contains("fast_forward_speed") {
            src = src.replace(
                "    pub frame_interpolation: Option<bool>,",
                "    pub frame_interpolation: Option<bool>,\n    pub fast_forward_speed: Option<u32>,",
            );
            src = src.replace(
                "    pub frame_interpolation: bool,",
                "    pub frame_interpolation: bool,\n    #[serde(default = \"blueorb_default_ff_speed\")]\n    pub fast_forward_speed: u32,",
            );
            src = src.replace(
                "            frame_interpolation: false,",
                "            frame_interpolation: false,\n            fast_forward_speed: 3,",
            );
            src = src.replace(
                "fn blueorb_true() -> bool { true }",
                "fn blueorb_true() -> bool { true }\nfn blueorb_default_ff_speed() -> u32 { 3 }",
            );
            src = src.replace(
                "        settings.frame_interpolation = self.frame_interpolation.unwrap_or(settings.frame_interpolation);",
                "        settings.frame_interpolation = self.frame_interpolation.unwrap_or(settings.frame_interpolation);\n        settings.fast_forward_speed = self.fast_forward_speed.unwrap_or(settings.fast_forward_speed);",
            );
        }
        fs::write(&settings_rs, src).map_err(|e| e.to_string())?;
    }

    // 4e. `menus.rs`, `coop.rs` & `updater.rs` : Assistant d'accueil + Auto-Updater + Co-op Multijoueur + Menu Pause accessible (`game`)
    fs::write(zakuro_src.join("launcher_bg.jpg"), LAUNCHER_BG_JPG).map_err(|e| e.to_string())?;
    fs::write(zakuro_src.join("coop.rs"), ZAKURO_COOP_RS).map_err(|e| e.to_string())?;
    let build_commit = std::env::var("GITHUB_SHA")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "dev".to_owned());
    fs::write(
        zakuro_src.join("updater.rs"),
        ZAKURO_UPDATER_RS.replace("__BLUEORB_BUILD_COMMIT__", build_commit.trim()),
    )
    .map_err(|e| e.to_string())?;
    fs::write(zakuro_src.join("discord.rs"), ZAKURO_DISCORD_RS).map_err(|e| e.to_string())?;
    let menus_rs = zakuro_src.join("menus.rs");
    if menus_rs.exists() {
        let mut src = read_lf(&menus_rs).map_err(|e| e.to_string())?;
        if src.contains("BLUEORB_WIDE_CONSOLE_MENU") && !src.contains("BLUEORB_LAUNCHER_UI_V3") {
            let _ = std::process::Command::new("git")
                .args(["-C", &zakuro_repo.to_string_lossy(), "checkout", "--", "crates/zakuro/src/menus.rs"])
                .status();
            src = read_lf(&menus_rs).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_WIDE_CONSOLE_MENU") {
            src = src.replace(
                "    CheatsFolder,\n}",
                "    CheatsFolder,\n    PickRomFile,\n    CleanResiduals,\n    InstallToApplications,\n    ToggleDualWindows,\n    SetLayout(Screens),\n    SetTopShape(TopShape),\n    OpenMultiplayer,\n    JoinLanRoom(String),\n    ToggleCoopPreview,\n}",
            );
            src = src.replace(
                "    pub menu_open: bool,",
                "    pub menu_open: bool,\n    pub tab: u8,",
            );

            // Remplacement de `Menus::library` par l'Assistant Premier Lancement ultra-accessible
            let lib_start = "    /// the library, filling the window, waiting are the games waiting for their turn to recompile\n    pub fn library(";
            let game_start = "    /// what goes over a running game, the menu when it's open and the frame rate when asked for\n    #[allow(clippy::too_many_arguments)]\n    pub fn game(";
            let Some(i0) = src.find(lib_start) else {
                return Err("Impossible de localiser `Menus::library` dans `menus.rs`".into());
            };
            let Some(i1) = src.find(game_start) else {
                return Err("Impossible de localiser `Menus::game` dans `menus.rs`".into());
            };
            src.replace_range(i0..i1, ONBOARDING_LIBRARY_FN_RS);

            let old_game_fn = r#"    /// what goes over a running game, the menu when it's open and the frame rate when asked for
    #[allow(clippy::too_many_arguments)]
    pub fn game(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        program_id: Option<u64>,
        fps: Option<f32>,
        recompiled: bool,
        fast: bool,
        jobs: &[Job],
        waiting: usize,
        saved: Option<&str>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let ctx = ui.ctx().clone();
        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2500) {
                egui::Area::new(egui::Id::new("toast"))
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 8.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(text);
                        });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }
        if fast {
            egui::Area::new(egui::Id::new("fast forward"))
                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-8.0, 8.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new("» fast forward").monospace());
                    });
                });
        }
        if let Some(fps) = fps {
            // clicks go thru to the screen under it
            egui::Area::new(egui::Id::new("fps")).fixed_pos(egui::pos2(8.0, 8.0)).interactable(false).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(RichText::new(format!("{fps:.0} fps")).monospace());
                });
            });
        }
        if !self.menu_open {
            return actions;
        }
        egui::Window::new("Paused")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(&ctx, |ui| {
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new(if recompiled { "running its recompiled code" } else { "interpreted" }).weak());
                ui.add_space(6.0);
                let wide = |ui: &mut egui::Ui, text: &str| ui.add_sized([200.0, 28.0], egui::Button::new(text)).clicked();
                if wide(ui, "Resume") {
                    actions.push(Action::Resume);
                }
                if wide(ui, "Reset") {
                    actions.push(Action::Reset);
                }
                if wide(ui, "Fullscreen") {
                    actions.push(Action::Fullscreen);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut self.slot).range(1..=9).prefix("Slot "));
                    ui.label(RichText::new(saved.unwrap_or("empty")).weak());
                });
                ui.horizontal(|ui| {
                    if ui.add_sized([98.0, 28.0], egui::Button::new("Save state")).on_hover_text("F5 saves to this slot while playing.").clicked() {
                        actions.push(Action::SaveState);
                    }
                    if ui
                        .add_enabled(saved.is_some(), egui::Button::new("Load state").min_size(Vec2::new(98.0, 28.0)))
                        .on_hover_text("F8 loads this slot while playing.")
                        .clicked()
                    {
                        actions.push(Action::LoadState);
                    }
                });
                ui.add_space(4.0);
                if wide(ui, "Settings") {
                    self.settings_open = true;
                }
                if let Some(program_id) = program_id.filter(|_| wide(ui, "Settings for this game")) {
                    self.game_settings = Some((program_id, name.to_owned()));
                }
                if wide(ui, "Cheats") {
                    self.cheats_open = true;
                }
                if wide(ui, "Multiplayer") {
                    self.multiplayer_open = true;
                }
                if ui
                    .add_sized([200.0, 28.0], egui::Button::new("Show the camera a picture"))
                    .on_hover_text("Pick a picture for the console's cameras to see, a QR code's say, in place of the computer's camera.")
                    .clicked()
                {
                    actions.push(Action::ChooseCameraPicture);
                }
                if wide(ui, "Copy info for a report") {
                    actions.push(Action::CopyReport);
                }
                if wide(ui, "Back to the library") {
                    actions.push(Action::Library);
                }
                if wide(ui, "Quit") {
                    actions.push(Action::Quit);
                }
                for job in jobs.iter().filter(|job| !job.stage().finished()) {
                    ui.separator();
                    ui.label(&job.name);
                    progress(ui, job);
                }
                if waiting > 0 {
                    ui.label(RichText::new(format!("{waiting} more waiting to recompile")).weak());
                }
            });
        actions
    }"#;

            if !src.contains(old_game_fn) {
                return Err("Impossible de localiser le corps de `Menus::game` dans `menus.rs`".into());
            }
            src = src.replace(old_game_fn, WIDE_CONSOLE_GAME_MENU_RS);
            src.push_str(WIDE_CONSOLE_WIDGET_HELPERS_RS);
            fs::write(&menus_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_COOP_ACTIONS_V2") {
            src = src.replace(
                "    ToggleCoopPreview,\n}",
                "    // BLUEORB_COOP_ACTIONS_V2\n    ToggleCoopPreview,\n    SetCoopLeader(zakuro_core::room::Mac),\n    TeleportToLeader,\n}",
            );
            fs::write(&menus_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_UPDATER_UI_V1") {
            src = src.replace(
                "    TeleportToLeader,\n}",
                "    // BLUEORB_UPDATER_UI_V1\n    TeleportToLeader,\n    CheckUpdates,\n    InstallUpdate,\n}",
            );
            src = src.replace(
                "    pub room_password: String,",
                "    pub room_password: String,\n    pub update_status: crate::updater::UpdateStatus,",
            );
            src = src.replace(
                "                        if show_install_apps && launcher_glass_button(ui, \"Install to /Applications\", true) {\n                            actions.push(Action::InstallToApplications);\n                        }\n                    });",
                "                        if show_install_apps && launcher_glass_button(ui, \"Install to /Applications\", true) {\n                            actions.push(Action::InstallToApplications);\n                        }\n                        if let Some(upd_act) = crate::updater::draw_launcher_update_bar(ui, &self.update_status) {\n                            actions.push(upd_act);\n                        }\n                    });",
            );
            src = src.replace(
                "                if console_secondary_button(ui, width, 32.0, \"Clean Residual Build Files…\", true) {\n                    actions.push(Action::CleanResiduals);\n                }",
                "                if console_secondary_button(ui, width, 32.0, \"Clean Residual Build Files…\", true) {\n                    actions.push(Action::CleanResiduals);\n                }\n                ui.add_space(10.0);\n                console_card(ui, width, |ui, _| {\n                    actions.extend(crate::updater::draw_pause_update_section(ui, &self.update_status, &mut settings.auto_update));\n                });",
            );
            fs::write(&menus_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_DISCORD_UI_V1") {
            src = src.replace(
                "    pub update_status: crate::updater::UpdateStatus,",
                "    // BLUEORB_DISCORD_UI_V1\n    pub update_status: crate::updater::UpdateStatus,\n    pub discord_state: crate::discord::RpcConnectionState,",
            );
            src = src.replace(
                "                console_card(ui, width, |ui, _| {\n                    actions.extend(crate::updater::draw_pause_update_section(ui, &self.update_status, &mut settings.auto_update));\n                });",
                "                console_card(ui, width, |ui, _| {\n                    actions.extend(crate::updater::draw_pause_update_section(ui, &self.update_status, &mut settings.auto_update));\n                });\n                ui.add_space(10.0);\n                console_card(ui, width, |ui, _| {\n                    crate::discord::draw_discord_rpc_section(ui, &mut settings.discord_rpc, &mut settings.discord_client_id, self.discord_state);\n                });",
            );
            fs::write(&menus_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("fast_forward_seg") {
            src = src.replace(
                "                    if own.frame_interpolation.is_some() {\n                        own.frame_interpolation = Some(settings.frame_interpolation);\n                    }",
                "                    if own.frame_interpolation.is_some() {\n                        own.frame_interpolation = Some(settings.frame_interpolation);\n                    }\n                    if own.fast_forward_speed.is_some() {\n                        own.fast_forward_speed = Some(settings.fast_forward_speed);\n                    }",
            );
            src = src.replace(
                "                    console_toggle_row(\n                        ui,\n                        cw,\n                        \"Frame Interpolation (60 Hz)\",\n                        \"Interpolates slow camera motion between 30 FPS frames.\",\n                        &mut settings.frame_interpolation,\n                    );",
                "                    console_toggle_row(\n                        ui,\n                        cw,\n                        \"Frame Interpolation (60 Hz)\",\n                        \"Interpolates slow camera motion between 30 FPS frames.\",\n                        &mut settings.frame_interpolation,\n                    );\n\n                    ui.add_space(8.0);\n                    ui.separator();\n                    ui.add_space(6.0);\n                    ui.label(RichText::new(\"Fast Forward Max Speed (Tab)\").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));\n                    ui.add_space(6.0);\n                    let ff_opts = [\n                        (2u32, \"2x\"),\n                        (3, \"3x (Default)\"),\n                        (4, \"4x\"),\n                        (6, \"6x\"),\n                    ];\n                    if let Some(ff) = console_segmented(ui, cw, 32.0, settings.fast_forward_speed.clamp(2, 6), &ff_opts, \"fast_forward_seg\") {\n                        settings.fast_forward_speed = ff;\n                    }",
            );
            fs::write(&menus_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_MULTI_NO_PAUSE_V1") {
            src = src.replace(
                "    TeleportToLeader,\n",
                "    // BLUEORB_MULTI_NO_PAUSE_V1\n    TeleportToLeader,\n    TeleportToPeer(zakuro_core::room::Mac),\n",
            );
            src = src.replace(
                "                    if console_secondary_button(ui, half, 32.0, \"Multiplayer & Co-op (F4)\", true) {\n                        self.multiplayer_open = true;\n                    }",
                "                    if console_secondary_button(ui, half, 32.0, \"Multiplayer & Co-op (F4)\", true) {\n                        self.multiplayer_open = true;\n                        self.menu_open = false;\n                    }",
            );
            fs::write(&menus_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_MAC_HINTS_V1") {
            src = src.replace(
                "// BLUEORB_MULTI_NO_PAUSE_V1",
                "// BLUEORB_MULTI_NO_PAUSE_V1\n    // BLUEORB_MAC_HINTS_V1",
            );
            src = src.replace(
                "                        launcher_key_hint(ui, \"F4\", \"Multiplayer\");\n                        launcher_key_hint(ui, \"F6\", \"Ultrawide\");\n                        launcher_key_hint(ui, \"F7\", \"Dual Screen\");\n                        launcher_key_hint(ui, \"F11\", \"Fullscreen\");",
                "                        launcher_key_hint(ui, if cfg!(target_os = \"macos\") { \"⌘4\" } else { \"F4\" }, \"Multiplayer\");\n                        launcher_key_hint(ui, if cfg!(target_os = \"macos\") { \"⌘6\" } else { \"F6\" }, \"Ultrawide\");\n                        launcher_key_hint(ui, if cfg!(target_os = \"macos\") { \"⌘7\" } else { \"F7\" }, \"Dual Screen\");\n                        launcher_key_hint(ui, if cfg!(target_os = \"macos\") { \"⌘⇧F\" } else { \"F11\" }, \"Fullscreen\");",
            );
            src = src.replace(
                "(Screens::SeparateWindows, \"Dual Window (F7)\", \"Detach touch screen into its own window\", 3),",
                "(Screens::SeparateWindows, if cfg!(target_os = \"macos\") { \"Dual Window (⌘7)\" } else { \"Dual Window (F7)\" }, \"Detach touch screen into its own window\", 3),",
            );
            src = src.replace(
                "console_secondary_button(ui, half, 32.0, \"Multiplayer & Co-op (F4)\", true)",
                "console_secondary_button(ui, half, 32.0, if cfg!(target_os = \"macos\") { \"Multiplayer & Co-op (⌘4)\" } else { \"Multiplayer & Co-op (F4)\" }, true)",
            );
            fs::write(&menus_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4f. `present.rs`
    let present_rs = zakuro_src.join("present.rs");
    if present_rs.exists() {
        let mut src = read_lf(&present_rs).map_err(|e| e.to_string())?;
        let sec_fn_resilient = r#"    pub fn create_secondary(
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
        shared: Option<std::sync::Arc<SharedDevice>>,
    ) -> Result<(Window, Backend), String> {
        // BLUEORB_DUAL_WIN_PRESENT_V1
        if let Some(shared_dev) = shared {
            let vk_attempt = event_loop
                .create_window(attributes.clone())
                .map_err(|e| e.to_string())
                .and_then(|window| {
                    let size = window.inner_size();
                    match VulkanPresenter::new_secondary(&window, (size.width.max(1), size.height.max(1)), shared_dev) {
                        Ok(mut presenter) => {
                            presenter.set_layout(ScreenLayout::BottomOnly);
                            Ok((window, Backend::Vulkan(presenter)))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                });
            match vk_attempt {
                Ok(res) => return Ok(res),
                Err(vk_err) => {
                    log::warn!("secondary Vulkan window failed ({vk_err}), falling back to OpenGL");
                }
            }
        }
        let (window, mut backend) = create_opengl(event_loop, attributes.clone())
            .or_else(|gl_err| {
                log::warn!("secondary OpenGL window failed ({gl_err}), trying standalone Vulkan");
                create_vulkan(event_loop, attributes)
            })?;
        backend.set_layout(ScreenLayout::BottomOnly);
        Ok((window, backend))
    }

    pub fn shares_vulkan_device(&self) -> bool {
        matches!(self, Backend::Vulkan(p) if p.shared_device().is_some())
    }

    pub fn create("#;
        let mut changed = false;
        if !src.contains("pub fn create_secondary(") {
            src = src.replace("    pub fn create(", sec_fn_resilient);
            changed = true;
        }
        if !src.contains("BLUEORB_DUAL_WIN_PRESENT_V1") {
            let old_sec_fn = r#"    pub fn create_secondary(
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
        shared: Option<std::sync::Arc<SharedDevice>>,
    ) -> Result<(Window, Backend), String> {
        if let Some(shared_dev) = shared {
            let window = event_loop.create_window(attributes).map_err(|e| e.to_string())?;
            let size = window.inner_size();
            let mut presenter = VulkanPresenter::new_secondary(&window, (size.width.max(1), size.height.max(1)), shared_dev)
                .map_err(|e| e.to_string())?;
            presenter.set_layout(ScreenLayout::BottomOnly);
            return Ok((window, Backend::Vulkan(presenter)));
        }
        let (window, mut backend) = create_opengl(event_loop, attributes)?;
        backend.set_layout(ScreenLayout::BottomOnly);
        Ok((window, backend))
    }

    pub fn create("#;
            if src.contains(old_sec_fn) {
                src = src.replace(old_sec_fn, sec_fn_resilient);
            }
            src = src.replace(
                "            Backend::OpenGl {\n                presenter,\n                context,\n                surface,\n            } => {\n                presenter.resize(width, height);",
                "            Backend::OpenGl {\n                presenter,\n                context,\n                surface,\n            } => {\n                let _ = context.make_current(surface);\n                presenter.resize(width, height);",
            );
            src = src.replace(
                "            Backend::OpenGl {\n                presenter,\n                context,\n                surface,\n            } => {\n                presenter.present(top, bottom, right, overlay)?;",
                "            Backend::OpenGl {\n                presenter,\n                context,\n                surface,\n            } => {\n                context.make_current(surface).map_err(|e| PresentError::Backend(e.to_string()))?;\n                presenter.present(top, bottom, right, overlay)?;",
            );
            changed = true;
        }
        if changed {
            fs::write(&present_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4g. `lib.rs` : auto-boot si déjà recompilé, recompilation auto dès sélection/glisser-déposer de la ROM, puis lancement auto !
    let lib_rs = zakuro_src.join("lib.rs");
    if lib_rs.exists() {
        let mut src = read_lf(&lib_rs).map_err(|e| e.to_string())?;
        if !src.contains("bottom_window") {
            src = src.replace(
                "        layout: settings.layout,",
                "        layout: settings.layout,\n        last_combined_layout: if settings.layout == Screens::SeparateWindows { Screens::Stacked } else { settings.layout },",
            );
            src = src.replace(
                "        backend: None,\n        window: None,",
                "        backend: None,\n        window: None,\n        bottom_backend: None,\n        bottom_window: None,\n        bottom_cursor: (0.0, 0.0),\n        bottom_mouse_down: false,\n        f7_held: false,\n        last_f7_press: None,\n        last_dual_toggle: None,",
            );
            src = src.replace(
                "    backend: Option<Backend>,\n    window: Option<Window>,",
                "    backend: Option<Backend>,\n    window: Option<Window>,\n    bottom_backend: Option<Backend>,\n    bottom_window: Option<Window>,\n    bottom_cursor: (f32, f32),\n    bottom_mouse_down: bool,\n    f7_held: bool,\n    last_f7_press: Option<Instant>,\n    last_dual_toggle: Option<Instant>,",
            );
            src = src.replace(
                "    layout: Screens,",
                "    layout: Screens,\n    last_combined_layout: Screens,",
            );

            // Toujours activer le profil matériel New 3DS pour Pokémon Saphir Alpha
            src = src.replace(
                "            new3ds: self.options.new3ds,",
                "            new3ds: true,",
            );

            // Démarrage intelligent : si une ROM a déjà été configurée, lancer tout de suite le jeu (ou reprendre la recompilation)
            let start_needle = r#"        } else if let Some(rom) = self.options.rom.clone() {
            if let Err(error) = self.play(Path::new(&rom)) {
                eprintln!("zakuro: {error}");
                std::process::exit(1);
            }
        }"#;
            let start_rep = r#"        } else if let Some(rom) = self.options.rom.clone() {
            if let Err(error) = self.play(Path::new(&rom)) {
                eprintln!("zakuro: {error}");
                std::process::exit(1);
            }
        } else if let Some(rom) = self.settings.blueorb_rom.clone().filter(|p| p.is_file()) {
            self.accept_rom_file(rom);
        }"#;
            src = src.replace(start_needle, start_rep);

            let set_layout_needle = "    fn set_layout(&mut self, layout: Screens) {";
            let set_layout_rep = r#"    fn accept_rom_file(&mut self, path: PathBuf) {
        let title = match zakuro_fs::Title::load(&path) {
            Ok(t) => t,
            Err(err) => {
                self.menus.message = Some(format!(
                    "Could not open ROM ({err}).\n\nPlease select a decrypted Nintendo 3DS ROM (.3ds, .cci, or .cxi)."
                ));
                return;
            }
        };
        let program_id = title.program_id();
        self.settings.blueorb_rom = Some(path.clone());
        if let Some(parent) = path.parent() {
            if self.settings.games.is_none() {
                self.settings.games = Some(parent.to_path_buf());
            }
        }
        let tid_hex = format!("{program_id:016X}");
        if (program_id == 0x0004_0000_0011_C500 || program_id == 0x0004_0000_0011_C400)
            && !self.settings.enhancements.contains_key(&tid_hex)
        {
            self.settings.enhancements.insert(tid_hex, vec!["No Outlines (HD)".to_owned()]);
        }
        self.settings.save();

        let already_recompiled = self.linked.is_some_and(|l| l.program_id() == program_id)
            || zakuro_core::recompiled::installed(program_id).is_some();

        if already_recompiled {
            self.apply_official_rom_icon(&path);
            if let Err(error) = self.play(&path) {
                self.menus.message = Some(error);
            }
        } else {
            self.menus.menu_open = false;
            self.game = None;
            self.recompile(vec![Wanted { program_id, path }], true);
        }
    }

    fn apply_official_rom_icon(&self, rom_path: &Path) {
        let Ok(title) = zakuro_fs::Title::load(rom_path) else { return };
        let Some(smdh) = title.exefs_file("icon").and_then(library::Smdh::parse) else { return };
        if smdh.icon.len() != 48 * 48 * 4 {
            return;
        }
        let rgba64 = blueorb_upscale_smdh_rgba(&smdh.icon, 64);
        if let Ok(win_icon) = winit::window::Icon::from_rgba(rgba64, 64, 64) {
            if let Some(w) = &self.window {
                w.set_window_icon(Some(win_icon.clone()));
            }
            if let Some(bw) = &self.bottom_window {
                bw.set_window_icon(Some(win_icon));
            }
        }
        let icon48 = smdh.icon;
        let game_title = if smdh.name.is_empty() {
            "Pokémon Alpha Sapphire".to_owned()
        } else {
            smdh.name
        };
        std::thread::spawn(move || {
            blueorb_persist_official_bundle_icon(&icon48, &game_title);
        });
    }

    /// Removes everything BlueOrb generated outside the app (native library, build cache, downloaded toolchain).
    /// Save files are never touched.
    fn clean_residual_files(&mut self) {
        let confirm = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Clean Up Residual Files")
            .set_description(
                "This removes the recompiled native library, the C build cache and the downloaded compiler.\n\nYour save files are kept. The game will be recompiled the next time you launch it.",
            )
            .set_buttons(rfd::MessageButtons::OkCancel)
            .show();
        if confirm != rfd::MessageDialogResult::Ok {
            return;
        }
        if self.game.is_some() {
            self.back_to_library();
        }

        fn size_of(path: &Path) -> u64 {
            let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
            if !meta.is_dir() {
                return meta.len();
            }
            std::fs::read_dir(path)
                .map(|entries| entries.flatten().map(|e| size_of(&e.path())).sum())
                .unwrap_or(0)
        }
        let mut freed = 0u64;
        let mut remove = |path: &Path| {
            if !path.exists() {
                return;
            }
            let bytes = size_of(path);
            let gone = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            match gone {
                Ok(()) => freed += bytes,
                Err(error) => log::warn!("could not remove {}: {error}", path.display()),
            }
        };

        let mut ids: Vec<u64> = vec![0x0004_0000_0011_C500, 0x0004_0000_0011_C400];
        if let Some(rom) = self.settings.blueorb_rom.as_ref() {
            if let Ok(title) = zakuro_fs::Title::load(rom) {
                ids.push(title.program_id());
            }
        }
        ids.extend(self.library.games.iter().map(|game| game.program_id));
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            if let Some(lib) = zakuro_core::recompiled::installed(id) {
                remove(&lib);
            }
            remove(&recomp3ds::build::cache_dir(id));
        }
        if let Some(tools) = zig::tools_dir(self.data_dir.as_deref()) {
            remove(&tools);
        }

        self.menus.toast = Some((
            format!("Removed {:.1} MB of residual files", freed as f64 / (1024.0 * 1024.0)),
            Instant::now(),
        ));
    }

    fn toggle_dual_windows(&mut self) {
        let target = if self.active.layout == Screens::SeparateWindows {
            self.last_combined_layout
        } else {
            self.last_combined_layout = self.active.layout;
            Screens::SeparateWindows
        };
        self.set_layout(target);
        self.menus.toast = Some((
            if target == Screens::SeparateWindows {
                "Dual Window: On".to_owned()
            } else {
                "Dual Window: Off".to_owned()
            },
            Instant::now(),
        ));
    }

    fn set_top_shape(&mut self, shape: settings::TopShape) {
        let own = self.program_id().and_then(|id| self.settings.per_game.get_mut(&settings::game_key(id)));
        match own.filter(|o| o.top_shape.is_some()) {
            Some(o) => o.top_shape = Some(shape),
            None => self.settings.top_shape = shape,
        }
        self.settings.save();
        self.active = self.settings.for_game(self.program_id());
        self.apply_layout();
        self.menus.toast = Some((format!("Aspect Ratio: {}", shape.name()), Instant::now()));
    }

    fn set_layout(&mut self, layout: Screens) {
        if layout != Screens::SeparateWindows {
            self.last_combined_layout = layout;
        }"#;
            src = src.replace(set_layout_needle, set_layout_rep);

            // Allow `next_recompile` to compile a single `.3ds` file directly without folder scanning
            let next_recomp_needle = r#"        if let Some(game) = self.library.games.iter().find(|game| game.path == wanted.path) {
            let update = game.update.as_ref().map(|update| update.path.as_path());
            self.jobs.push(Job::start(&game.path, update, game.program_id, &game.name, self.data_dir.as_deref(), toolchain));
        }"#;
            let next_recomp_rep = r#"        if let Some(game) = self.library.games.iter().find(|game| game.path == wanted.path) {
            let update = game.update.as_ref().map(|update| update.path.as_path());
            self.jobs.push(Job::start(&game.path, update, game.program_id, &game.name, self.data_dir.as_deref(), toolchain));
        } else if wanted.path.is_file() {
            let addon_upd = library::addons_beside(&wanted.path).and_then(|(u, _, _)| u.map(|x| x.path));
            let gname = wanted
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Pokémon Alpha Sapphire".to_owned());
            self.jobs.push(Job::start(
                &wanted.path,
                addon_upd.as_deref(),
                wanted.program_id,
                &gname,
                self.data_dir.as_deref(),
                toolchain,
            ));
        }"#;
            src = src.replace(next_recomp_needle, next_recomp_rep);

            // As soon as initial recompilation finishes (`Stage::Done`), apply the official Alpha Sapphire icon and auto-launch!
            let poll_done_needle = "                Stage::Done => {\n                    self.library.recompiled(program_id);";
            let poll_done_rep = r#"                Stage::Done => {
                    self.library.recompiled(program_id);
                    if self.game.is_none() {
                        if let Some(rom) = self.settings.blueorb_rom.clone().filter(|p| p.is_file()) {
                            self.apply_official_rom_icon(&rom);
                            if let Err(error) = self.play(&rom) {
                                self.menus.message = Some(error);
                            }
                            continue;
                        }
                    }"#;
            src = src.replace(poll_done_needle, poll_done_rep);

            let win_ev_needle = "    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {";
            let win_ev_rep = r#"    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.bottom_window.as_ref().is_some_and(|w| w.id() == id) {
            match event {
                WindowEvent::CloseRequested => {
                    self.bottom_mouse_down = false;
                    self.keyboard.touch(None);
                    let restore = self.last_combined_layout;
                    self.set_layout(restore);
                }
                WindowEvent::Focused(false) => {
                    self.bottom_mouse_down = false;
                    self.keyboard.release();
                }
                WindowEvent::Resized(size) => {
                    if let Some(bot) = &mut self.bottom_backend {
                        bot.resize(size.width, size.height);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => self.key(event, false),
                WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                    self.bottom_mouse_down = state == ElementState::Pressed && self.game.is_some() && !self.menus.menu_open;
                    self.touch_bottom_window();
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.bottom_cursor = (position.x as f32, position.y as f32);
                    self.touch_bottom_window();
                }
                _ => {}
            }
            return;
        }"#;
            src = src.replace(win_ev_needle, win_ev_rep);

            // Glisser-déposer direct d'un fichier .3ds sur la fenêtre principale !
            src = src.replace(
                "            WindowEvent::RedrawRequested => self.step(event_loop),",
                "            WindowEvent::DroppedFile(path) => {\n                if self.game.is_none() || self.menus.menu_open {\n                    self.accept_rom_file(path);\n                }\n            }\n            WindowEvent::RedrawRequested => self.step(event_loop),",
            );

            let key_needle = "                KeyCode::F9 => self.set_layout(self.active.layout.next()),";
            let key_rep = r#"                KeyCode::F6 => {
                    let shapes = settings::TopShape::ALL;
                    let idx = shapes.iter().position(|&s| s == self.active.top_shape).unwrap_or(0);
                    self.set_top_shape(shapes[(idx + 1) % shapes.len()]);
                }
                KeyCode::F7 => self.toggle_dual_windows(),
                KeyCode::F9 => self.set_layout(self.active.layout.next()),"#;
            src = src.replace(key_needle, key_rep);

            let touch_needle = "    fn touch(&mut self) {\n        let Some(window) = &self.window else { return };";
            let touch_rep = r#"    fn touch_bottom_window(&mut self) {
        let Some(window) = &self.bottom_window else { return };
        let size = window.inner_size();
        let (_, bottom) = layout(size.width, size.height, zakuro_gpu::ScreenLayout::BottomOnly, self.orientation.screen_orientation(), 400.0, self.active.integer_scale);
        let Some(bottom) = bottom else {
            self.keyboard.touch(None);
            return;
        };
        let x = self.bottom_cursor.0 - bottom.x;
        let y = self.bottom_cursor.1 - bottom.y;
        let inside = x >= 0.0 && y >= 0.0 && x < bottom.width && y < bottom.height;
        self.keyboard.touch((self.bottom_mouse_down && inside).then(|| {
            let (u, v) = (x / bottom.width, y / bottom.height);
            let (u, v) = match self.orientation {
                Orientation::North => (u, v),
                Orientation::East => (v, 1.0 - u),
                Orientation::South => (1.0 - u, 1.0 - v),
                Orientation::West => (1.0 - v, u),
            };
            (((u * 320.0) as u16).min(319), ((v * 240.0) as u16).min(239))
        }));
    }

    fn touch(&mut self) {
        if self.layout == Screens::SeparateWindows {
            return;
        }
        let Some(window) = &self.window else { return };"#;
            src = src.replace(touch_needle, touch_rep);

            let apply_needle = "    fn apply_layout(&mut self) {\n        if let Some(backend) = &mut self.backend {";
            let apply_rep = r#"    fn apply_layout(&mut self) {
        zakuro_gpu::set_top_aspect_scale(self.active.top_shape.width());
        if let Some(bot) = &mut self.bottom_backend {
            bot.set_scaling(self.active.filter.screens(), self.active.integer_scale);
            bot.set_orientation(self.orientation.screen_orientation());
        }
        if let Some(backend) = &mut self.backend {"#;
            src = src.replace(apply_needle, apply_rep);

            src = src.replace(
                "backend.set_top_width(self.active.top_shape.width());",
                "zakuro_gpu::set_top_aspect_scale(self.active.top_shape.width());\n                backend.set_top_width(self.active.top_shape.width());",
            );

            src = src.replace(
                "            Action::Quit => event_loop.exit(),",
                r#"            Action::PickRomFile => {
                let picked = rfd::FileDialog::new()
                    .set_title("Select Decrypted Nintendo 3DS ROM (.3ds)")
                    .add_filter("Nintendo 3DS ROM (.3ds, .cci, .cxi)", &["3ds", "cci", "cxi"])
                    .pick_file();
                if let Some(file) = picked {
                    self.accept_rom_file(file);
                }
            }
            Action::CleanResiduals => self.clean_residual_files(),
            Action::InstallToApplications => {
                #[cfg(target_os = "macos")]
                blueorb_move_bundle_to_applications_now();
            }
            Action::ToggleDualWindows => self.toggle_dual_windows(),
            Action::SetLayout(layout) => self.set_layout(layout),
            Action::SetTopShape(shape) => self.set_top_shape(shape),
            Action::Quit => event_loop.exit(),"#,
            );

            src = src.replace(
                "Some(game) => format!(\"Zakuro - {} - {}\", game.name, game.system.status_line()),\n                    None => \"Zakuro\".to_owned(),",
                "Some(game) => game.name.clone(),\n                    None => \"BlueOrb\".to_owned(),",
            );
            src = src.replace(
                "Window::default_attributes().with_title(\"Zakuro\")",
                "Window::default_attributes().with_title(\"BlueOrb\").with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))",
            );

            src = src.replace(
                "        let cheats = self.game.as_ref().filter(|_| self.menus.cheats_open).map(|game| {",
                "        let cheats = self.game.as_ref().filter(|_| self.menus.cheats_open || self.menus.menu_open).map(|game| {",
            );
            src = src.replace(
                "        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
                "        let (active_layout, active_top_shape) = (self.active.layout, self.active.top_shape);\n        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
            );
            src = src.replace(
                "menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref())",
                "menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref(), settings, cheats.as_deref(), active_layout, active_top_shape)",
            );

            let present_needle = "    fn present(&mut self, event_loop: &ActiveEventLoop) {\n        let overlay = self.interface(event_loop);";
            let present_rep = r#"    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {
        // BLUEORB_DUAL_WIN_FULLSCREEN_V2
        let wants_second = self.layout == Screens::SeparateWindows && self.game.is_some();
        if wants_second && self.bottom_window.is_none() {
            let main_full = self.window.as_ref().is_some_and(|w| w.fullscreen().is_some());
            let main_mon = self.window.as_ref().and_then(|w| w.current_monitor());
            let other_mon = main_mon.as_ref().and_then(|cur| {
                event_loop.available_monitors().find(|m| m.position() != cur.position())
            });
            let pip_mode = main_full && other_mon.is_none();
            let bot_scale = if pip_mode { 2 } else { self.scale.clamp(2, 4) };
            let bot_size = winit::dpi::LogicalSize::new(320 * bot_scale, 240 * bot_scale);
            let mut attrs = Window::default_attributes()
                .with_title("BlueOrb — Touch Screen")
                .with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))
                .with_inner_size(bot_size);
            if pip_mode {
                attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
            let shared = self.backend.as_ref().and_then(Backend::shared_device);
            match Backend::create_secondary(event_loop, attrs, shared) {
                Ok((win, mut bot_backend)) => {
                    bot_backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
                    bot_backend.set_orientation(self.orientation.screen_orientation());
                    if pip_mode {
                        win.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
                    }
                    if let Some(main_win) = &self.window {
                        let bot_outer = win.outer_size();
                        let sf = main_win.scale_factor();
                        let bot_w = if bot_outer.width > 0 {
                            bot_outer.width as i32
                        } else {
                            ((320 * bot_scale) as f64 * sf).round() as i32
                        };
                        let bot_h = if bot_outer.height > 0 {
                            bot_outer.height as i32
                        } else {
                            ((240 * bot_scale) as f64 * sf).round() as i32
                        };
                        if let Some(mon) = &main_mon {
                            let mpos = mon.position();
                            let msize = mon.size();
                            let mon_left = mpos.x;
                            let mon_top = mpos.y;
                            let mon_right = mpos.x + msize.width as i32;
                            let mon_bottom = mpos.y + msize.height as i32;
                            let pos = main_win.outer_position().unwrap_or(mpos);
                            let main_w = main_win.outer_size().width as i32;
                            let target_pos = if pip_mode {
                                winit::dpi::PhysicalPosition::new(
                                    (mon_right - bot_w - 24).clamp(mon_left, (mon_right - bot_w).max(mon_left)),
                                    (mon_bottom - bot_h - 48).clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else if !main_full && !main_win.is_maximized() && pos.x + main_w + 16 + bot_w <= mon_right {
                                winit::dpi::PhysicalPosition::new(
                                    pos.x + main_w + 16,
                                    pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else if let Some(other) = &other_mon {
                                let opos = other.position();
                                winit::dpi::PhysicalPosition::new(opos.x + 48, opos.y + 48)
                            } else if !main_full && !main_win.is_maximized() && pos.x - bot_w - 16 >= mon_left {
                                winit::dpi::PhysicalPosition::new(
                                    pos.x - bot_w - 16,
                                    pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else {
                                winit::dpi::PhysicalPosition::new(
                                    (mon_right - bot_w - 24).clamp(mon_left, (mon_right - bot_w).max(mon_left)),
                                    (mon_bottom - bot_h - 48).clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            };
                            win.set_outer_position(target_pos);
                        } else if let Ok(pos) = main_win.outer_position() {
                            win.set_outer_position(winit::dpi::PhysicalPosition::new(
                                (pos.x + 48).max(0),
                                (pos.y + 48).max(0),
                            ));
                        }
                    }
                    win.set_visible(true);
                    if !pip_mode {
                        win.focus_window();
                    }
                    self.bottom_window = Some(win);
                    self.bottom_backend = Some(bot_backend);
                }
                Err(err) => {
                    log::warn!("could not open secondary touch window: {err}");
                    self.menus.toast = Some((format!("Dual Window unavailable: {err}"), Instant::now()));
                    let restore = self.last_combined_layout;
                    self.set_layout(restore);
                }
            }
        } else if !wants_second && self.bottom_window.is_some() {
            self.bottom_mouse_down = false;
            self.bottom_backend = None;
            self.bottom_window = None;
        }
    }

    fn present(&mut self, event_loop: &ActiveEventLoop) {
        let overlay = self.interface(event_loop);
        self.sync_bottom_window(event_loop);"#;
            src = src.replace(present_needle, present_rep);

            let present_end_needle = r#"        match result {
            Ok(()) | Err(PresentError::OutOfDate) => {}
            Err(error) => {
                log::error!("presentation failed: {error}");
                event_loop.exit();
            }
        }
    }"#;
            let present_end_rep = r#"        match result {
            Ok(()) | Err(PresentError::OutOfDate) => {}
            Err(error) => {
                log::error!("presentation failed: {error}");
                event_loop.exit();
            }
        }
        let main_shared = self.backend.as_ref().and_then(Backend::shared_device);
        if let Some(bot_backend) = &mut self.bottom_backend {
            let empty_px: Vec<u8> = Vec::new();
            let empty_overlay = Overlay::default();
            let same_vk_device = bot_backend.shares_vulkan_device()
                && match (&main_shared, bot_backend.shared_device()) {
                    (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, &b),
                    _ => false,
                };
            let _ = bot_backend.present(
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                ScreenImage { width: bottom_width, height: bottom_height, pixels: &bottom, gpu: if same_vk_device { bottom_gpu } else { None } },
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                &empty_overlay,
            );
        }
    }"#;
            src = src.replace(present_end_needle, present_end_rep);
            src.push_str(OFFICIAL_ICON_UPDATER_RS);

            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if src.contains("fn sync_bottom_window(") && !src.contains("BLUEORB_DUAL_WIN_FULLSCREEN_V2") {
            let old_sync_v0 = r#"    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {
        let wants_second = self.layout == Screens::SeparateWindows && self.game.is_some();
        if wants_second && self.bottom_window.is_none() {
            let bot_scale = self.scale.clamp(2, 4);
            let bot_size = winit::dpi::LogicalSize::new(320 * bot_scale, 240 * bot_scale);
            let attrs = Window::default_attributes()
                .with_title("BlueOrb — Touch Screen")
                .with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))
                .with_inner_size(bot_size);
            let shared = self.backend.as_ref().and_then(Backend::shared_device);
            match Backend::create_secondary(event_loop, attrs, shared) {
                Ok((win, mut bot_backend)) => {
                    bot_backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
                    bot_backend.set_orientation(self.orientation.screen_orientation());
                    if let Some(main_win) = &self.window {
                        if let Ok(pos) = main_win.outer_position() {
                            let main_sz = main_win.outer_size();
                            win.set_outer_position(winit::dpi::PhysicalPosition::new(
                                pos.x + (main_sz.width as i32) + 16,
                                pos.y,
                            ));
                        }
                    }
                    self.bottom_window = Some(win);
                    self.bottom_backend = Some(bot_backend);
                }
                Err(err) => log::warn!("could not open secondary touch window: {err}"),
            }
        } else if !wants_second && self.bottom_window.is_some() {
            self.bottom_mouse_down = false;
            self.bottom_backend = None;
            self.bottom_window = None;
        }
    }"#;
            let old_sync_v1 = r#"    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {
        // BLUEORB_DUAL_WIN_SYNC_V1
        let wants_second = self.layout == Screens::SeparateWindows && self.game.is_some();
        if wants_second && self.bottom_window.is_none() {
            let bot_scale = self.scale.clamp(2, 4);
            let bot_size = winit::dpi::LogicalSize::new(320 * bot_scale, 240 * bot_scale);
            let attrs = Window::default_attributes()
                .with_title("BlueOrb — Touch Screen")
                .with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))
                .with_inner_size(bot_size);
            let shared = self.backend.as_ref().and_then(Backend::shared_device);
            match Backend::create_secondary(event_loop, attrs, shared) {
                Ok((win, mut bot_backend)) => {
                    bot_backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
                    bot_backend.set_orientation(self.orientation.screen_orientation());
                    if let Some(main_win) = &self.window {
                        let bot_outer = win.outer_size();
                        let sf = main_win.scale_factor();
                        let bot_w = if bot_outer.width > 0 {
                            bot_outer.width as i32
                        } else {
                            ((320 * bot_scale) as f64 * sf).round() as i32
                        };
                        let bot_h = if bot_outer.height > 0 {
                            bot_outer.height as i32
                        } else {
                            ((240 * bot_scale) as f64 * sf).round() as i32
                        };
                        if let Ok(pos) = main_win.outer_position() {
                            let main_sz = main_win.outer_size();
                            let main_w = main_sz.width as i32;
                            if let Some(mon) = main_win.current_monitor() {
                                let mpos = mon.position();
                                let msize = mon.size();
                                let mon_left = mpos.x;
                                let mon_top = mpos.y;
                                let mon_right = mpos.x + msize.width as i32;
                                let mon_bottom = mpos.y + msize.height as i32;
                                let target_pos = if pos.x + main_w + 16 + bot_w <= mon_right {
                                    winit::dpi::PhysicalPosition::new(
                                        pos.x + main_w + 16,
                                        pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                    )
                                } else if let Some(other) = event_loop
                                    .available_monitors()
                                    .find(|m| m.position() != mpos)
                                {
                                    let opos = other.position();
                                    winit::dpi::PhysicalPosition::new(opos.x + 48, opos.y + 48)
                                } else if pos.x - bot_w - 16 >= mon_left {
                                    winit::dpi::PhysicalPosition::new(
                                        pos.x - bot_w - 16,
                                        pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                    )
                                } else {
                                    winit::dpi::PhysicalPosition::new(
                                        (pos.x + 48).clamp(mon_left, (mon_right - bot_w).max(mon_left)),
                                        (pos.y + 48).clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                    )
                                };
                                win.set_outer_position(target_pos);
                            } else {
                                win.set_outer_position(winit::dpi::PhysicalPosition::new(
                                    (pos.x + 48).max(0),
                                    (pos.y + 48).max(0),
                                ));
                            }
                        }
                    }
                    win.set_visible(true);
                    win.focus_window();
                    self.bottom_window = Some(win);
                    self.bottom_backend = Some(bot_backend);
                }
                Err(err) => {
                    log::warn!("could not open secondary touch window: {err}");
                    self.menus.toast = Some((format!("Dual Window unavailable: {err}"), Instant::now()));
                    let restore = self.last_combined_layout;
                    self.set_layout(restore);
                }
            }
        } else if !wants_second && self.bottom_window.is_some() {
            self.bottom_mouse_down = false;
            self.bottom_backend = None;
            self.bottom_window = None;
        }
    }"#;
            let new_sync = r#"    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {
        // BLUEORB_DUAL_WIN_FULLSCREEN_V2
        let wants_second = self.layout == Screens::SeparateWindows && self.game.is_some();
        if wants_second && self.bottom_window.is_none() {
            let main_full = self.window.as_ref().is_some_and(|w| w.fullscreen().is_some());
            let main_mon = self.window.as_ref().and_then(|w| w.current_monitor());
            let other_mon = main_mon.as_ref().and_then(|cur| {
                event_loop.available_monitors().find(|m| m.position() != cur.position())
            });
            let pip_mode = main_full && other_mon.is_none();
            let bot_scale = if pip_mode { 2 } else { self.scale.clamp(2, 4) };
            let bot_size = winit::dpi::LogicalSize::new(320 * bot_scale, 240 * bot_scale);
            let mut attrs = Window::default_attributes()
                .with_title("BlueOrb — Touch Screen")
                .with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))
                .with_inner_size(bot_size);
            if pip_mode {
                attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
            }
            let shared = self.backend.as_ref().and_then(Backend::shared_device);
            match Backend::create_secondary(event_loop, attrs, shared) {
                Ok((win, mut bot_backend)) => {
                    bot_backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
                    bot_backend.set_orientation(self.orientation.screen_orientation());
                    if pip_mode {
                        win.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
                    }
                    if let Some(main_win) = &self.window {
                        let bot_outer = win.outer_size();
                        let sf = main_win.scale_factor();
                        let bot_w = if bot_outer.width > 0 {
                            bot_outer.width as i32
                        } else {
                            ((320 * bot_scale) as f64 * sf).round() as i32
                        };
                        let bot_h = if bot_outer.height > 0 {
                            bot_outer.height as i32
                        } else {
                            ((240 * bot_scale) as f64 * sf).round() as i32
                        };
                        if let Some(mon) = &main_mon {
                            let mpos = mon.position();
                            let msize = mon.size();
                            let mon_left = mpos.x;
                            let mon_top = mpos.y;
                            let mon_right = mpos.x + msize.width as i32;
                            let mon_bottom = mpos.y + msize.height as i32;
                            let pos = main_win.outer_position().unwrap_or(mpos);
                            let main_w = main_win.outer_size().width as i32;
                            let target_pos = if pip_mode {
                                winit::dpi::PhysicalPosition::new(
                                    (mon_right - bot_w - 24).clamp(mon_left, (mon_right - bot_w).max(mon_left)),
                                    (mon_bottom - bot_h - 48).clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else if !main_full && !main_win.is_maximized() && pos.x + main_w + 16 + bot_w <= mon_right {
                                winit::dpi::PhysicalPosition::new(
                                    pos.x + main_w + 16,
                                    pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else if let Some(other) = &other_mon {
                                let opos = other.position();
                                winit::dpi::PhysicalPosition::new(opos.x + 48, opos.y + 48)
                            } else if !main_full && !main_win.is_maximized() && pos.x - bot_w - 16 >= mon_left {
                                winit::dpi::PhysicalPosition::new(
                                    pos.x - bot_w - 16,
                                    pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else {
                                winit::dpi::PhysicalPosition::new(
                                    (mon_right - bot_w - 24).clamp(mon_left, (mon_right - bot_w).max(mon_left)),
                                    (mon_bottom - bot_h - 48).clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            };
                            win.set_outer_position(target_pos);
                        } else if let Ok(pos) = main_win.outer_position() {
                            win.set_outer_position(winit::dpi::PhysicalPosition::new(
                                (pos.x + 48).max(0),
                                (pos.y + 48).max(0),
                            ));
                        }
                    }
                    win.set_visible(true);
                    if !pip_mode {
                        win.focus_window();
                    }
                    self.bottom_window = Some(win);
                    self.bottom_backend = Some(bot_backend);
                }
                Err(err) => {
                    log::warn!("could not open secondary touch window: {err}");
                    self.menus.toast = Some((format!("Dual Window unavailable: {err}"), Instant::now()));
                    let restore = self.last_combined_layout;
                    self.set_layout(restore);
                }
            }
        } else if !wants_second && self.bottom_window.is_some() {
            self.bottom_mouse_down = false;
            self.bottom_backend = None;
            self.bottom_window = None;
        }
    }"#;
            src = src.replace(old_sync_v1, new_sync);
            src = src.replace(old_sync_v0, new_sync);
            let old_bot_present = r#"        if let Some(bot_backend) = &mut self.bottom_backend {
            let empty_px: Vec<u8> = Vec::new();
            let empty_overlay = Overlay::default();
            let _ = bot_backend.present(
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                ScreenImage { width: bottom_width, height: bottom_height, pixels: &bottom, gpu: bottom_gpu },
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                &empty_overlay,
            );
        }"#;
            let new_bot_present = r#"        let main_shared = self.backend.as_ref().and_then(Backend::shared_device);
        if let Some(bot_backend) = &mut self.bottom_backend {
            let empty_px: Vec<u8> = Vec::new();
            let empty_overlay = Overlay::default();
            let same_vk_device = bot_backend.shares_vulkan_device()
                && match (&main_shared, bot_backend.shared_device()) {
                    (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, &b),
                    _ => false,
                };
            let _ = bot_backend.present(
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                ScreenImage { width: bottom_width, height: bottom_height, pixels: &bottom, gpu: if same_vk_device { bottom_gpu } else { None } },
                ScreenImage { width: 400, height: 240, pixels: &empty_px, gpu: None },
                &empty_overlay,
            );
        }"#;
            src = src.replace(old_bot_present, new_bot_present);
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_TOGGLE_FULLSCREEN_DUAL_V1") {
            src = src.replace(
                "                WindowEvent::Focused(false) => {\n                    self.bottom_mouse_down = false;\n                    self.keyboard.release();\n                }",
                "                WindowEvent::Focused(false) => {\n                    self.bottom_mouse_down = false;\n                    self.fast_forward_key = false;\n                    self.keyboard.release();\n                }",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_DUAL_WIN_FIX_V2") {
            if !src.contains("f7_held:") {
                src = src.replace(
                    "        bottom_mouse_down: false,",
                    "        bottom_mouse_down: false,\n        f7_held: false,\n        last_f7_press: None,\n        last_dual_toggle: None,",
                );
                src = src.replace(
                    "    bottom_mouse_down: bool,",
                    "    bottom_mouse_down: bool,\n    f7_held: bool,\n    last_f7_press: Option<Instant>,\n    last_dual_toggle: Option<Instant>,",
                );
            }
            src = src.replace(
                "        if pressed && !event.repeat {\n            match code {",
                "        // BLUEORB_DUAL_WIN_FIX_V2\n        if code == KeyCode::F7 {\n            if !pressed {\n                self.f7_held = false;\n            } else {\n                let still_holding = self.f7_held\n                    && self.last_f7_press.is_some_and(|t| t.elapsed() < Duration::from_millis(600));\n                self.last_f7_press = Some(Instant::now());\n                if !event.repeat && !still_holding {\n                    self.f7_held = true;\n                    let cooled = self.last_dual_toggle.is_none_or(|t| t.elapsed() >= Duration::from_millis(350));\n                    if cooled && self.game.is_some() {\n                        self.last_dual_toggle = Some(Instant::now());\n                        self.toggle_dual_windows();\n                    }\n                }\n            }\n            return;\n        }\n        if pressed && !event.repeat {\n            match code {",
            );
            src = src.replace("                KeyCode::F7 => self.toggle_dual_windows(),\n", "");

            let new_toggle_fullscreen = r#"    fn toggle_fullscreen(&mut self) {
        // BLUEORB_DUAL_WIN_FIX_V2
        let Some(full) = self.window.as_ref().map(|w| w.fullscreen().is_some()) else { return };
        let entering_full = !full;
        if entering_full {
            if let Some(window) = &self.window {
                self.maximized_before_full_screen = window.is_maximized();
                self.resize_after_full_screen = false;
            }
            if self.layout == Screens::SeparateWindows {
                let (other_pos, bot_same_mon) = match &self.window {
                    Some(window) => {
                        let main_mpos = window.current_monitor().map(|m| m.position());
                        let other_pos = window
                            .available_monitors()
                            .find(|m| Some(m.position()) != main_mpos)
                            .map(|m| m.position());
                        let bot_same = self.bottom_window.as_ref().is_some_and(|bw| {
                            match (main_mpos, bw.current_monitor().map(|m| m.position())) {
                                (Some(m), Some(b)) => m == b,
                                _ => true,
                            }
                        });
                        (other_pos, bot_same)
                    }
                    None => (None, false),
                };
                if let Some(opos) = other_pos {
                    if bot_same_mon {
                        if let Some(bot_win) = &self.bottom_window {
                            bot_win.set_window_level(winit::window::WindowLevel::Normal);
                            bot_win.set_outer_position(winit::dpi::PhysicalPosition::new(opos.x + 48, opos.y + 48));
                            bot_win.set_visible(true);
                        }
                    }
                } else {
                    let restore = self.last_combined_layout;
                    self.set_layout(restore);
                    self.bottom_mouse_down = false;
                    self.bottom_backend = None;
                    self.bottom_window = None;
                    self.menus.toast = Some(("Fullscreen: combined screens (1 monitor)".to_owned(), Instant::now()));
                }
            }
        }
        if let Some(window) = &self.window {
            window.set_fullscreen(entering_full.then_some(Fullscreen::Borderless(None)));
            if let Some(bot_win) = &self.bottom_window {
                bot_win.set_window_level(winit::window::WindowLevel::Normal);
            }
            if full && self.resize_after_full_screen && !self.maximized_before_full_screen {
                let _ = window.request_inner_size(self.window_size());
            }
        }
    }"#;
            let old_toggle_fs_v1 = r#"    fn toggle_fullscreen(&mut self) {
        if let Some(window) = &self.window {
            let full = window.fullscreen().is_some();
            if !full {
                self.maximized_before_full_screen = window.is_maximized();
                self.resize_after_full_screen = false;
            }
            // BLUEORB_TOGGLE_FULLSCREEN_DUAL_V1
            let entering_full = !full;
            window.set_fullscreen(entering_full.then_some(Fullscreen::Borderless(None)));
            if let Some(bot_win) = &self.bottom_window {
                let main_mon = window.current_monitor();
                let bot_mon = bot_win.current_monitor();
                let same_mon = match (&main_mon, &bot_mon) {
                    (Some(m), Some(b)) => m.position() == b.position(),
                    _ => true,
                };
                if entering_full && same_mon {
                    bot_win.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
                    if let Some(mon) = &main_mon {
                        let mpos = mon.position();
                        let msize = mon.size();
                        let bsz = bot_win.outer_size();
                        let bw = (bsz.width as i32).max(320);
                        let bh = (bsz.height as i32).max(240);
                        let target_x = (mpos.x + msize.width as i32 - bw - 24).clamp(mpos.x, (mpos.x + msize.width as i32 - bw).max(mpos.x));
                        let target_y = (mpos.y + msize.height as i32 - bh - 48).clamp(mpos.y, (mpos.y + msize.height as i32 - bh).max(mpos.y));
                        bot_win.set_outer_position(winit::dpi::PhysicalPosition::new(target_x, target_y));
                    }
                    bot_win.set_visible(true);
                } else if !entering_full {
                    bot_win.set_window_level(winit::window::WindowLevel::Normal);
                }
            }
            // the layout may have changed meanwhile, full screen kept the window's size thru it
            // a window maximized b4 comes back maximized, resizing it would take it out of that
            if full && self.resize_after_full_screen && !self.maximized_before_full_screen {
                let _ = window.request_inner_size(self.window_size());
            }
        }
    }"#;
            let old_toggle_fs_orig = r#"    fn toggle_fullscreen(&mut self) {
        if let Some(window) = &self.window {
            let full = window.fullscreen().is_some();
            if !full {
                self.maximized_before_full_screen = window.is_maximized();
                self.resize_after_full_screen = false;
            }
            window.set_fullscreen((!full).then_some(Fullscreen::Borderless(None)));
            // the layout may have changed meanwhile, full screen kept the window's size thru it
            // a window maximized b4 comes back maximized, resizing it would take it out of that
            if full && self.resize_after_full_screen && !self.maximized_before_full_screen {
                let _ = window.request_inner_size(self.window_size());
            }
        }
    }"#;
            src = src.replace(old_toggle_fs_v1, new_toggle_fullscreen);
            src = src.replace(old_toggle_fs_orig, new_toggle_fullscreen);

            let new_sync_v2 = r#"    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {
        let wants_second = self.layout == Screens::SeparateWindows && self.game.is_some();
        if wants_second && self.bottom_window.is_none() {
            let (main_full, main_mon, other_mon) = match &self.window {
                Some(main_win) => {
                    let full = main_win.fullscreen().is_some();
                    let cur = main_win.current_monitor();
                    let other = cur.as_ref().and_then(|c| {
                        event_loop.available_monitors().find(|m| m.position() != c.position())
                    });
                    (full, cur, other)
                }
                None => (false, None, None),
            };
            let exited_full_now = main_full && other_mon.is_none();
            if exited_full_now {
                if let Some(main_win) = &self.window {
                    main_win.set_fullscreen(None);
                    let _ = main_win.request_inner_size(self.window_size());
                    if let Some(mon) = &main_mon {
                        let mpos = mon.position();
                        main_win.set_outer_position(winit::dpi::PhysicalPosition::new(mpos.x + 24, mpos.y + 48));
                    }
                }
                self.menus.toast = Some(("Dual Window: On (exited fullscreen)".to_owned(), Instant::now()));
            }
            let bot_scale = self.scale.clamp(2, 4);
            let bot_size = winit::dpi::LogicalSize::new(320 * bot_scale, 240 * bot_scale);
            let attrs = Window::default_attributes()
                .with_title("BlueOrb — Touch Screen")
                .with_window_icon(blueorb_initial_window_icon(self.settings.blueorb_rom.as_deref()))
                .with_inner_size(bot_size);
            let shared = self.backend.as_ref().and_then(Backend::shared_device);
            match Backend::create_secondary(event_loop, attrs, shared) {
                Ok((win, mut bot_backend)) => {
                    bot_backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
                    bot_backend.set_orientation(self.orientation.screen_orientation());
                    win.set_window_level(winit::window::WindowLevel::Normal);
                    if let Some(main_win) = &self.window {
                        let bot_outer = win.outer_size();
                        let sf = main_win.scale_factor();
                        let bot_w = if bot_outer.width > 0 {
                            bot_outer.width as i32
                        } else {
                            ((320 * bot_scale) as f64 * sf).round() as i32
                        };
                        let bot_h = if bot_outer.height > 0 {
                            bot_outer.height as i32
                        } else {
                            ((240 * bot_scale) as f64 * sf).round() as i32
                        };
                        if main_full && !exited_full_now {
                            if let Some(other) = &other_mon {
                                let opos = other.position();
                                win.set_outer_position(winit::dpi::PhysicalPosition::new(opos.x + 48, opos.y + 48));
                            }
                        } else if let Some(mon) = &main_mon {
                            let mpos = mon.position();
                            let msize = mon.size();
                            let mon_left = mpos.x;
                            let mon_top = mpos.y;
                            let mon_right = mpos.x + msize.width as i32;
                            let mon_bottom = mpos.y + msize.height as i32;
                            let (pos, main_w) = if exited_full_now {
                                let logical = self.window_size().to_logical::<u32>(sf);
                                let mw = (logical.width as f64 * sf).round() as i32;
                                (winit::dpi::PhysicalPosition::new(mpos.x + 24, mpos.y + 48), mw)
                            } else {
                                (
                                    main_win.outer_position().unwrap_or(mpos),
                                    if !main_win.is_maximized() {
                                        main_win.outer_size().width as i32
                                    } else {
                                        0
                                    },
                                )
                            };
                            let target_pos = if main_w > 0 && pos.x + main_w + 16 + bot_w <= mon_right {
                                winit::dpi::PhysicalPosition::new(
                                    pos.x + main_w + 16,
                                    pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else if let Some(other) = &other_mon {
                                let opos = other.position();
                                winit::dpi::PhysicalPosition::new(opos.x + 48, opos.y + 48)
                            } else if main_w > 0 && pos.x - bot_w - 16 >= mon_left {
                                winit::dpi::PhysicalPosition::new(
                                    pos.x - bot_w - 16,
                                    pos.y.clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            } else {
                                winit::dpi::PhysicalPosition::new(
                                    (pos.x + 48).clamp(mon_left, (mon_right - bot_w).max(mon_left)),
                                    (pos.y + 48).clamp(mon_top, (mon_bottom - bot_h).max(mon_top)),
                                )
                            };
                            win.set_outer_position(target_pos);
                        } else if let Ok(pos) = main_win.outer_position() {
                            win.set_outer_position(winit::dpi::PhysicalPosition::new(
                                (pos.x + 48).max(0),
                                (pos.y + 48).max(0),
                            ));
                        }
                    }
                    win.set_visible(true);
                    win.focus_window();
                    self.bottom_window = Some(win);
                    self.bottom_backend = Some(bot_backend);
                }
                Err(err) => {
                    log::warn!("could not open secondary touch window: {err}");
                    self.menus.toast = Some((format!("Dual Window unavailable: {err}"), Instant::now()));
                    let restore = self.last_combined_layout;
                    self.set_layout(restore);
                }
            }
        } else if !wants_second && self.bottom_window.is_some() {
            self.bottom_mouse_down = false;
            self.bottom_backend = None;
            self.bottom_window = None;
        }
    }"#;
            if let Some(start) = src.find("    fn sync_bottom_window(&mut self, event_loop: &ActiveEventLoop) {") {
                if let Some(rel_end) = src[start..].find("\n\n    fn present(&mut self, event_loop: &ActiveEventLoop)") {
                    src.replace_range(start..start + rel_end, new_sync_v2);
                }
            }
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_FAST_FORWARD_CAP_V1") {
            src = src.replace(
                "        if fast != self.fast_forwarding {\n            self.fast_forwarding = fast;\n            if !fast {\n                // back at the console's pace, w/ no sound piled up\n                if let Some(audio) = &self.audio {\n                    audio.clear();\n                }\n                self.next_frame = Instant::now();\n            }\n        }",
                "        if fast != self.fast_forwarding {\n            self.fast_forwarding = fast;\n            self.next_frame = Instant::now();\n            self.next_shown = Instant::now();\n            if !fast {\n                // back at the console's pace, w/ no sound piled up\n                if let Some(audio) = &self.audio {\n                    audio.clear();\n                }\n            }\n        }",
            );
            src = src.replace(
                "        self.next_frame += FRAME_TIME;\n        if fast {\n            // nothing to wait for\n            self.next_frame = Instant::now();\n        }",
                "        // BLUEORB_FAST_FORWARD_CAP_V1\n        let ff_mult = self.active.fast_forward_speed.clamp(2, 8);\n        self.next_frame += if fast { FRAME_TIME / ff_mult } else { FRAME_TIME };",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_SCALE_FACTOR_RESIZE") {
            src = src.replace(
                "                WindowEvent::Resized(size) => {\n                    if let Some(bot) = &mut self.bottom_backend {\n                        bot.resize(size.width, size.height);\n                    }\n                }",
                "                WindowEvent::Resized(size) => {\n                    if let Some(bot) = &mut self.bottom_backend {\n                        bot.resize(size.width, size.height);\n                    }\n                }\n                WindowEvent::ScaleFactorChanged { .. } => {\n                    if let (Some(bot), Some(bw)) = (&mut self.bottom_backend, &self.bottom_window) {\n                        let size = bw.inner_size();\n                        bot.resize(size.width, size.height);\n                    }\n                }",
            );
            src = src.replace(
                "            WindowEvent::Resized(size) => {\n                if let Some(backend) = &mut self.backend {\n                    backend.resize(size.width, size.height);\n                }\n            }",
                "            WindowEvent::Resized(size) => {\n                if let Some(backend) = &mut self.backend {\n                    backend.resize(size.width, size.height);\n                }\n            }\n            WindowEvent::ScaleFactorChanged { .. } => {\n                // BLUEORB_SCALE_FACTOR_RESIZE\n                if let (Some(backend), Some(window)) = (&mut self.backend, &self.window) {\n                    let size = window.inner_size();\n                    backend.resize(size.width, size.height);\n                }\n            }",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_COOP_MOD_V1") {
            src = src.replace("mod gui;", "// BLUEORB_COOP_MOD_V1\nmod coop;\nmod gui;");
            src = src.replace(
                "    room_server: Option<zakuro_core::room::Server>,\n    room: Option<zakuro_core::room::Connection>,",
                "    room_server: Option<zakuro_core::room::Server>,\n    room: Option<zakuro_core::room::Connection>,\n    coop: coop::CoopRuntime,",
            );
            src = src.replace(
                "        room_server: None,\n        room: None,",
                "        room_server: None,\n        room: None,\n        coop: coop::CoopRuntime::new(),",
            );
            src = src.replace(
                "        self.game = None;\n        self.overlay_corner = None;",
                "        self.game = None;\n        self.overlay_corner = None;\n        self.coop.reset_game();",
            );
            src = src.replace(
                "                KeyCode::F6 => {",
                "                KeyCode::F4 => self.menus.multiplayer_open = !self.menus.multiplayer_open,\n                KeyCode::F6 => {",
            );
            src = src.replace(
                "            Action::JoinRoom => {\n                let address = self.settings.room_address.clone();\n                self.join_room(&address);\n            }\n            Action::LeaveRoom => self.leave_room(),",
                "            Action::OpenMultiplayer => self.menus.multiplayer_open = true,\n            Action::JoinLanRoom(address) => {\n                self.settings.room_address = address.clone();\n                self.settings.save();\n                self.join_room(&address);\n            }\n            Action::ToggleCoopPreview => self.coop.preview_buddy = !self.coop.preview_buddy,\n            Action::JoinRoom => {\n                let address = self.settings.room_address.clone();\n                self.join_room(&address);\n            }\n            Action::LeaveRoom => self.leave_room(),",
            );
            src = src.replace(
                "        self.download_play();",
                "        self.download_play();\n        let hosting_info = self.room_server.as_ref().map(|srv| {\n            let rname = match self.settings.room_name.trim() {\n                \"\" => \"BlueOrb Room\",\n                n => n,\n            };\n            (rname, srv.port(), self.room.as_ref().map_or(1, |r| r.members().len().max(1)))\n        });\n        self.coop.tick(self.room.as_ref(), hosting_info, self.menus.multiplayer_open && self.room.is_none());",
            );
            src = src.replace(
                "        let outcome = game.system.run_frame();",
                "        let outcome = game.system.run_frame();\n        self.coop.sample_game(&mut game.system.memory, self.room.as_ref(), self.settings.coop_avatar_style);",
            );
            src = src.replace(
                "        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
                "        let coop_frame = self.coop.build_frame();\n        let coop_ref = &self.coop;\n        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);",
            );
            src = src.replace(
                "                    actions.extend(menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref(), settings, cheats.as_deref(), active_layout, active_top_shape));",
                "                    actions.extend(menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref(), settings, cheats.as_deref(), active_layout, active_top_shape));\n                    if !menus.menu_open && settings.coop_enabled {\n                        if let Some(top_rect) = coop::top_screen_rect_points(window, settings, active_layout, active_top_shape) {\n                            coop::render_overworld_coop(ui.ctx(), top_rect, &coop_frame, settings);\n                        }\n                    }",
            );
            src = src.replace(
                "            if menus.multiplayer_open {\n                actions.extend(menus.multiplayer(ui.ctx(), settings, room.as_ref()));\n            }",
                "            if menus.multiplayer_open {\n                actions.extend(coop::draw_multiplayer_window(ui.ctx(), &mut menus.multiplayer_open, &mut menus.room_password, settings, room.as_ref(), coop_ref));\n            }",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_COOP_MOD_V2") {
            src = src.replace(
                "// BLUEORB_COOP_MOD_V1",
                "// BLUEORB_COOP_MOD_V1\n// BLUEORB_COOP_MOD_V2",
            );
            src = src.replace(
                "            Action::ToggleCoopPreview => self.coop.preview_buddy = !self.coop.preview_buddy,",
                "            Action::ToggleCoopPreview => self.coop.preview_buddy = !self.coop.preview_buddy,\n            Action::SetCoopLeader(mac) => {\n                if let Some(room) = &self.room {\n                    room.set_leader(mac);\n                }\n            }\n            Action::TeleportToLeader => self.coop.request_manual_warp(),",
            );
            src = src.replace(
                "        self.coop.sample_game(&mut game.system.memory, self.room.as_ref(), self.settings.coop_avatar_style);",
                "        self.coop.sample_game(&mut game.system.memory, self.room.as_ref(), &self.settings);",
            );
            src = src.replace(
                "        let coop_frame = self.coop.build_frame();",
                "        let coop_frame = self.coop.build_frame(&self.settings);",
            );
            src = src.replace(
                "        let coop_ref = &self.coop;",
                "        let coop_ref = &self.coop;\n        let room_conn = self.room.as_ref();",
            );
            src = src.replace(
                "                actions.extend(coop::draw_multiplayer_window(ui.ctx(), &mut menus.multiplayer_open, &mut menus.room_password, settings, room.as_ref(), coop_ref));",
                "                actions.extend(coop::draw_multiplayer_window(ui.ctx(), &mut menus.multiplayer_open, &mut menus.room_password, settings, room.as_ref(), room_conn, coop_ref));",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_UPDATER_V1") {
            src = src.replace(
                "// BLUEORB_COOP_MOD_V1",
                "// BLUEORB_UPDATER_V1\n// BLUEORB_COOP_MOD_V1\nmod updater;",
            );
            src = src.replace(
                "    coop: coop::CoopRuntime,",
                "    coop: coop::CoopRuntime,\n    updater: updater::UpdaterRuntime,",
            );
            src = src.replace(
                "        coop: coop::CoopRuntime::new(),",
                "        coop: coop::CoopRuntime::new(),\n        updater: updater::UpdaterRuntime::new(),",
            );
            src = src.replace(
                "                // the game the command line names starts on the device the presenter may lend the renderer, once there's one\n                self.start();",
                "                // the game the command line names starts on the device the presenter may lend the renderer, once there's one\n                self.start();\n                self.updater.start_check(false);",
            );
            src = src.replace(
                "        self.poll_jobs();",
                "        self.poll_jobs();\n        self.menus.update_status = self.updater.status();\n        if self.updater.should_exit_for_restart() {\n            event_loop.exit();\n            return;\n        }\n        if self.settings.auto_update\n            && self.game.is_none()\n            && self.jobs.iter().all(|j| j.stage().finished())\n            && !self.updater.auto_started()\n            && matches!(self.updater.status(), updater::UpdateStatus::Available { .. })\n        {\n            self.updater.start_install(true);\n        }",
            );
            src = src.replace(
                "            Action::TeleportToLeader => self.coop.request_manual_warp(),",
                "            Action::TeleportToLeader => self.coop.request_manual_warp(),\n            Action::CheckUpdates => self.updater.start_check(true),\n            Action::InstallUpdate => self.updater.start_install(false),",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_DISCORD_V1") {
            src = src.replace(
                "// BLUEORB_UPDATER_V1",
                "// BLUEORB_DISCORD_V1\n// BLUEORB_UPDATER_V1\nmod discord;",
            );
            src = src.replace(
                "    updater: updater::UpdaterRuntime,",
                "    updater: updater::UpdaterRuntime,\n    discord: discord::DiscordRpcRuntime,",
            );
            src = src.replace(
                "        updater: updater::UpdaterRuntime::new(),",
                "        updater: updater::UpdaterRuntime::new(),\n        discord: discord::DiscordRpcRuntime::new(),",
            );
            src = src.replace(
                "        self.coop.tick(self.room.as_ref(), hosting_info, self.menus.multiplayer_open && self.room.is_none());",
                "        self.coop.tick(self.room.as_ref(), hosting_info, self.menus.multiplayer_open && self.room.is_none());\n        let rpc_frame = self.coop.build_frame(&self.settings, self.room.as_ref());\n        self.discord.update(\n            self.settings.discord_rpc,\n            &self.settings.discord_client_id,\n            discord::RpcSnapshot {\n                in_game: self.game.is_some(),\n                zone_id: rpc_frame.local_zone,\n                in_battle: rpc_frame.local_in_battle,\n                in_cutscene: rpc_frame.local_in_cutscene,\n                party_count: rpc_frame.local_party_count,\n                coop_members: self.room.as_ref().map_or(0, |r| r.members().len()),\n                is_leader: rpc_frame.local_is_leader,\n            },\n        );\n        self.menus.discord_state = self.discord.state();",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_COOP_MOD_V3") {
            src = src.replace(
                "// BLUEORB_COOP_MOD_V2",
                "// BLUEORB_COOP_MOD_V2\n// BLUEORB_COOP_MOD_V3",
            );
            src = src.replace(
                "                KeyCode::F4 => self.menus.multiplayer_open = !self.menus.multiplayer_open,",
                "                KeyCode::F4 => {\n                    self.menus.multiplayer_open = !self.menus.multiplayer_open;\n                    if self.menus.multiplayer_open {\n                        self.menus.menu_open = false;\n                    }\n                }",
            );
            src = src.replace(
                "            Action::OpenMultiplayer => self.menus.multiplayer_open = true,",
                "            Action::OpenMultiplayer => {\n                self.menus.multiplayer_open = true;\n                self.menus.menu_open = false;\n            }",
            );
            src = src.replace(
                "            Action::TeleportToLeader => self.coop.request_manual_warp(),",
                "            Action::TeleportToLeader => {\n                self.menus.menu_open = false;\n                self.paused = false;\n                self.coop.request_manual_warp();\n            }\n            Action::TeleportToPeer(mac) => {\n                self.menus.menu_open = false;\n                self.paused = false;\n                self.coop.request_warp_to_peer(mac);\n            }",
            );
            src = src.replace(
                "self.coop.build_frame(&self.settings)",
                "self.coop.build_frame(&self.settings, self.room.as_ref())",
            );
            src = src.replace(
                "                    if !menus.menu_open && settings.coop_enabled {\n                        if let Some(top_rect) = coop::top_screen_rect_points(window, settings, active_layout, active_top_shape) {\n                            coop::render_overworld_coop(ui.ctx(), top_rect, &coop_frame, settings);\n                        }\n                    }",
                "                    if settings.coop_enabled && (!menus.menu_open || coop_frame.room_connected || !coop_frame.peers.is_empty()) {\n                        if let Some(top_rect) = coop::top_screen_rect_points(window, settings, active_layout, active_top_shape) {\n                            coop::render_overworld_coop(ui.ctx(), top_rect, &coop_frame, settings);\n                        }\n                    }",
            );
            src = src.replace(
                "        match zakuro_core::room::Connection::join(address.trim(), &self.nickname(), &password) {\n            Ok(room) => {\n                self.room = Some(room);\n                self.link_room();\n            }\n            Err(error) => self.menus.message = Some(format!(\"Could not join the room, {error}.\")),\n        }",
                "        let addr = address.trim();\n        let base_nick = self.nickname();\n        let mut joined: Option<(zakuro_core::room::Connection, String)> = None;\n        let mut last_err = String::new();\n        for attempt in 0..8u32 {\n            let nick = if attempt == 0 {\n                base_nick.clone()\n            } else {\n                let max_prefix = base_nick.chars().take(8).collect::<String>();\n                format!(\"{max_prefix}{}\", attempt + 1)\n            };\n            match zakuro_core::room::Connection::join(addr, &nick, &password) {\n                Ok(room) => {\n                    joined = Some((room, nick));\n                    break;\n                }\n                Err(error) => {\n                    let dup = error.contains(\"has that nickname\");\n                    last_err = error;\n                    if !dup {\n                        break;\n                    }\n                }\n            }\n        }\n        match joined {\n            Some((room, used_nick)) => {\n                let room_title = room.room().to_owned();\n                let count = room.members().len().max(1);\n                self.room = Some(room);\n                self.link_room();\n                self.menus.toast = Some((\n                    format!(\"Connected to '{room_title}' as {used_nick} ({count} online)\"),\n                    Instant::now(),\n                ));\n            }\n            None => {\n                self.menus.message = Some(format!(\"Could not join the room, {last_err}.\"));\n            }\n        }",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_COOP_MOD_V4") {
            src = src.replace(
                "// BLUEORB_COOP_MOD_V3",
                "// BLUEORB_COOP_MOD_V3\n// BLUEORB_COOP_MOD_V4",
            );
            src = src.replace(
                "in_battle: !rpc_frame.in_overworld,",
                "in_battle: rpc_frame.local_in_battle,",
            );
            src = src.replace(
                "if settings.coop_enabled && (!menus.menu_open || coop_frame.room_connected)",
                "if settings.coop_enabled && (!menus.menu_open || coop_frame.room_connected || !coop_frame.peers.is_empty())",
            );
            fs::write(&lib_rs, src.clone()).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_MAC_SHORTCUTS_V1") {
            src = src.replace(
                "// BLUEORB_COOP_MOD_V4",
                "// BLUEORB_COOP_MOD_V4\n// BLUEORB_MAC_SHORTCUTS_V1",
            );
            src = src.replace(
                "    discord: discord::DiscordRpcRuntime,",
                "    discord: discord::DiscordRpcRuntime,\n    modifiers: winit::keyboard::ModifiersState,",
            );
            src = src.replace(
                "        discord: discord::DiscordRpcRuntime::new(),",
                "        discord: discord::DiscordRpcRuntime::new(),\n        modifiers: winit::keyboard::ModifiersState::empty(),",
            );
            src = src.replace(
                "                WindowEvent::Focused(false) => {\n                    self.bottom_mouse_down = false;\n                    self.fast_forward_key = false;\n                    self.keyboard.release();\n                }",
                "                WindowEvent::Focused(false) => {\n                    self.bottom_mouse_down = false;\n                    self.fast_forward_key = false;\n                    self.modifiers = winit::keyboard::ModifiersState::empty();\n                    self.keyboard.release();\n                }",
            );
            src = src.replace(
                "                WindowEvent::KeyboardInput { event, .. } => self.key(event, false),",
                "                WindowEvent::ModifiersChanged(mods) => self.modifiers = mods.state(),\n                WindowEvent::KeyboardInput { event, .. } => self.key(event, false),",
            );
            src = src.replace(
                "            WindowEvent::Focused(false) => {\n                self.fast_forward_key = false;\n                self.blow(false);\n                self.keyboard.release();\n            }",
                "            WindowEvent::Focused(false) => {\n                self.fast_forward_key = false;\n                self.modifiers = winit::keyboard::ModifiersState::empty();\n                self.blow(false);\n                self.keyboard.release();\n            }",
            );
            src = src.replace(
                "            WindowEvent::KeyboardInput { event, .. } => self.key(event, consumed),",
                "            WindowEvent::ModifiersChanged(mods) => self.modifiers = mods.state(),\n            WindowEvent::KeyboardInput { event, .. } => self.key(event, consumed),",
            );
            src = src.replace(
                "        if pressed && !event.repeat {\n            match code {",
                "        if pressed && !event.repeat && (self.modifiers.super_key() || self.modifiers.control_key()) {\n            match code {\n                KeyCode::Digit4 => {\n                    self.menus.multiplayer_open = !self.menus.multiplayer_open;\n                    if self.menus.multiplayer_open {\n                        self.menus.menu_open = false;\n                    }\n                    return;\n                }\n                KeyCode::KeyM if self.modifiers.shift_key() => {\n                    self.menus.multiplayer_open = !self.menus.multiplayer_open;\n                    if self.menus.multiplayer_open {\n                        self.menus.menu_open = false;\n                    }\n                    return;\n                }\n                KeyCode::Digit6 => {\n                    let shapes = settings::TopShape::ALL;\n                    let idx = shapes.iter().position(|&s| s == self.active.top_shape).unwrap_or(0);\n                    self.set_top_shape(shapes[(idx + 1) % shapes.len()]);\n                    return;\n                }\n                KeyCode::Digit7 => {\n                    let cooled = self.last_dual_toggle.is_none_or(|t| t.elapsed() >= Duration::from_millis(350));\n                    if cooled && self.game.is_some() {\n                        self.last_dual_toggle = Some(Instant::now());\n                        self.toggle_dual_windows();\n                    }\n                    return;\n                }\n                KeyCode::KeyD if self.modifiers.shift_key() => {\n                    let cooled = self.last_dual_toggle.is_none_or(|t| t.elapsed() >= Duration::from_millis(350));\n                    if cooled && self.game.is_some() {\n                        self.last_dual_toggle = Some(Instant::now());\n                        self.toggle_dual_windows();\n                    }\n                    return;\n                }\n                KeyCode::KeyF if self.modifiers.shift_key() => {\n                    self.toggle_fullscreen();\n                    return;\n                }\n                KeyCode::KeyP => {\n                    self.paused = !self.paused;\n                    log::info!(\"{}\", if self.paused { \"paused\" } else { \"resumed\" });\n                    return;\n                }\n                _ => {}\n            }\n        }\n        if pressed && !event.repeat {\n            match code {",
            );
            fs::write(&lib_rs, src).map_err(|e| e.to_string())?;
        }
    }

    // 4h. `zig.rs` : empêcher les fenêtres cmd Windows lors du téléchargement/extraction de Zig (`tar`/`curl`/`zig --version`)
    let zig_rs = zakuro_src.join("zig.rs");
    if zig_rs.exists() {
        let mut src = read_lf(&zig_rs).map_err(|e| e.to_string())?;
        if src.contains("let cmd = hidden_command(program);") {
            src = src.replace(
                "let cmd = hidden_command(program);",
                "let cmd = std::process::Command::new(program);",
            );
            fs::write(&zig_rs, &src).map_err(|e| e.to_string())?;
        }
        if !src.contains("BLUEORB_ZIG_NO_WINDOW") {
            src = src.replace("Command::new(program)", "hidden_command(program)");
            src = src.replace("Command::new(self.program())", "hidden_command(self.program())");
            src = src.replace("Command::new(\"tar\")", "hidden_command(\"tar\")");
            src = src.replace(
                "use recomp3ds::compile::Compiler;",
                "use recomp3ds::compile::Compiler;\n\n// BLUEORB_ZIG_NO_WINDOW\nfn hidden_command(program: &str) -> Command {\n    let cmd = std::process::Command::new(program);\n    #[cfg(windows)]\n    {\n        use std::os::windows::process::CommandExt;\n        let mut cmd = cmd;\n        cmd.creation_flags(0x0800_0000);\n        return cmd;\n    }\n    #[allow(unreachable_code)]\n    cmd\n}",
            );
            fs::write(&zig_rs, src).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// Writes `settings.toml` (`frame_interpolation = false` and `language = "english"` by default).
pub fn write_remaster_settings_toml(
    settings_path: &Path,
    title_id: &str,
    internal_res: u32,
    no_outlines: bool,
    unlock_turbo_60: bool,
    language_code: u32,
    top_aspect: &str,
    dual_windows: bool,
    force_overwrite: bool,
) -> Result<(), String> {
    if settings_path.exists() && !force_overwrite {
        return Ok(());
    }
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let lang_name = match language_code {
        0 => "japanese",
        1 => "english",
        2 => "french",
        3 => "german",
        4 => "italian",
        5 => "spanish",
        _ => "english",
    };

    let layout_str = if dual_windows {
        "separate_windows"
    } else {
        "stacked"
    };

    let mut active_enhancements = Vec::new();
    if no_outlines {
        active_enhancements.push("\"No Outlines (HD)\"");
    }
    if unlock_turbo_60 {
        active_enhancements.push("\"60 FPS\"");
    }
    let enh_list = active_enhancements.join(", ");

    let toml_text = format!(
        r#"# Configuration BlueOrb (zéro clignotement 60 Hz par défaut)
language = "{lang}"
renderer = "vulkan"
hardware_rasterizer = true
resolution = {res}
texture_packs = true
frame_interpolation = false
recompiled = true
scale = 2
layout = "{layout}"
orientation = "north"
filter = "sharp"
integer_scale = false
top_shape = "{top_shape}"
stereo = "off"
depth_3d = 50
swap_eyes = false
volume = 1.0
mute = false
show_fps = false

[enhancements]
{tid} = [{enh}]
"#,
        lang = lang_name,
        res = internal_res.clamp(1, 10),
        layout = layout_str,
        top_shape = top_aspect,
        tid = title_id,
        enh = enh_list,
    );

    fs::write(settings_path, toml_text).map_err(|e| e.to_string())
}

const CONSOLE_THEME_FN_RS: &str = r#"
// BLUEORB_CONSOLE_THEME
fn apply_blueorb_console_theme(ctx: &egui::Context) {
    use egui::{Color32, CornerRadius, CursorIcon, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme};
    ctx.set_theme(Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Body, FontId::new(13.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(17.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace));

        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(14);

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.window_fill = Color32::from_rgb(24, 24, 27);
        v.panel_fill = Color32::from_rgb(24, 24, 27);
        v.faint_bg_color = Color32::from_rgb(31, 31, 35);
        v.extreme_bg_color = Color32::from_rgb(17, 17, 20);
        v.code_bg_color = Color32::from_rgb(17, 17, 20);
        v.window_corner_radius = CornerRadius::same(5);
        v.menu_corner_radius = CornerRadius::same(4);
        v.window_stroke = Stroke::new(1.0, Color32::from_rgb(54, 54, 61));
        v.window_shadow = Shadow {
            offset: [0, 8],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };
        v.popup_shadow = Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(120),
        };
        v.slider_trailing_fill = true;
        v.interact_cursor = Some(CursorIcon::PointingHand);
        v.selection.bg_fill = Color32::from_rgb(56, 98, 154);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);

        v.widgets.noninteractive.bg_fill = Color32::from_rgb(30, 30, 34);
        v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(30, 30, 34);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(50, 50, 57));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(228, 228, 232));
        v.widgets.noninteractive.corner_radius = CornerRadius::same(4);

        v.widgets.inactive.bg_fill = Color32::from_rgb(38, 38, 43);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 38, 43);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(60, 60, 68));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(228, 228, 232));
        v.widgets.inactive.corner_radius = CornerRadius::same(4);

        v.widgets.hovered.bg_fill = Color32::from_rgb(48, 48, 55);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(48, 48, 55);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(82, 82, 92));
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        v.widgets.hovered.corner_radius = CornerRadius::same(4);

        v.widgets.active.bg_fill = Color32::from_rgb(56, 98, 154);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(56, 98, 154);
        v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(88, 134, 196));
        v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        v.widgets.active.corner_radius = CornerRadius::same(4);

        v.widgets.open = v.widgets.hovered;
    });
}
"#;

const ONBOARDING_LIBRARY_FN_RS: &str = r#"    /// Full-bleed desktop game launcher with edge-to-edge twilight ocean vista,
    /// floating glass header, and bottom Hero Launch Dock.
    pub fn library(&mut self, ui: &mut egui::Ui, library: &Library, settings: &Settings, jobs: &[Job], waiting: &[Wanted]) -> Vec<Action> {
        // BLUEORB_LAUNCHER_UI_V3
        let mut actions = Vec::new();
        let _ = (
            self.background.get(ui.ctx(), None),
            idle(library.games.iter().map(Game::wanted), jobs, waiting),
        );

        let active_job = jobs.iter().find(|j| !j.stage().finished());
        let failed_job = jobs.iter().rev().find_map(|j| match j.stage() {
            Stage::Failed(err) if err != "cancelled" => Some((j.program_id, err)),
            _ => None,
        });
        let hovering_file = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let show_install_apps = can_install_to_applications();

        let saved_rom = settings.blueorb_rom.as_ref().filter(|p| p.is_file());
        let matched_game = saved_rom.and_then(|rom| {
            library.games.iter().find(|g| &g.path == rom).or_else(|| library.games.first())
        });
        let game_icon_id = matched_game.and_then(|g| {
            let rgba = g.icon.as_deref().filter(|px| px.len() == ICON_SIZE * ICON_SIZE * 4)?;
            let tex = self.icons.entry(g.path.clone()).or_insert_with(|| {
                let img = egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], rgba);
                ui.ctx().load_texture(format!("game_icon_{:016x}", g.program_id), img, egui::TextureOptions::LINEAR)
            });
            Some(tex.id())
        });

        // Edge-to-edge full-bleed canvas
        let ws_frame = egui::Frame::NONE
            .fill(Color32::from_rgb(7, 13, 24))
            .inner_margin(egui::Margin::ZERO);

        egui::CentralPanel::default().frame(ws_frame).show(ui, |ui| {
            let full_rect = ui.max_rect();
            let ocean_tex = self.icons.entry(PathBuf::from("__blueorb_ocean_bg__")).or_insert_with(|| {
                let img = blueorb_load_ocean_scenery();
                ui.ctx().load_texture("blueorb_ocean_bg", img, egui::TextureOptions::LINEAR)
            });
            let tex_size = ocean_tex.size();
            let uv = cover(full_rect.size(), tex_size);
            ui.painter().image(ocean_tex.id(), full_rect, uv, Color32::WHITE);

            // Smooth vertical edge gradients: keep center (24%..50%) completely untouched
            let h = full_rect.height();
            let top_grad_rect = egui::Rect::from_min_max(
                full_rect.min,
                egui::pos2(full_rect.right(), full_rect.top() + h * 0.24),
            );
            paint_vertical_gradient(
                ui.painter(),
                top_grad_rect,
                Color32::from_rgba_unmultiplied(6, 12, 24, 160),
                Color32::from_rgba_unmultiplied(6, 12, 24, 0),
            );

            let mid_hero_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.left(), full_rect.top() + h * 0.50),
                egui::pos2(full_rect.right(), full_rect.top() + h * 0.78),
            );
            paint_vertical_gradient(
                ui.painter(),
                mid_hero_rect,
                Color32::from_rgba_unmultiplied(6, 12, 24, 0),
                Color32::from_rgba_unmultiplied(6, 12, 24, 168),
            );

            let bot_anchor_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.left(), full_rect.top() + h * 0.78),
                full_rect.max,
            );
            paint_vertical_gradient(
                ui.painter(),
                bot_anchor_rect,
                Color32::from_rgba_unmultiplied(6, 12, 24, 168),
                Color32::from_rgba_unmultiplied(6, 12, 24, 238),
            );

            // Floating Top Header
            let pad_x = 24.0;
            let header_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.left() + pad_x, full_rect.top() + 18.0),
                egui::pos2(full_rect.right() - pad_x, full_rect.top() + 54.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(header_rect), |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (icon_rect, _) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), egui::Sense::hover());
                    paint_diveball_emblem(ui.painter(), icon_rect.center(), 12.0);
                    ui.label(RichText::new("BlueOrb").size(17.5).strong().color(Color32::WHITE));

                    egui::Frame::NONE
                        .fill(Color32::from_rgba_unmultiplied(56, 134, 232, 42))
                        .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(130, 198, 255, 86)))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(8, 2))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("NATIVE AOT")
                                    .size(10.0)
                                    .strong()
                                    .monospace()
                                    .color(Color32::from_rgb(174, 218, 255)),
                            );
                        });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let rom_btn_lbl = if saved_rom.is_some() { "Change ROM…" } else { "Open ROM…" };
                        if launcher_glass_button(ui, rom_btn_lbl, active_job.is_none()) {
                            actions.push(Action::PickRomFile);
                        }
                        if launcher_glass_button(ui, "Clean Cache", active_job.is_none()) {
                            actions.push(Action::CleanResiduals);
                        }
                        if launcher_glass_button(ui, "Multiplayer", true) {
                            actions.push(Action::OpenMultiplayer);
                        }
                        if show_install_apps && launcher_glass_button(ui, "Install to /Applications", true) {
                            actions.push(Action::InstallToApplications);
                        }
                    });
                });
            });

            // Bottom Hero Launch Dock
            let err_h = if failed_job.is_some() && active_job.is_none() { 44.0 } else { 0.0 };
            let dock_h = if active_job.is_some() { 154.0 } else { 116.0 + err_h };
            let dock_bottom = full_rect.bottom() - 18.0;
            let dock_top = (dock_bottom - dock_h).max(header_rect.bottom() + 14.0);
            let dock_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.left() + pad_x, dock_top),
                egui::pos2(full_rect.right() - pad_x, dock_bottom),
            );

            ui.scope_builder(egui::UiBuilder::new().max_rect(dock_rect), |ui| {
                ui.vertical(|ui| {
                    if let Some((_, ref err)) = failed_job.filter(|_| active_job.is_none()) {
                        egui::Frame::NONE
                            .fill(Color32::from_rgba_unmultiplied(78, 18, 28, 224))
                            .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 118, 130, 128)))
                            .corner_radius(egui::CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(12, 7))
                            .show(ui, |ui| {
                                ui.set_width((dock_rect.width() - 26.0).max(160.0));
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("Build failed: {err}"))
                                            .size(12.5)
                                            .color(Color32::from_rgb(255, 210, 214)),
                                    )
                                    .truncate(),
                                );
                            });
                        ui.add_space(10.0);
                    }

                    if let Some(job) = active_job {
                        ui.ctx().request_repaint();
                        egui::Frame::NONE
                            .fill(Color32::from_rgba_unmultiplied(9, 16, 30, 216))
                            .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(150, 206, 255, 72)))
                            .corner_radius(egui::CornerRadius::same(14))
                            .shadow(egui::Shadow {
                                offset: [0, 8],
                                blur: 24,
                                spread: 0,
                                color: Color32::from_rgba_unmultiplied(0, 6, 18, 130),
                            })
                            .inner_margin(egui::Margin::same(18))
                            .show(ui, |ui| {
                                let cw = (dock_rect.width() - 38.0).max(200.0);
                                ui.set_min_width(cw);
                                ui.set_max_width(cw);

                                let stage = job.stage();
                                let mins = job.elapsed().as_secs() / 60;
                                let secs = job.elapsed().as_secs() % 60;
                                let pct = (stage.fraction().clamp(0.0, 1.0) * 100.0).round() as u32;
                                let step_txt = match &stage {
                                    Stage::Downloading(p) => format!("Downloading portable Zig C toolchain ({:.0}%)", p * 100.0),
                                    Stage::Generating => "Translating ARM11 & CRO modules to native C…".to_owned(),
                                    Stage::Compiling { done, total } => format!("Compiling native object units ({done} / {total})"),
                                    Stage::Installing => "Linking native library…".to_owned(),
                                    Stage::Done => "Launching game…".to_owned(),
                                    Stage::Failed(e) => format!("Failed: {e}"),
                                };

                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 12.0;
                                    let (tile_rect, _) = ui.allocate_exact_size(Vec2::splat(42.0), egui::Sense::hover());
                                    if ui.is_rect_visible(tile_rect) {
                                        ui.painter().rect_filled(
                                            tile_rect,
                                            egui::CornerRadius::same(10),
                                            Color32::from_rgba_unmultiplied(16, 28, 48, 230),
                                        );
                                        ui.painter().rect_stroke(
                                            tile_rect,
                                            egui::CornerRadius::same(10),
                                            egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(150, 206, 255, 84)),
                                            egui::StrokeKind::Inside,
                                        );
                                        if let Some(tex_id) = game_icon_id {
                                            ui.painter().image(
                                                tex_id,
                                                tile_rect.shrink(3.0),
                                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                                Color32::WHITE,
                                            );
                                        } else {
                                            paint_diveball_emblem(ui.painter(), tile_rect.center(), 15.0);
                                        }
                                    }

                                    ui.vertical(|ui| {
                                        ui.set_max_width((cw - 240.0).max(120.0));
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&job.name).size(16.0).strong().color(Color32::WHITE),
                                            )
                                            .truncate(),
                                        );
                                        ui.add_space(2.0);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&step_txt).size(12.5).color(Color32::from_rgb(176, 196, 222)),
                                            )
                                            .truncate(),
                                        );
                                    });

                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.spacing_mut().item_spacing.x = 12.0;
                                        if launcher_glass_button(ui, "Cancel", true) {
                                            actions.push(Action::CancelRecompile(job.program_id));
                                        }
                                        ui.label(
                                            RichText::new(format!("{mins:02}:{secs:02}"))
                                                .size(12.5)
                                                .monospace()
                                                .color(Color32::from_rgb(154, 174, 202)),
                                        );
                                        ui.label(
                                            RichText::new(format!("{pct}%"))
                                                .size(18.0)
                                                .strong()
                                                .color(Color32::from_rgb(112, 206, 255)),
                                        );
                                    });
                                });

                                ui.add_space(12.0);
                                let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(cw, 10.0), egui::Sense::hover());
                                if ui.is_rect_visible(bar_rect) {
                                    paint_horizontal_progress_bar(ui.painter(), bar_rect, stage.fraction());
                                }
                            });
                    } else {
                        // Hero Row: Game Icon + Title/Status on left, Primary Play CTA on right
                        let rom_is_recompiled = matched_game.is_some_and(|g| g.recompiled);
                        let hero_title = if let Some(rom) = saved_rom {
                            matched_game
                                .map(|g| g.name.clone())
                                .filter(|n| !n.trim().is_empty())
                                .unwrap_or_else(|| {
                                    rom.file_stem()
                                        .map(|s| s.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "BlueOrb".to_owned())
                                })
                        } else {
                            "BlueOrb".to_owned()
                        };

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 16.0;

                            // 58x58 Hero Icon Tile
                            let (tile_rect, _) = ui.allocate_exact_size(Vec2::splat(58.0), egui::Sense::hover());
                            if ui.is_rect_visible(tile_rect) {
                                ui.painter().rect_filled(
                                    tile_rect.translate(Vec2::new(0.0, 3.0)),
                                    egui::CornerRadius::same(14),
                                    Color32::from_rgba_unmultiplied(0, 6, 18, 120),
                                );
                                ui.painter().rect_filled(
                                    tile_rect,
                                    egui::CornerRadius::same(14),
                                    Color32::from_rgba_unmultiplied(12, 20, 36, 232),
                                );
                                ui.painter().rect_stroke(
                                    tile_rect,
                                    egui::CornerRadius::same(14),
                                    egui::Stroke::new(1.2, Color32::from_rgba_unmultiplied(164, 212, 255, 96)),
                                    egui::StrokeKind::Inside,
                                );
                                if let Some(tex_id) = game_icon_id {
                                    ui.painter().image(
                                        tex_id,
                                        tile_rect.shrink(5.0),
                                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                } else {
                                    paint_diveball_emblem(ui.painter(), tile_rect.center(), 21.5);
                                }
                            }

                            let cta_reserved = if saved_rom.is_some() { 296.0 } else { 208.0 };
                            let text_max_w = (dock_rect.width() - cta_reserved - 82.0).max(140.0);

                            ui.vertical(|ui| {
                                ui.set_max_width(text_max_w);
                                ui.add_space(4.0);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&hero_title).size(23.0).strong().color(Color32::WHITE),
                                    )
                                    .truncate(),
                                );
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 7.0;
                                    if saved_rom.is_some() {
                                        let (dot_col, status_txt) = if rom_is_recompiled {
                                            (
                                                Color32::from_rgb(64, 224, 148),
                                                "Ready to Play  ·  Native HD & Ultrawide",
                                            )
                                        } else {
                                            (
                                                Color32::from_rgb(88, 188, 255),
                                                "Ready to Compile  ·  Automatic Native AOT Build",
                                            )
                                        };
                                        let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
                                        if ui.is_rect_visible(dot_rect) {
                                            ui.painter().circle_filled(
                                                dot_rect.center(),
                                                4.2,
                                                Color32::from_rgba_unmultiplied(dot_col.r(), dot_col.g(), dot_col.b(), 64),
                                            );
                                            ui.painter().circle_filled(dot_rect.center(), 3.0, dot_col);
                                        }
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(status_txt).size(13.0).color(Color32::from_rgb(198, 214, 234)),
                                            )
                                            .truncate(),
                                        );
                                    } else {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new("Drop a decrypted .3ds ROM anywhere in this window, or click Select ROM.")
                                                    .size(13.0)
                                                    .color(Color32::from_rgb(186, 204, 228)),
                                            )
                                            .truncate(),
                                        );
                                    }
                                });
                            });

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;
                                if let Some(rom) = saved_rom {
                                    if launcher_hero_cta(ui, 168.0, 46.0, "Play", true) {
                                        actions.push(Action::Play(rom.clone()));
                                    }
                                    if launcher_secondary_hero_button(ui, 114.0, 46.0, "Browse…") {
                                        actions.push(Action::PickRomFile);
                                    }
                                } else if launcher_hero_cta(ui, 196.0, 46.0, "Select .3ds ROM…", false) {
                                    actions.push(Action::PickRomFile);
                                }
                            });
                        });
                    }

                    // Micro Footer Strip
                    ui.add_space(12.0);
                    let div_y = ui.cursor().top();
                    ui.painter().line_segment(
                        [egui::pos2(dock_rect.left(), div_y), egui::pos2(dock_rect.right(), div_y)],
                        egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(160, 206, 255, 28)),
                    );
                    ui.add_space(9.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        launcher_key_hint(ui, "Esc", "Settings");
                        launcher_key_hint(ui, "F4", "Multiplayer");
                        launcher_key_hint(ui, "F6", "Ultrawide");
                        launcher_key_hint(ui, "F7", "Dual Screen");
                        launcher_key_hint(ui, "F11", "Fullscreen");

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let arch_lbl = format!("{}  ·  Vulkan", std::env::consts::ARCH);
                            ui.label(
                                RichText::new(arch_lbl)
                                    .size(11.5)
                                    .monospace()
                                    .color(Color32::from_rgb(138, 158, 188)),
                            );
                        });
                    });
                });
            });

            // Floating Toast
            if let Some((text, since)) = &self.toast {
                if since.elapsed() < std::time::Duration::from_millis(2600) {
                    let ctx = ui.ctx().clone();
                    egui::Area::new(egui::Id::new("launcher_toast"))
                        .order(egui::Order::Foreground)
                        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 62.0))
                        .interactable(false)
                        .show(&ctx, |ui| {
                            egui::Frame::NONE
                                .fill(Color32::from_rgba_unmultiplied(10, 19, 36, 232))
                                .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(150, 206, 255, 85)))
                                .corner_radius(egui::CornerRadius::same(16))
                                .inner_margin(egui::Margin::symmetric(16, 7))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(text).size(13.0).color(Color32::from_rgb(236, 244, 255)));
                                });
                        });
                    ctx.request_repaint();
                } else {
                    self.toast = None;
                }
            }

            // Full-Window Drag-and-Drop Overlay
            if hovering_file {
                let ctx = ui.ctx().clone();
                egui::Area::new(egui::Id::new("blueorb_drop_overlay"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(full_rect.min)
                    .interactable(false)
                    .show(&ctx, |ui| {
                        ui.set_min_size(full_rect.size());
                        ui.set_max_size(full_rect.size());
                        ui.painter().rect_filled(
                            full_rect,
                            egui::CornerRadius::ZERO,
                            Color32::from_rgba_unmultiplied(7, 20, 42, 184),
                        );
                        let drop_card = egui::Rect::from_center_size(
                            full_rect.center(),
                            Vec2::new((full_rect.width() - 64.0).clamp(260.0, 440.0), 172.0),
                        );
                        ui.painter().rect_filled(
                            drop_card,
                            egui::CornerRadius::same(16),
                            Color32::from_rgba_unmultiplied(12, 28, 54, 230),
                        );
                        ui.painter().rect_stroke(
                            drop_card,
                            egui::CornerRadius::same(16),
                            egui::Stroke::new(2.0, Color32::from_rgb(96, 192, 255)),
                            egui::StrokeKind::Inside,
                        );
                        paint_diveball_emblem(
                            ui.painter(),
                            egui::pos2(drop_card.center().x, drop_card.top() + 46.0),
                            22.0,
                        );
                        ui.painter().text(
                            egui::pos2(drop_card.center().x, drop_card.center().y + 12.0),
                            egui::Align2::CENTER_CENTER,
                            "Drop .3ds ROM to Launch",
                            egui::FontId::proportional(20.0),
                            Color32::WHITE,
                        );
                        ui.painter().text(
                            egui::pos2(drop_card.center().x, drop_card.center().y + 42.0),
                            egui::Align2::CENTER_CENTER,
                            "Automatic native recompilation will begin immediately",
                            egui::FontId::proportional(13.0),
                            Color32::from_rgb(168, 214, 255),
                        );
                    });
            }
        });
        actions
    }

"#;

const WIDE_CONSOLE_GAME_MENU_RS: &str = r#"    /// BLUEORB_WIDE_CONSOLE_MENU: Clean native desktop pause overlay (English by default)
    #[allow(clippy::too_many_arguments)]
    pub fn game(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        program_id: Option<u64>,
        fps: Option<f32>,
        recompiled: bool,
        fast: bool,
        jobs: &[Job],
        waiting: usize,
        saved: Option<&str>,
        settings: &mut Settings,
        cheats: Option<&[(String, bool, String, bool)]>,
        active_layout: Screens,
        active_top_shape: TopShape,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let before_settings = settings.clone();
        let ctx = ui.ctx().clone();

        let hud_frame = || {
            egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(17, 17, 20, 224))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(54, 54, 62)))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(10, 5))
        };

        if let Some((text, since)) = &self.toast {
            if since.elapsed() < std::time::Duration::from_millis(2200) {
                egui::Area::new(egui::Id::new("toast"))
                    .order(egui::Order::Foreground)
                    .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 12.0))
                    .interactable(false)
                    .show(&ctx, |ui| {
                        hud_frame().show(ui, |ui| {
                            ui.label(RichText::new(text).size(13.0).color(Color32::from_rgb(236, 236, 240)));
                        });
                    });
                ctx.request_repaint();
            } else {
                self.toast = None;
            }
        }
        if fast && !self.menu_open {
            egui::Area::new(egui::Id::new("fast forward"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::RIGHT_TOP, Vec2::new(-10.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    hud_frame().show(ui, |ui| {
                        ui.label(RichText::new("Fast Forward").size(12.0).monospace());
                    });
                });
        }
        if let Some(fps) = fps.filter(|_| !self.menu_open) {
            egui::Area::new(egui::Id::new("fps"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(10.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    hud_frame().show(ui, |ui| {
                        ui.label(RichText::new(format!("{fps:.0} FPS")).size(12.0).monospace());
                    });
                });
        }
        if !self.menu_open {
            return actions;
        }

        let nav_tabs: [(u8, &str); 5] = [
            (0, "Game"),
            (1, "Display"),
            (2, "Graphics & Audio"),
            (3, "Controls"),
            (4, "System & Cheats"),
        ];

        let total_w = ui.available_width();
        let wide_mode = total_w >= 540.0;

        if wide_mode {
            let sidebar_w = 196.0;
            let sidebar_frame = egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(18, 18, 21, 248))
                .inner_margin(egui::Margin::symmetric(14, 18));

            egui::Panel::left("blueorb_pause_sidebar")
                .resizable(false)
                .exact_size(sidebar_w)
                .show_separator_line(true)
                .frame(sidebar_frame)
                .show(ui, |ui| {
                    let item_w = ui.available_width();

                    egui::Panel::bottom("blueorb_pause_sidebar_footer")
                        .resizable(false)
                        .show_separator_line(false)
                        .frame(egui::Frame::NONE.inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 0 }))
                        .show(ui, |ui| {
                            if console_secondary_button(ui, item_w, 32.0, "Multiplayer & Co-op", true) {
                                self.multiplayer_open = true;
                            }
                            ui.add_space(6.0);
                            if console_primary_button(ui, item_w, 34.0, "Resume") {
                                actions.push(Action::Resume);
                            }
                            ui.add_space(6.0);
                            if console_danger_button(ui, item_w, 32.0, "Quit") {
                                actions.push(Action::Quit);
                            }
                        });

                    ui.label(RichText::new(name).size(14.5).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.label(
                        RichText::new(if recompiled { "Native AOT" } else { "Interpreter" })
                            .size(11.5)
                            .monospace()
                            .color(Color32::from_rgb(130, 130, 140)),
                    );
                    ui.add_space(14.0);

                    ui.spacing_mut().item_spacing.y = 4.0;
                    for (idx, title) in nav_tabs {
                        if console_nav_button(ui, item_w, title, self.tab == idx) {
                            self.tab = idx;
                        }
                    }
                });
        } else {
            let top_frame = egui::Frame::NONE
                .fill(Color32::from_rgba_unmultiplied(18, 18, 21, 248))
                .inner_margin(egui::Margin::symmetric(10, 8));

            egui::Panel::top("blueorb_pause_topbar")
                .resizable(false)
                .show_separator_line(true)
                .frame(top_frame)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (idx, title) in nav_tabs {
                            let sel = self.tab == idx;
                            let btn = egui::Button::new(
                                RichText::new(title)
                                    .size(12.5)
                                    .color(if sel { Color32::WHITE } else { Color32::from_rgb(170, 170, 180) }),
                            )
                            .fill(if sel { Color32::from_rgb(56, 98, 154) } else { Color32::from_rgb(30, 30, 35) });
                            if ui.add(btn).clicked() {
                                self.tab = idx;
                            }
                        }
                    });
                });
        }

        let central_margin_x: i8 = if wide_mode { 24 } else { 14 };
        let central_frame = egui::Frame::NONE
            .fill(Color32::from_rgba_unmultiplied(24, 24, 27, 245))
            .inner_margin(egui::Margin::symmetric(central_margin_x, 18));

        egui::CentralPanel::default().frame(central_frame).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("blueorb_pause_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let body_w = (ui.available_width() - 10.0).clamp(240.0, 680.0);
                    ui.set_max_width(body_w);
                    self.draw_console_tab_body(
                        ui,
                        body_w,
                        program_id,
                        jobs,
                        waiting,
                        saved,
                        settings,
                        cheats,
                        active_layout,
                        active_top_shape,
                        &mut actions,
                    );
                    ui.add_space(12.0);
                });
        });

        if *settings != before_settings {
            if let Some(pid) = program_id {
                if let Some(own) = settings.per_game.get_mut(&game_key(pid)) {
                    if own.resolution.is_some() {
                        own.resolution = Some(settings.resolution);
                    }
                    if own.frame_interpolation.is_some() {
                        own.frame_interpolation = Some(settings.frame_interpolation);
                    }
                    if own.scale.is_some() {
                        own.scale = Some(settings.scale);
                    }
                    if own.filter.is_some() {
                        own.filter = Some(settings.filter);
                    }
                    if own.integer_scale.is_some() {
                        own.integer_scale = Some(settings.integer_scale);
                    }
                    if own.language.is_some() {
                        own.language = Some(settings.language);
                    }
                }
            }
            actions.push(Action::Settings);
        }
        actions
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_console_tab_body(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        program_id: Option<u64>,
        jobs: &[Job],
        waiting: usize,
        saved: Option<&str>,
        settings: &mut Settings,
        cheats: Option<&[(String, bool, String, bool)]>,
        active_layout: Screens,
        active_top_shape: TopShape,
        actions: &mut Vec<Action>,
    ) {
        match self.tab {
            0 => {
                // TAB 0: GAME
                ui.label(RichText::new("Paused").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                let layout_desc = match active_layout {
                    Screens::Stacked => "Stacked",
                    Screens::LargeTop => "Large Top",
                    Screens::SideBySide => "Side by Side",
                    Screens::SeparateWindows => "Dual Window",
                    Screens::TopOnly => "Top Only",
                    Screens::BottomOnly => "Bottom Only",
                };
                let ratio_desc = match active_top_shape {
                    TopShape::Console => "5:3",
                    TopShape::Wide16x9 => "16:9",
                    TopShape::Wide21x9 => "21:9",
                    TopShape::Wide32x9 => "32:9",
                };
                ui.label(
                    RichText::new(format!("{ratio_desc}  ·  {layout_desc}"))
                        .size(12.5)
                        .monospace()
                        .color(Color32::from_rgb(136, 136, 146)),
                );
                ui.add_space(14.0);

                let third = (width - 16.0) / 3.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if console_primary_button(ui, third, 34.0, "Resume (Esc)") {
                        actions.push(Action::Resume);
                    }
                    if console_secondary_button(ui, third, 34.0, "Fullscreen (F11)", true) {
                        actions.push(Action::Fullscreen);
                    }
                    if console_secondary_button(ui, third, 34.0, "Reset", true) {
                        actions.push(Action::Reset);
                    }
                });

                ui.add_space(14.0);

                console_card(ui, width, |ui, cw| {
                    let state_info = saved.unwrap_or("Empty");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Save States").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                        ui.label(RichText::new(format!("·  Slot {} ({state_info})", self.slot)).size(12.5).color(Color32::from_rgb(140, 140, 150)));
                    });
                    ui.add_space(8.0);

                    let slot_opts: [(u8, &str); 9] = [
                        (1, "1"), (2, "2"), (3, "3"), (4, "4"), (5, "5"), (6, "6"), (7, "7"), (8, "8"), (9, "9"),
                    ];
                    if let Some(s) = console_segmented(ui, cw, 32.0, self.slot, &slot_opts, "slots_seg") {
                        self.slot = s;
                    }

                    ui.add_space(10.0);
                    let half = (cw - 8.0) / 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if console_secondary_button(ui, half, 32.0, "Save State (F5)", true) {
                            actions.push(Action::SaveState);
                        }
                        if console_secondary_button(ui, half, 32.0, "Load State (F8)", saved.is_some()) {
                            actions.push(Action::LoadState);
                        }
                    });
                });

                ui.add_space(12.0);
                let half_bot = (width - 8.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if console_secondary_button(ui, half_bot, 32.0, "Change ROM…", true) {
                        actions.push(Action::PickRomFile);
                    }
                    if console_danger_button(ui, half_bot, 32.0, "Quit") {
                        actions.push(Action::Quit);
                    }
                });

                for job in jobs.iter().filter(|job| !job.stage().finished()) {
                    ui.add_space(8.0);
                    ui.label(&job.name);
                    progress(ui, job);
                }
                if waiting > 0 {
                    ui.label(RichText::new(format!("{waiting} queued")).weak());
                }
            }

            1 => {
                // TAB 1: DISPLAY
                ui.label(RichText::new("Display").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                ui.add_space(10.0);

                let card_w = (width - 10.0) / 2.0;
                let row1 = [
                    (Screens::Stacked, "Stacked (Default)", "Top screen above touch screen", 0u8),
                    (Screens::SeparateWindows, "Dual Window (F7)", "Detach touch screen into its own window", 3),
                ];
                let row2 = [
                    (Screens::LargeTop, "Large Top", "Main display with compact touch panel", 1u8),
                    (Screens::SideBySide, "Side by Side", "Horizontal dual-screen layout", 2),
                ];
                for row in [row1, row2] {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        for (lay, title, sub, icon_kind) in row {
                            let selected = active_layout == lay;
                            if console_screen_layout_card(ui, Vec2::new(card_w, 64.0), title, sub, icon_kind, selected) && !selected {
                                if lay == Screens::SeparateWindows {
                                    actions.push(Action::ToggleDualWindows);
                                } else {
                                    actions.push(Action::SetLayout(lay));
                                }
                            }
                        }
                    });
                    ui.add_space(8.0);
                }

                ui.add_space(6.0);
                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Top Screen Aspect Ratio (F6)").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let shape_opts = [
                        (TopShape::Console, "5:3 Native"),
                        (TopShape::Wide16x9, "16:9"),
                        (TopShape::Wide21x9, "21:9"),
                        (TopShape::Wide32x9, "32:9"),
                    ];
                    if let Some(chosen) = console_segmented(ui, cw, 32.0, active_top_shape, &shape_opts, "shape_seg") {
                        actions.push(Action::SetTopShape(chosen));
                    }

                    ui.add_space(12.0);
                    ui.label(RichText::new("Window Scale").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let scales = [(1u32, "1x"), (2, "2x"), (3, "3x"), (4, "4x")];
                    if let Some(s) = console_segmented(ui, cw, 32.0, settings.scale.clamp(1, 4), &scales, "scale_seg") {
                        settings.scale = s;
                    }

                    ui.add_space(12.0);
                    ui.label(RichText::new("Scaling Filter").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let filters = [
                        (Filter::Sharp, "Sharp"),
                        (Filter::Smooth, "Bilinear"),
                        (Filter::Pixels, "Nearest"),
                    ];
                    if let Some(f) = console_segmented(ui, cw, 32.0, settings.filter, &filters, "filter_seg") {
                        settings.filter = f;
                    }
                });
            }

            2 => {
                // TAB 2: GRAPHICS & AUDIO
                ui.label(RichText::new("Graphics & Audio").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                ui.add_space(10.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Internal 3D Resolution (applies on restart)").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let res_opts = [
                        (1u32, "1x (240p)"),
                        (3, "3x (720p)"),
                        (4, "4x (1080p)"),
                        (5, "5x (1440p)"),
                        (8, "8x (4K)"),
                    ];
                    let current_preset = match settings.resolution {
                        0..=2 => 1,
                        3 => 3,
                        4 => 4,
                        5..=6 => 5,
                        _ => 8,
                    };
                    if let Some(r) = console_segmented(ui, cw, 32.0, current_preset, &res_opts, "res_seg") {
                        settings.resolution = r;
                    }

                    if let Some(cheat_list) = cheats {
                        for (idx, (cname, enabled, notes, builtin)) in cheat_list.iter().enumerate() {
                            if !*builtin {
                                continue;
                            }
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(6.0);
                            let mut current = *enabled;
                            let title_en = if cname.contains("No Outlines") {
                                "No Outlines (HD)"
                            } else if cname == "60 FPS" {
                                "60 FPS Speedup (2x)"
                            } else {
                                cname.as_str()
                            };
                            if console_toggle_row(ui, cw, title_en, notes.as_str(), &mut current) {
                                actions.push(Action::Cheat(idx, current));
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(
                        ui,
                        cw,
                        "Frame Interpolation (60 Hz)",
                        "Interpolates slow camera motion between 30 FPS frames.",
                        &mut settings.frame_interpolation,
                    );

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(
                        ui,
                        cw,
                        "Custom Texture Packs",
                        "Loads replacement PNG textures from the Mods folder.",
                        &mut settings.texture_packs,
                    );
                });

                ui.add_space(12.0);

                console_card(ui, width, |ui, cw| {
                    let pct = (settings.volume * 100.0).round() as i32;
                    ui.label(RichText::new(format!("Master Volume ({pct}%)")).size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(4.0);
                    ui.spacing_mut().slider_width = cw;
                    ui.add(egui::Slider::new(&mut settings.volume, 0.0..=1.0).show_value(false));

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(ui, cw, "Mute Audio", "", &mut settings.mute);

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                    console_toggle_row(ui, cw, "Show FPS Counter", "", &mut settings.show_fps);

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.label(RichText::new("System Language (applies on restart)").size(13.5).strong().color(Color32::from_rgb(232, 232, 238)));
                    ui.add_space(6.0);
                    let lang_opts = [
                        (Language::English, "English"),
                        (Language::French, "Français"),
                        (Language::Japanese, "日本語"),
                        (Language::German, "Deutsch"),
                        (Language::Spanish, "Español"),
                        (Language::Italian, "Italiano"),
                    ];
                    if let Some(l) = console_segmented(ui, cw, 32.0, settings.language, &lang_opts, "lang_seg") {
                        settings.language = l;
                    }
                });
            }

            3 => {
                // TAB 3: CONTROLS
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Controls").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                    let rem = (ui.available_width() - 136.0).max(0.0);
                    ui.add_space(rem);
                    if console_secondary_button(ui, 132.0, 28.0, "Reset Defaults", true) {
                        settings.keys = Keys::default();
                        settings.pad = PadButtons::default();
                        self.rebinding = None;
                        self.rebinding_pad = None;
                    }
                });
                ui.add_space(10.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Keyboard").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.add_space(6.0);
                    for (i, (kname, key)) in settings.keys.all_mut().into_iter().enumerate() {
                        let waiting_this = self.rebinding == Some(i);
                        let badge = if waiting_this { "Press key…".to_owned() } else { key_name(*key) };
                        if console_binding_row(ui, cw, friendly_control_name(kname), &badge, waiting_this) {
                            self.rebinding = Some(i);
                            self.rebinding_pad = None;
                        }
                    }
                });

                ui.add_space(12.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Controller").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.add_space(6.0);
                    for (i, (bname, button)) in settings.pad.all_mut().into_iter().enumerate() {
                        let waiting_this = self.rebinding_pad == Some(i);
                        let badge = if waiting_this { "Press button…" } else { button_name(*button) };
                        if console_binding_row(ui, cw, friendly_control_name(bname), badge, waiting_this) {
                            self.rebinding_pad = Some(i);
                            self.rebinding = None;
                        }
                    }
                });
            }

            _ => {
                // TAB 4: SYSTEM & CHEATS
                ui.label(RichText::new("System & Cheats").size(18.0).strong().color(Color32::from_rgb(238, 238, 242)));
                ui.add_space(10.0);

                let half = (width - 8.0) / 2.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if console_secondary_button(ui, half, 32.0, "Multiplayer & Co-op (F4)", true) {
                        self.multiplayer_open = true;
                    }
                    if console_secondary_button(ui, half, 32.0, "Load Camera Image / QR…", true) {
                        actions.push(Action::ChooseCameraPicture);
                    }
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if let Some(pid) = program_id {
                        if console_secondary_button(ui, half, 32.0, "Open Mods Folder", true) {
                            actions.push(Action::Mods(pid));
                        }
                    } else if console_secondary_button(ui, half, 32.0, "Open Cheats Folder", true) {
                        actions.push(Action::CheatsFolder);
                    }
                    if console_secondary_button(ui, half, 32.0, "Back to Launcher", true) {
                        actions.push(Action::Library);
                    }
                });

                ui.add_space(8.0);
                if console_secondary_button(ui, width, 32.0, "Clean Residual Build Files…", true) {
                    actions.push(Action::CleanResiduals);
                }

                ui.add_space(12.0);

                console_card(ui, width, |ui, cw| {
                    ui.label(RichText::new("Action Replay Cheats").size(14.0).strong().color(Color32::from_rgb(236, 236, 240)));
                    ui.add_space(6.0);
                    let btn_half = (cw - 8.0) / 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if console_secondary_button(ui, btn_half, 30.0, "Reload Cheats", true) {
                            actions.push(Action::ReloadCheats);
                        }
                        if console_secondary_button(ui, btn_half, 30.0, "Open Cheats Folder", true) {
                            actions.push(Action::CheatsFolder);
                        }
                    });
                    ui.add_space(8.0);

                    if let Some(cheat_list) = cheats {
                        let user_cheats: Vec<_> = cheat_list.iter().enumerate().filter(|(_, c)| !c.3).collect();
                        if user_cheats.is_empty() {
                            ui.label(
                                RichText::new("No custom cheat codes configured.")
                                    .size(12.5)
                                    .color(Color32::from_rgb(138, 138, 148)),
                            );
                        } else {
                            for (idx, (cname, enabled, notes, _)) in user_cheats {
                                let mut on = *enabled;
                                if console_toggle_row(ui, cw, cname, notes, &mut on) {
                                    actions.push(Action::Cheat(idx, on));
                                }
                                ui.add_space(4.0);
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.cheat_name)
                            .hint_text("Cheat name")
                            .margin(egui::Margin::symmetric(8, 6))
                            .desired_width(cw),
                    );
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cheat_code)
                            .hint_text("00000000 00000000")
                            .margin(egui::Margin::symmetric(8, 6))
                            .desired_rows(3)
                            .desired_width(cw)
                            .font(egui::TextStyle::Monospace),
                    );
                    ui.add_space(6.0);
                    if console_primary_button(ui, 132.0, 30.0, "Add Cheat") && !self.cheat_code.trim().is_empty() {
                        actions.push(Action::AddCheat(
                            std::mem::take(&mut self.cheat_name),
                            std::mem::take(&mut self.cheat_code),
                        ));
                    }
                });
            }
        }
    }"#;

const WIDE_CONSOLE_WIDGET_HELPERS_RS: &str = r#"
fn friendly_control_name(raw: &str) -> &str {
    raw
}

fn can_install_to_applications() -> bool {
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("CI").is_some() {
            return false;
        }
        let Ok(exe) = std::env::current_exe() else { return false };
        let Some(bundle) = exe.ancestors().nth(3) else { return false };
        if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
            return false;
        }
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        !bundle.starts_with("/Applications") && !bundle.starts_with(home.join("Applications"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Loads the embedded 16:9 hand-painted royalty-free twilight ocean illustration.
fn blueorb_load_ocean_scenery() -> egui::ColorImage {
    const BG_BYTES: &[u8] = include_bytes!("launcher_bg.jpg");
    if let Ok(img) = image::load_from_memory(BG_BYTES) {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        return egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
    }
    egui::ColorImage::filled([2, 2], Color32::from_rgb(10, 20, 38))
}

fn paint_diveball_emblem(painter: &egui::Painter, center: egui::Pos2, r: f32) {
    // Clean geometric Dive-Ball vector icon painted directly via egui primitives
    painter.circle_filled(center, r, Color32::from_rgb(232, 238, 246));
    // Upper Sapphire hemisphere clip
    let top_clip = egui::Rect::from_min_max(
        egui::pos2(center.x - r - 1.0, center.y - r - 1.0),
        egui::pos2(center.x + r + 1.0, center.y - r * 0.04),
    );
    let tp = painter.with_clip_rect(top_clip);
    tp.circle_filled(center, r, Color32::from_rgb(24, 108, 218));
    // Aqua upper cap
    let cap_clip = egui::Rect::from_min_max(
        egui::pos2(center.x - r - 1.0, center.y - r - 1.0),
        egui::pos2(center.x + r + 1.0, center.y - r * 0.48),
    );
    painter.with_clip_rect(cap_clip).circle_filled(center, r, Color32::from_rgb(86, 184, 248));
    // White wave accent band
    let wave_clip = egui::Rect::from_min_max(
        egui::pos2(center.x - r - 1.0, center.y - r * 0.25),
        egui::pos2(center.x + r + 1.0, center.y - r * 0.14),
    );
    painter.with_clip_rect(wave_clip).circle_filled(center, r * 0.96, Color32::from_rgb(212, 242, 255));

    // Equatorial dark seam
    let seam_h = (r * 0.16).max(1.8);
    let ball_clip = egui::Rect::from_center_size(center, Vec2::splat(r * 2.0));
    painter.with_clip_rect(ball_clip).rect_filled(
        egui::Rect::from_center_size(center, Vec2::new(r * 2.0, seam_h)),
        egui::CornerRadius::ZERO,
        Color32::from_rgb(24, 28, 36),
    );
    painter.circle_stroke(center, r, egui::Stroke::new(1.0, Color32::from_rgb(20, 24, 32)));

    // Center clasp ring + sapphire button
    painter.circle_filled(center, r * 0.36, Color32::from_rgb(24, 28, 36));
    painter.circle_filled(center, r * 0.25, Color32::from_rgb(240, 244, 250));
    painter.circle_filled(center, r * 0.14, Color32::from_rgb(56, 182, 255));
}

fn paint_vertical_gradient(painter: &egui::Painter, rect: egui::Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_horizontal_progress_bar(painter: &egui::Painter, rect: egui::Rect, fraction: f32) {
    let rounding = egui::CornerRadius::same(5);
    painter.rect_filled(rect, rounding, Color32::from_rgba_unmultiplied(14, 22, 38, 225));
    painter.rect_stroke(
        rect,
        rounding,
        egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(140, 194, 255, 42)),
        egui::StrokeKind::Inside,
    );
    let f = fraction.clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    let fill_w = (rect.width() * f).max(10.0).min(rect.width());
    let fill_rect = egui::Rect::from_min_size(rect.min, Vec2::new(fill_w, rect.height()));
    let mut mesh = egui::Mesh::default();
    let left_col = Color32::from_rgb(42, 122, 232);
    let right_col = Color32::from_rgb(84, 206, 255);
    mesh.colored_vertex(fill_rect.left_top(), left_col);
    mesh.colored_vertex(fill_rect.right_top(), right_col);
    mesh.colored_vertex(fill_rect.right_bottom(), right_col);
    mesh.colored_vertex(fill_rect.left_bottom(), left_col);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    let clipped = painter.with_clip_rect(fill_rect);
    clipped.rect_filled(fill_rect, rounding, left_col);
    clipped.add(egui::Shape::mesh(mesh));
    // Soft top highlight line across active progress bar
    let hi_rect = egui::Rect::from_min_size(
        egui::pos2(fill_rect.left() + 2.0, fill_rect.top() + 1.0),
        Vec2::new((fill_w - 4.0).max(0.0), 2.0),
    );
    if hi_rect.width() > 2.0 {
        clipped.rect_filled(hi_rect, egui::CornerRadius::same(1), Color32::from_rgba_unmultiplied(210, 244, 255, 96));
    }
}

fn launcher_glass_button(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let font = egui::FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE);
    let w = (galley.size().x + 28.0).max(76.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), sense);
    if ui.is_rect_visible(rect) {
        let rounding = egui::CornerRadius::same(15);
        let (fill, stroke) = if !enabled {
            (
                Color32::from_rgba_unmultiplied(12, 20, 34, 128),
                Color32::from_rgba_unmultiplied(140, 184, 232, 28),
            )
        } else if resp.is_pointer_button_down_on() {
            (
                Color32::from_rgba_unmultiplied(36, 78, 134, 218),
                Color32::from_rgba_unmultiplied(150, 212, 255, 138),
            )
        } else if resp.hovered() {
            (
                Color32::from_rgba_unmultiplied(24, 48, 86, 208),
                Color32::from_rgba_unmultiplied(156, 214, 255, 130),
            )
        } else {
            (
                Color32::from_rgba_unmultiplied(12, 21, 38, 178),
                Color32::from_rgba_unmultiplied(150, 200, 255, 64),
            )
        };
        ui.painter().rect_filled(rect, rounding, fill);
        ui.painter().rect_stroke(rect, rounding, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
        let text_col = if enabled {
            Color32::from_rgb(236, 244, 255)
        } else {
            Color32::from_rgb(122, 138, 162)
        };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, text_col);
    }
    enabled && resp.clicked()
}

fn launcher_secondary_hero_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let rounding = egui::CornerRadius::same(11);
        let (fill, stroke) = if resp.is_pointer_button_down_on() {
            (
                Color32::from_rgba_unmultiplied(32, 58, 98, 226),
                Color32::from_rgba_unmultiplied(152, 212, 255, 136),
            )
        } else if resp.hovered() {
            (
                Color32::from_rgba_unmultiplied(24, 44, 78, 218),
                Color32::from_rgba_unmultiplied(156, 212, 255, 120),
            )
        } else {
            (
                Color32::from_rgba_unmultiplied(14, 24, 42, 200),
                Color32::from_rgba_unmultiplied(148, 202, 255, 72),
            )
        };
        ui.painter().rect_filled(rect, rounding, fill);
        ui.painter().rect_stroke(rect, rounding, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.5),
            Color32::from_rgb(232, 240, 252),
        );
    }
    resp.clicked()
}

fn launcher_hero_cta(ui: &mut egui::Ui, width: f32, height: f32, label: &str, with_play_icon: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let rounding = egui::CornerRadius::same(11);
        // Subtle blue ambient shadow beneath primary button
        ui.painter().rect_filled(
            rect.translate(Vec2::new(0.0, 3.0)),
            rounding,
            Color32::from_rgba_unmultiplied(24, 110, 232, 76),
        );
        let (top_col, bot_col, stroke_col) = if resp.is_pointer_button_down_on() {
            (
                Color32::from_rgb(38, 108, 210),
                Color32::from_rgb(28, 84, 174),
                Color32::from_rgb(138, 208, 255),
            )
        } else if resp.hovered() {
            (
                Color32::from_rgb(66, 152, 252),
                Color32::from_rgb(44, 114, 224),
                Color32::from_rgb(168, 224, 255),
            )
        } else {
            (
                Color32::from_rgb(50, 134, 242),
                Color32::from_rgb(34, 98, 204),
                Color32::from_rgb(132, 204, 255),
            )
        };
        ui.painter().rect_filled(rect, rounding, bot_col);
        let top_half = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.center().y + 2.0));
        ui.painter().with_clip_rect(rect).rect_filled(top_half, rounding, top_col);
        ui.painter().rect_stroke(rect, rounding, egui::Stroke::new(1.0, stroke_col), egui::StrokeKind::Inside);

        let font = egui::FontId::proportional(15.5);
        if with_play_icon {
            let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE);
            let total_w = 13.0 + 8.0 + galley.size().x;
            let start_x = rect.center().x - total_w * 0.5;
            let cy = rect.center().y;
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(start_x, cy - 6.0),
                    egui::pos2(start_x + 11.0, cy),
                    egui::pos2(start_x, cy + 6.0),
                ],
                Color32::WHITE,
                egui::Stroke::NONE,
            ));
            ui.painter().text(
                egui::pos2(start_x + 19.0, cy),
                egui::Align2::LEFT_CENTER,
                label,
                font,
                Color32::WHITE,
            );
        } else {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, Color32::WHITE);
        }
    }
    resp.clicked()
}

fn launcher_key_hint(ui: &mut egui::Ui, key: &str, desc: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        egui::Frame::NONE
            .fill(Color32::from_rgba_unmultiplied(24, 38, 64, 176))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(150, 200, 255, 48)))
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(egui::Margin::symmetric(6, 1))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(key)
                        .size(10.5)
                        .strong()
                        .monospace()
                        .color(Color32::from_rgb(206, 224, 248)),
                );
            });
        ui.label(RichText::new(desc).size(11.5).color(Color32::from_rgb(154, 172, 198)));
    });
}

fn console_nav_button(ui: &mut egui::Ui, width: f32, label: &str, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let bg = if active {
            Color32::from_rgb(36, 36, 42)
        } else if hovered {
            Color32::from_rgb(28, 28, 33)
        } else {
            Color32::TRANSPARENT
        };
        if bg != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(4), bg);
        }
        if active {
            let bar = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.top() + 7.0),
                Vec2::new(2.5, rect.height() - 14.0),
            );
            ui.painter().rect_filled(bar, egui::CornerRadius::same(1), Color32::from_rgb(82, 138, 212));
        }
        let text_col = if active || hovered {
            Color32::from_rgb(242, 242, 246)
        } else {
            Color32::from_rgb(164, 164, 174)
        };
        ui.painter().text(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            text_col,
        );
    }
    resp.clicked()
}

fn console_primary_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.is_pointer_button_down_on() {
            Color32::from_rgb(46, 82, 132)
        } else if resp.hovered() {
            Color32::from_rgb(66, 112, 174)
        } else {
            Color32::from_rgb(56, 98, 154)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(86, 134, 198)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(13.0),
            Color32::WHITE,
        );
    }
    resp.clicked()
}

fn console_secondary_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str, enabled: bool) -> bool {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    if ui.is_rect_visible(rect) {
        let fill = if !enabled {
            Color32::from_rgb(28, 28, 32)
        } else if resp.is_pointer_button_down_on() {
            Color32::from_rgb(54, 54, 62)
        } else if resp.hovered() {
            Color32::from_rgb(46, 46, 53)
        } else {
            Color32::from_rgb(38, 38, 44)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(62, 62, 71)),
            egui::StrokeKind::Inside,
        );
        let col = if enabled {
            Color32::from_rgb(232, 232, 238)
        } else {
            Color32::from_rgb(104, 104, 114)
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5),
            col,
        );
    }
    enabled && resp.clicked()
}

fn console_danger_button(ui: &mut egui::Ui, width: f32, height: f32, label: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if resp.hovered() {
            Color32::from_rgb(72, 32, 34)
        } else {
            Color32::from_rgb(46, 25, 27)
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(104, 50, 54)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.5),
            Color32::from_rgb(244, 168, 168),
        );
    }
    resp.clicked()
}

fn console_card(ui: &mut egui::Ui, width: f32, body: impl FnOnce(&mut egui::Ui, f32)) {
    egui::Frame::NONE
        .fill(Color32::from_rgb(30, 30, 34))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 57)))
        .corner_radius(egui::CornerRadius::same(5))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            let cw = (width - 30.0).max(160.0);
            ui.set_min_width(cw);
            ui.set_max_width(cw);
            body(ui, cw);
        });
}

fn console_segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    current: T,
    options: &[(T, &str)],
    id_salt: &str,
) -> Option<T> {
    let mut picked = None;
    let (outer_rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    if ui.is_rect_visible(outer_rect) {
        ui.painter().rect_filled(outer_rect, egui::CornerRadius::same(4), Color32::from_rgb(20, 20, 23));
        ui.painter().rect_stroke(
            outer_rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(48, 48, 55)),
            egui::StrokeKind::Inside,
        );
    }
    let inner = outer_rect.shrink(3.0);
    let n = options.len().max(1) as f32;
    let gap = 3.0;
    let item_w = ((inner.width() - gap * (n - 1.0)) / n).max(12.0);

    for (idx, &(val, label)) in options.iter().enumerate() {
        let x0 = inner.left() + (idx as f32) * (item_w + gap);
        let item_rect = egui::Rect::from_min_size(egui::pos2(x0, inner.top()), Vec2::new(item_w, inner.height()));
        let resp = ui.interact(item_rect, ui.make_persistent_id((id_salt, idx)), egui::Sense::click());
        let selected = val == current;
        if ui.is_rect_visible(item_rect) {
            if selected {
                ui.painter().rect_filled(item_rect, egui::CornerRadius::same(3), Color32::from_rgb(56, 98, 154));
            } else if resp.hovered() {
                ui.painter().rect_filled(item_rect, egui::CornerRadius::same(3), Color32::from_rgb(34, 34, 40));
            }
            let col = if selected || resp.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(168, 168, 178)
            };
            ui.painter().text(
                item_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(12.5),
                col,
            );
        }
        if resp.clicked() && !selected {
            picked = Some(val);
        }
    }
    picked
}

fn console_toggle_row(ui: &mut egui::Ui, width: f32, title: &str, subtitle: &str, value: &mut bool) -> bool {
    let row_h = if subtitle.is_empty() { 28.0 } else { 40.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), egui::Sense::click());
    let mut changed = false;
    if resp.clicked() {
        *value = !*value;
        changed = true;
    }
    if ui.is_rect_visible(rect) {
        let switch_w = 40.0;
        let switch_h = 22.0;
        let switch_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - switch_w, rect.center().y - switch_h * 0.5),
            Vec2::new(switch_w, switch_h),
        );
        let track = if *value {
            Color32::from_rgb(56, 98, 154)
        } else if resp.hovered() {
            Color32::from_rgb(56, 56, 64)
        } else {
            Color32::from_rgb(42, 42, 49)
        };
        ui.painter().rect_filled(switch_rect, egui::CornerRadius::same(11), track);
        let knob_x = if *value { switch_rect.right() - 11.0 } else { switch_rect.left() + 11.0 };
        ui.painter().circle_filled(egui::pos2(knob_x, switch_rect.center().y), 8.0, Color32::WHITE);

        let text_clip = egui::Rect::from_min_max(rect.min, egui::pos2(switch_rect.left() - 12.0, rect.bottom()));
        let tp = ui.painter().with_clip_rect(text_clip);
        if subtitle.is_empty() {
            tp.text(
                egui::pos2(rect.left(), rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(232, 232, 238),
            );
        } else {
            tp.text(
                egui::pos2(rect.left(), rect.top() + 10.0),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(13.0),
                Color32::from_rgb(232, 232, 238),
            );
            tp.text(
                egui::pos2(rect.left(), rect.top() + 28.0),
                egui::Align2::LEFT_CENTER,
                subtitle,
                egui::FontId::proportional(11.5),
                Color32::from_rgb(138, 138, 148),
            );
        }
    }
    changed
}

fn console_binding_row(ui: &mut egui::Ui, width: f32, label: &str, badge: &str, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 30.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(4), Color32::from_rgb(36, 36, 42));
        }
        ui.painter().text(
            egui::pos2(rect.left() + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            Color32::from_rgb(224, 224, 232),
        );
        let badge_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 120.0, rect.center().y - 12.0),
            Vec2::new(116.0, 24.0),
        );
        let fill = if active {
            Color32::from_rgb(56, 98, 154)
        } else {
            Color32::from_rgb(20, 20, 23)
        };
        ui.painter().rect_filled(badge_rect, egui::CornerRadius::same(4), fill);
        ui.painter().rect_stroke(
            badge_rect,
            egui::CornerRadius::same(4),
            egui::Stroke::new(1.0, Color32::from_rgb(52, 52, 60)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            badge,
            egui::FontId::monospace(12.0),
            Color32::WHITE,
        );
    }
    resp.clicked()
}

fn console_screen_layout_card(
    ui: &mut egui::Ui,
    size: Vec2,
    title: &str,
    subtitle: &str,
    kind: u8,
    selected: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let bg = if selected {
            Color32::from_rgb(30, 40, 56)
        } else if resp.hovered() {
            Color32::from_rgb(36, 36, 42)
        } else {
            Color32::from_rgb(30, 30, 34)
        };
        let border = if selected {
            egui::Stroke::new(1.5, Color32::from_rgb(82, 138, 212))
        } else if resp.hovered() {
            egui::Stroke::new(1.0, Color32::from_rgb(74, 74, 84))
        } else {
            egui::Stroke::new(1.0, Color32::from_rgb(50, 50, 57))
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::same(5), bg);
        ui.painter().rect_stroke(rect, egui::CornerRadius::same(5), border, egui::StrokeKind::Inside);

        let cx = rect.left() + 32.0;
        let cy = rect.center().y;
        let top_fill = if selected {
            Color32::from_rgb(104, 168, 248)
        } else {
            Color32::from_rgb(142, 142, 154)
        };
        let bot_fill = if selected {
            Color32::from_rgba_unmultiplied(104, 168, 248, 165)
        } else {
            Color32::from_rgba_unmultiplied(142, 142, 154, 140)
        };

        match kind {
            0 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx, cy - 9.0), Vec2::new(30.0, 15.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx, cy + 9.0), Vec2::new(23.0, 13.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(2), bot_fill);
            }
            1 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx - 7.0, cy), Vec2::new(28.0, 19.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx + 14.0, cy + 4.0), Vec2::new(13.0, 11.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(2), bot_fill);
            }
            2 => {
                let r_top = egui::Rect::from_center_size(egui::pos2(cx - 11.0, cy), Vec2::new(21.0, 14.0));
                let r_bot = egui::Rect::from_center_size(egui::pos2(cx + 11.0, cy), Vec2::new(18.0, 14.0));
                ui.painter().rect_filled(r_top, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(r_bot, egui::CornerRadius::same(2), bot_fill);
            }
            _ => {
                let w1 = egui::Rect::from_center_size(egui::pos2(cx - 7.0, cy - 4.0), Vec2::new(27.0, 17.0));
                let w2 = egui::Rect::from_center_size(egui::pos2(cx + 9.0, cy + 5.0), Vec2::new(22.0, 15.0));
                ui.painter().rect_filled(w1, egui::CornerRadius::same(2), top_fill);
                ui.painter().rect_filled(w2.expand(2.0), egui::CornerRadius::same(3), bg);
                ui.painter().rect_filled(w2, egui::CornerRadius::same(2), bot_fill);
            }
        }

        let text_clip = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 64.0, rect.top() + 4.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 4.0),
        );
        let tp = ui.painter().with_clip_rect(text_clip);
        tp.text(
            egui::pos2(rect.left() + 64.0, rect.center().y - 8.0),
            egui::Align2::LEFT_CENTER,
            title,
            egui::FontId::proportional(13.0),
            Color32::from_rgb(236, 236, 240),
        );
        tp.text(
            egui::pos2(rect.left() + 64.0, rect.center().y + 10.0),
            egui::Align2::LEFT_CENTER,
            subtitle,
            egui::FontId::proportional(11.5),
            Color32::from_rgb(138, 138, 148),
        );
    }
    resp.clicked()
}
"#;

const MOVE_TO_APPLICATIONS_RS: &str = r#"
// BLUEORB_MOVE_TO_APPLICATIONS
/// On first launch outside /Applications, offers to install the bundle there (like most Mac apps do).
#[cfg(target_os = "macos")]
fn blueorb_offer_move_to_applications() {
    use std::path::{Path, PathBuf};

    if std::env::var_os("CI").is_some() || std::env::args().skip(1).any(|a| a.starts_with('-')) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // .../BlueOrb.app/Contents/MacOS/BlueOrb
    let Some(bundle) = exe.ancestors().nth(3).map(Path::to_path_buf) else { return };
    if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
        return;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if bundle.starts_with("/Applications") || bundle.starts_with(home.join("Applications")) {
        return;
    }
    let stamp_dir = home.join("Library/Application Support/BlueOrb");
    let stamp = stamp_dir.join(".skip_move_to_applications");
    if stamp.exists() {
        return;
    }

    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title("Move to Applications folder?")
        .set_description(
            "BlueOrb is not in your Applications folder.\n\nMove it there now so it appears in Launchpad and Spotlight?",
        )
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if answer != rfd::MessageDialogResult::Yes {
        let _ = std::fs::create_dir_all(&stamp_dir);
        let _ = std::fs::write(&stamp, b"1");
        return;
    }

    let name = bundle.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "BlueOrb.app".into());
    let copy_to = |dir: &Path| -> Option<PathBuf> {
        let dest = dir.join(&name);
        if dest.exists() && std::fs::remove_dir_all(&dest).is_err() {
            return None;
        }
        let ok = std::process::Command::new("/usr/bin/ditto")
            .arg(&bundle)
            .arg(&dest)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(dest)
    };
    let user_apps = home.join("Applications");
    let dest = copy_to(Path::new("/Applications")).or_else(|| {
        let _ = std::fs::create_dir_all(&user_apps);
        copy_to(&user_apps)
    });
    let Some(dest) = dest else {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Could not move BlueOrb")
            .set_description("BlueOrb could not be copied to your Applications folder. You can drag it there manually.")
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        return;
    };

    // a quarantined copy launched from a randomized read-only path can't be deleted, leave it alone
    if !bundle.to_string_lossy().contains("/AppTranslocation/") {
        let _ = std::fs::remove_dir_all(&bundle);
    }
    if std::process::Command::new("/usr/bin/open").arg("-n").arg(&dest).spawn().is_ok() {
        std::process::exit(0);
    }
}
"#;

const OFFICIAL_ICON_UPDATER_RS: &str = r#"
#[cfg(target_os = "macos")]
fn blueorb_move_bundle_to_applications_now() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(bundle) = exe.ancestors().nth(3).map(Path::to_path_buf) else { return };
    if bundle.extension().and_then(|e| e.to_str()) != Some("app") {
        return;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if bundle.starts_with("/Applications") || bundle.starts_with(home.join("Applications")) {
        return;
    }
    let name = bundle.file_name().map(|n| n.to_os_string()).unwrap_or_else(|| "BlueOrb.app".into());
    let copy_to = |dir: &Path| -> Option<PathBuf> {
        let dest = dir.join(&name);
        if dest.exists() && std::fs::remove_dir_all(&dest).is_err() {
            return None;
        }
        let ok = std::process::Command::new("/usr/bin/ditto")
            .arg(&bundle)
            .arg(&dest)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(dest)
    };
    let user_apps = home.join("Applications");
    let dest = copy_to(Path::new("/Applications")).or_else(|| {
        let _ = std::fs::create_dir_all(&user_apps);
        copy_to(&user_apps)
    });
    let Some(dest) = dest else {
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Could not move BlueOrb")
            .set_description("BlueOrb could not be copied to your Applications folder. You can drag it there manually.")
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        return;
    };
    if !bundle.to_string_lossy().contains("/AppTranslocation/") {
        let _ = std::fs::remove_dir_all(&bundle);
    }
    if std::process::Command::new("/usr/bin/open").arg("-n").arg(&dest).spawn().is_ok() {
        std::process::exit(0);
    }
}
/// Returns the official Alpha Sapphire ROM icon if a recompiled ROM is already configured,
/// or the procedural Sapphire Dive-Ball launcher icon prior to recompilation.
fn blueorb_initial_window_icon(rom_path: Option<&Path>) -> Option<winit::window::Icon> {
    if let Some(rom) = rom_path.filter(|p| p.is_file()) {
        if let Ok(title) = zakuro_fs::Title::load(rom) {
            let pid = title.program_id();
            if zakuro_core::recompiled::installed(pid).is_some() {
                if let Some(smdh) = title.exefs_file("icon").and_then(library::Smdh::parse) {
                    if smdh.icon.len() == 48 * 48 * 4 {
                        let rgba = blueorb_upscale_smdh_rgba(&smdh.icon, 64);
                        if let Ok(ic) = winit::window::Icon::from_rgba(rgba, 64, 64) {
                            return Some(ic);
                        }
                    }
                }
            }
        }
    }
    let rgba = blueorb_render_diveball_rgba(64);
    winit::window::Icon::from_rgba(rgba, 64, 64).ok()
}

fn blueorb_render_diveball_rgba(size: usize) -> Vec<u8> {
    let mut dst = vec![0u8; size * size * 4];
    let szf = size as f32;
    for y in 0..size {
        let uy = ((y as f32 + 0.5) / szf) * 2.0 - 1.0;
        for x in 0..size {
            let ux = ((x as f32 + 0.5) / szf) * 2.0 - 1.0;
            let idx = (y * size + x) * 4;
            let r_ball = 0.88f32;
            let d2 = ux * ux + uy * uy;
            if d2 > r_ball * r_ball {
                continue;
            }
            let d = d2.sqrt();
            let aa = ((r_ball - d) * szf * 0.5).clamp(0.0, 1.0);
            let nx = ux / r_ball;
            let ny = -uy / r_ball;
            let nz = (1.0 - (nx * nx + ny * ny).min(1.0)).sqrt();
            let py = ny * 0.98 - nz * (-0.19);
            let pz = ny * (-0.19) + nz * 0.98;
            let clasp_ang = pz.clamp(-1.0, 1.0).acos();
            let shade = (0.38 + 0.62 * (nx * -0.4 + ny * 0.56 + nz * 0.72).max(0.0)).clamp(0.0, 1.0);

            let (r, g, b) = if clasp_ang < 0.11 {
                (56.0, 189.0, 255.0)
            } else if clasp_ang < 0.21 {
                (238.0 * shade, 242.0 * shade, 248.0 * shade)
            } else if clasp_ang < 0.28 || py.abs() < 0.07 {
                (26.0, 30.0, 40.0)
            } else if py > 0.0 {
                let wave = 0.23 + 0.08 * (nx * 5.0).cos();
                if (py - wave).abs() < 0.05 {
                    (205.0, 240.0, 255.0)
                } else if py > 0.55 {
                    (64.0 * shade + 20.0, 168.0 * shade + 30.0, 248.0)
                } else {
                    (18.0 * shade + 8.0, 88.0 * shade + 24.0, 212.0 * shade + 40.0)
                }
            } else {
                (238.0 * shade, 242.0 * shade, 248.0 * shade)
            };
            dst[idx] = r.clamp(0.0, 255.0) as u8;
            dst[idx + 1] = g.clamp(0.0, 255.0) as u8;
            dst[idx + 2] = b.clamp(0.0, 255.0) as u8;
            dst[idx + 3] = (aa * 255.0) as u8;
        }
    }
    dst
}

/// Smooth 2x EPX + bilinear squircle upscaler turning the ROM's official 48x48 SMDH icon into crisp HD RGBA.
fn blueorb_upscale_smdh_rgba(src48: &[u8], out_size: usize) -> Vec<u8> {
    let sim = |a: &[u8], b: &[u8]| -> bool {
        (a[0] as i32 - b[0] as i32).abs() * 3
            + (a[1] as i32 - b[1] as i32).abs() * 4
            + (a[2] as i32 - b[2] as i32).abs() * 2
            < 160
    };
    let px48 = |x: usize, y: usize| -> [u8; 4] {
        let i = (y.min(47) * 48 + x.min(47)) * 4;
        [src48[i], src48[i + 1], src48[i + 2], 255]
    };
    let mut s96 = vec![[0u8; 4]; 96 * 96];
    for y in 0..48 {
        for x in 0..48 {
            let p = px48(x, y);
            let a = px48(x, y.saturating_sub(1));
            let b = px48(x + 1, y);
            let c = px48(x.saturating_sub(1), y);
            let d = px48(x, y + 1);
            let blend = |q: [u8; 4], r: [u8; 4]| [
                ((q[0] as u16 + r[0] as u16 + p[0] as u16 * 2) / 4) as u8,
                ((q[1] as u16 + r[1] as u16 + p[1] as u16 * 2) / 4) as u8,
                ((q[2] as u16 + r[2] as u16 + p[2] as u16 * 2) / 4) as u8,
                255,
            ];
            s96[(y * 2) * 96 + x * 2] = if sim(&c, &a) && !sim(&c, &d) && !sim(&a, &b) { blend(c, a) } else { p };
            s96[(y * 2) * 96 + x * 2 + 1] = if sim(&a, &b) && !sim(&c, &a) && !sim(&b, &d) { blend(a, b) } else { p };
            s96[(y * 2 + 1) * 96 + x * 2] = if sim(&d, &c) && !sim(&b, &d) && !sim(&c, &a) { blend(d, c) } else { p };
            s96[(y * 2 + 1) * 96 + x * 2 + 1] = if sim(&b, &d) && !sim(&a, &b) && !sim(&d, &c) { blend(b, d) } else { p };
        }
    }

    let mut dst = vec![0u8; out_size * out_size * 4];
    let szf = out_size as f32;
    let rad = szf * 0.22;
    for y in 0..out_size {
        let yf = y as f32 + 0.5;
        let gy = ((yf / szf) * 96.0 - 0.5).clamp(0.0, 95.0);
        let y0 = gy.floor() as usize;
        let y1 = (y0 + 1).min(95);
        let ty = gy - y0 as f32;
        for x in 0..out_size {
            let xf = x as f32 + 0.5;
            let gx = ((xf / szf) * 96.0 - 0.5).clamp(0.0, 95.0);
            let x0 = gx.floor() as usize;
            let x1 = (x0 + 1).min(95);
            let tx = gx - x0 as f32;

            let c00 = s96[y0 * 96 + x0];
            let c10 = s96[y0 * 96 + x1];
            let c01 = s96[y1 * 96 + x0];
            let c11 = s96[y1 * 96 + x1];

            let dx = if xf < rad { rad - xf } else if xf > szf - rad { xf - (szf - rad) } else { 0.0 };
            let dy = if yf < rad { rad - yf } else if yf > szf - rad { yf - (szf - rad) } else { 0.0 };
            let cd = (dx * dx + dy * dy).sqrt();
            let alpha = ((rad + 0.5 - cd) * 255.0).round().clamp(0.0, 255.0) as u8;

            let o = (y * out_size + x) * 4;
            for ch in 0..3 {
                let top = c00[ch] as f32 * (1.0 - tx) + c10[ch] as f32 * tx;
                let bot = c01[ch] as f32 * (1.0 - tx) + c11[ch] as f32 * tx;
                dst[o + ch] = (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
            }
            dst[o + 3] = alpha;
        }
    }
    dst
}

fn blueorb_encode_png(size: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba, size, size, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(out)
}

/// Replaces the bundle's placeholder icon on disk with the user's official ROM SMDH icon once recompiled.
fn blueorb_persist_official_bundle_icon(icon48: &[u8], game_title: &str) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(exe_dir) = exe.parent() else { return };

    let rgba256 = blueorb_upscale_smdh_rgba(icon48, 256);
    let Some(png256) = blueorb_encode_png(256, &rgba256) else { return };

    #[cfg(target_os = "macos")]
    {
        if exe_dir.ends_with("Contents/MacOS") {
            if let Some(contents) = exe_dir.parent() {
                let res_dir = contents.join("Resources");
                let stamp = res_dir.join(".official_icon_applied");
                let icns_path = res_dir.join("AppIcon.icns");
                if !stamp.is_file() && res_dir.is_dir() {
                    let rgba512 = blueorb_upscale_smdh_rgba(icon48, 512);
                    if let Some(png512) = blueorb_encode_png(512, &rgba512) {
                        let total = 8 + (8 + png256.len()) + (8 + png512.len());
                        let mut icns = Vec::with_capacity(total);
                        icns.extend_from_slice(b"icns");
                        icns.extend_from_slice(&(total as u32).to_be_bytes());
                        icns.extend_from_slice(b"ic08");
                        icns.extend_from_slice(&((8 + png256.len()) as u32).to_be_bytes());
                        icns.extend_from_slice(&png256);
                        icns.extend_from_slice(b"ic09");
                        icns.extend_from_slice(&((8 + png512.len()) as u32).to_be_bytes());
                        icns.extend_from_slice(&png512);
                        if std::fs::write(&icns_path, icns).is_ok() {
                            let _ = std::fs::write(&stamp, game_title.as_bytes());
                            if let Some(bundle) = contents.parent() {
                                let _ = std::process::Command::new("codesign")
                                    .args(["--force", "--deep", "--sign", "-"])
                                    .arg(bundle)
                                    .status();
                                let _ = std::process::Command::new("touch").arg(bundle).status();
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let ico_path = exe_dir.join("AppIcon.ico");
        let stamp = exe_dir.join(".official_icon_applied");
        if !stamp.is_file() {
            let mut ico = Vec::with_capacity(22 + png256.len());
            ico.extend_from_slice(&[0, 0, 1, 0, 1, 0, 0, 0, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(png256.len() as u32).to_le_bytes());
            ico.extend_from_slice(&22u32.to_le_bytes());
            ico.extend_from_slice(&png256);
            if std::fs::write(&ico_path, ico).is_ok() {
                let _ = std::fs::write(&stamp, game_title.as_bytes());
                let safe_name: String = game_title
                    .chars()
                    .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
                    .collect();
                let lnk_path = exe_dir.join(format!("{}.lnk", if safe_name.is_empty() { "Pokemon Alpha Sapphire" } else { &safe_name }));
                let ps_script = format!(
                    "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.IconLocation='{}';$s.Save()",
                    lnk_path.display().to_string().replace('\'', "''"),
                    exe.display().to_string().replace('\'', "''"),
                    exe_dir.display().to_string().replace('\'', "''"),
                    ico_path.display().to_string().replace('\'', "''"),
                );
                let _ = std::process::Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps_script])
                    .status();
            }
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let png_path = exe_dir.join("AppIcon.png");
        let stamp = exe_dir.join(".official_icon_applied");
        if !stamp.is_file() && std::fs::write(&png_path, &png256).is_ok() {
            let _ = std::fs::write(&stamp, game_title.as_bytes());
            let desktop = format!(
                "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\"\nIcon={}\nCategories=Game;\nTerminal=false\n",
                game_title,
                exe.display(),
                png_path.display()
            );
            let _ = std::fs::write(exe_dir.join("BlueOrb.desktop"), desktop);
        }
    }
}
"#;

const UPSCALE_HELPERS_RS: &str = r#"
#[inline]
fn blueorb_color_similar(a: [u8; 4], b: [u8; 4]) -> bool {
    let da = (a[3] as i32 - b[3] as i32).abs();
    if a[3] < 16 && b[3] < 16 {
        return true;
    }
    if da > 48 {
        return false;
    }
    let dr = (a[0] as i32 - b[0] as i32).abs();
    let dg = (a[1] as i32 - b[1] as i32).abs();
    let db = (a[2] as i32 - b[2] as i32).abs();
    (dr * 3 + dg * 4 + db * 2) <= 360
}

#[inline]
fn blueorb_mix_rgba(p: [u8; 4], q: [u8; 4]) -> [u8; 4] {
    [
        ((p[0] as u16 * 3 + q[0] as u16 + 2) >> 2) as u8,
        ((p[1] as u16 * 3 + q[1] as u16 + 2) >> 2) as u8,
        ((p[2] as u16 * 3 + q[2] as u16 + 2) >> 2) as u8,
        ((p[3] as u16 * 3 + q[3] as u16 + 2) >> 2) as u8,
    ]
}

#[inline]
fn blueorb_avg_rgba(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    [
        ((a[0] as u16 + b[0] as u16 + 1) >> 1) as u8,
        ((a[1] as u16 + b[1] as u16 + 1) >> 1) as u8,
        ((a[2] as u16 + b[2] as u16 + 1) >> 1) as u8,
        ((a[3] as u16 + b[3] as u16 + 1) >> 1) as u8,
    ]
}

fn blueorb_epx_smooth_2x(w: usize, h: usize, src: &[[u8; 4]]) -> Vec<[u8; 4]> {
    let dw = w * 2;
    let dh = h * 2;
    let mut dst = vec![[0u8; 4]; dw * dh];

    for y in 0..h {
        let ym1 = y.saturating_sub(1);
        let yp1 = (y + 1).min(h - 1);
        for x in 0..w {
            let xm1 = x.saturating_sub(1);
            let xp1 = (x + 1).min(w - 1);

            let p = src[y * w + x];
            let a = src[ym1 * w + x];
            let b = src[y * w + xp1];
            let c = src[y * w + xm1];
            let d = src[yp1 * w + x];

            let ca = blueorb_color_similar(c, a);
            let ab = blueorb_color_similar(a, b);
            let bd = blueorb_color_similar(b, d);
            let dc = blueorb_color_similar(d, c);
            let cd = blueorb_color_similar(c, d);

            let e0 = if ca && !cd && !ab { blueorb_mix_rgba(blueorb_avg_rgba(c, a), p) } else { p };
            let e1 = if ab && !ca && !bd { blueorb_mix_rgba(blueorb_avg_rgba(a, b), p) } else { p };
            let e2 = if dc && !bd && !ca { blueorb_mix_rgba(blueorb_avg_rgba(d, c), p) } else { p };
            let e3 = if bd && !ab && !dc { blueorb_mix_rgba(blueorb_avg_rgba(b, d), p) } else { p };

            let out_row0 = (y * 2) * dw + (x * 2);
            let out_row1 = (y * 2 + 1) * dw + (x * 2);
            dst[out_row0] = e0;
            dst[out_row0 + 1] = e1;
            dst[out_row1] = e2;
            dst[out_row1 + 1] = e3;
        }
    }
    dst
}

fn blueorb_upscale_4x_mipmapped(width: u32, height: u32, base_texels: &[[u8; 4]]) -> crate::pack::Picture {
    let w1 = width as usize;
    let h1 = height as usize;
    let mip1 = blueorb_epx_smooth_2x(w1, h1, base_texels);
    let mip0 = blueorb_epx_smooth_2x(w1 * 2, h1 * 2, &mip1);

    let len0 = mip0.len() * 4;
    let len1 = mip1.len() * 4;
    let len2 = base_texels.len() * 4;

    let mut raw = Vec::with_capacity(len0 + len1 + len2);
    for px in &mip0 {
        raw.extend_from_slice(px);
    }
    for px in &mip1 {
        raw.extend_from_slice(px);
    }
    for px in base_texels {
        raw.extend_from_slice(px);
    }

    crate::pack::Picture {
        width: width * 4,
        height: height * 4,
        texels: raw,
        levels: vec![
            crate::pack::Level { offset: 0, width: width * 4, height: height * 4 },
            crate::pack::Level { offset: len0, width: width * 2, height: height * 2 },
            crate::pack::Level { offset: len0 + len1, width, height },
        ],
    }
}
"#;
