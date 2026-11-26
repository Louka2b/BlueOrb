#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui::{self, Align, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Layout, Margin, RichText, Stroke, StrokeKind, TextStyle, Vec2};

const EMBEDDED_BLUEORB_TOML: &str = include_str!("../../../blueorb.toml");
const EMBEDDED_OVERRIDES_C: &str = include_str!("../../../overrides/blueorb_overrides.c");

#[derive(Clone, Debug)]
struct RomSummary {
    valid_ncch: bool,
    is_alpha_sapphire: bool,
    product_code: String,
    title_id_hex: String,
    size_mb: f64,
    note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BuildPhase {
    Idle,
    Running,
    Success(PathBuf),
    Failed(String),
}

struct SharedJob {
    phase: BuildPhase,
    progress: f32,
    step_label: String,
    logs: Vec<String>,
}

impl Default for SharedJob {
    fn default() -> Self {
        Self {
            phase: BuildPhase::Idle,
            progress: 0.0,
            step_label: "Idle".into(),
            logs: Vec::new(),
        }
    }
}

struct InstallerApp {
    project_root: PathBuf,
    cli_bin: Option<PathBuf>,
    rom_path: Option<PathBuf>,
    rom_info: Option<RomSummary>,
    hovering_drop: bool,

    aspect_idx: usize,  // 0 = 5:3, 1 = 16:9, 2 = 21:9, 3 = 32:9
    dual_windows: bool, // false by default
    resolution: u32,    // 5 by default
    no_outlines: bool,  // true by default
    lang_idx: usize,    // 0 = English
    portable: bool,
    show_logs: bool,
    status_note: Option<String>,

    job: Arc<Mutex<SharedJob>>,
}

impl InstallerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_clean_dark_theme(&cc.egui_ctx);

        let (project_root, cli_bin) = locate_workspace_and_cli();
        let initial_rom = auto_detect_rom(&project_root);
        let rom_info = initial_rom.as_deref().map(inspect_3ds_rom);

        Self {
            project_root,
            cli_bin,
            rom_path: initial_rom,
            rom_info,
            hovering_drop: false,
            aspect_idx: 0,
            dual_windows: false,
            resolution: 5,
            no_outlines: true,
            lang_idx: 0,
            portable: false,
            show_logs: false,
            status_note: None,
            job: Arc::new(Mutex::new(SharedJob::default())),
        }
    }

    fn set_rom(&mut self, path: PathBuf) {
        self.rom_info = Some(inspect_3ds_rom(&path));
        self.rom_path = Some(path);
    }

    fn clean_build_cache(&mut self) {
        let mut dirs = vec![
            self.project_root.join("build").join("recomp_sdk").join("3dsrecomp").join("target"),
            self.project_root.join("build").join("recomp_sdk").join("zakuro").join("target"),
            self.project_root.join("build").join("native_port").join("target"),
            self.project_root.join("tools").join("launcher").join("target"),
            self.project_root.join("asm"),
            self.project_root.join("baserom"),
        ];
        if cfg!(not(target_os = "windows")) {
            dirs.push(self.project_root.join("tools").join("installer").join("target"));
        }
        let mut freed = 0u64;
        for dir in dirs {
            if dir.exists() {
                freed += dir_byte_size(&dir);
                let _ = fs::remove_dir_all(&dir);
            }
        }
        self.status_note = Some(format!(
            "Cleaned {:.1} MB of temporary build cache (saves kept intact).",
            freed as f64 / (1024.0 * 1024.0)
        ));
    }

    fn start_installation(&mut self, ctx: egui::Context) {
        let Some(rom) = self.rom_path.clone() else {
            return;
        };
        if !rom.is_file() {
            let mut j = self.job.lock().unwrap();
            j.phase = BuildPhase::Failed(format!("File not found: {}", rom.display()));
            return;
        }

        if !self.project_root.join("blueorb.toml").exists() {
            let _ = fs::create_dir_all(self.project_root.join("overrides"));
            let _ = fs::write(self.project_root.join("blueorb.toml"), EMBEDDED_BLUEORB_TOML);
            let _ = fs::write(
                self.project_root.join("overrides").join("blueorb_overrides.c"),
                EMBEDDED_OVERRIDES_C,
            );
        }

        let aspect_flag = match self.aspect_idx {
            0 => "--5:3",
            1 => "--16:9",
            2 => "--21:9",
            _ => "--32:9",
        };
        let lang_code = match self.lang_idx {
            0 => "en",
            1 => "fr",
            2 => "ja",
            3 => "de",
            4 => "es",
            _ => "it",
        };
        let res_str = self.resolution.to_string();
        let dual_windows = self.dual_windows;
        let no_outlines = self.no_outlines;
        let portable = self.portable;
        let root = self.project_root.clone();
        let cli_bin = self.cli_bin.clone();
        let job = self.job.clone();
        self.show_logs = true;
        self.status_note = None;

        {
            let mut j = job.lock().unwrap();
            j.phase = BuildPhase::Running;
            j.progress = 0.05;
            j.step_label = "Preparing AOT recompilation…".into();
            j.logs.clear();
        }

        thread::spawn(move || {
            let mut cmd = build_cli_command(&root, cli_bin.as_deref());
            cmd.current_dir(&root)
                .arg("app")
                .arg("--rom")
                .arg(&rom)
                .arg("--res")
                .arg(&res_str)
                .arg("--lang")
                .arg(lang_code)
                .arg(aspect_flag);

            if dual_windows {
                cmd.arg("--dual-windows");
            } else {
                cmd.arg("--single-window");
            }
            if !no_outlines {
                cmd.arg("--with-outlines");
            }
            if portable {
                cmd.arg("--portable");
            }

            prepend_standard_paths(&mut cmd);
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(err) => {
                    let mut j = job.lock().unwrap();
                    j.phase = BuildPhase::Failed(format!("Could not start builder tool: {err}"));
                    ctx.request_repaint();
                    return;
                }
            };

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            let job_err = job.clone();
            let ctx_err = ctx.clone();
            let err_handle = thread::spawn(move || {
                if let Some(err_pipe) = stderr {
                    let reader = BufReader::new(err_pipe);
                    for line in reader.lines().map_while(Result::ok) {
                        let trimmed = line.trim().to_string();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let mut j = job_err.lock().unwrap();
                        update_progress_from_line(&mut j, &trimmed);
                        j.logs.push(trimmed);
                        if j.logs.len() > 400 {
                            j.logs.remove(0);
                        }
                        ctx_err.request_repaint();
                    }
                }
            });

            if let Some(out_pipe) = stdout {
                let reader = BufReader::new(out_pipe);
                for line in reader.lines().map_while(Result::ok) {
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let mut j = job.lock().unwrap();
                    update_progress_from_line(&mut j, &trimmed);
                    j.logs.push(trimmed);
                    if j.logs.len() > 400 {
                        j.logs.remove(0);
                    }
                    ctx.request_repaint();
                }
            }

            let _ = err_handle.join();
            match child.wait() {
                Ok(status) if status.success() => {
                    let out_bundle = expected_output_bundle(&root);
                    let mut j = job.lock().unwrap();
                    j.progress = 1.0;
                    j.step_label = "Completed".into();
                    j.phase = BuildPhase::Success(out_bundle);
                }
                Ok(status) => {
                    let mut j = job.lock().unwrap();
                    let last_err = j
                        .logs
                        .iter()
                        .rev()
                        .find(|l| l.contains("error") || l.contains("erreur") || l.contains("Échec"))
                        .cloned()
                        .unwrap_or_else(|| format!("Exit status {status}"));
                    j.phase = BuildPhase::Failed(last_err);
                }
                Err(e) => {
                    let mut j = job.lock().unwrap();
                    j.phase = BuildPhase::Failed(e.to_string());
                }
            }
            ctx.request_repaint();
        });
    }
}

impl eframe::App for InstallerApp {
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.hovering_drop = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(first) = dropped.into_iter().next() {
            self.set_rom(first);
        }

        let (phase, progress, step_label, logs_snapshot) = {
            let j = self.job.lock().unwrap();
            (j.phase.clone(), j.progress, j.step_label.clone(), j.logs.clone())
        };
        let is_running = matches!(phase, BuildPhase::Running);

        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(Color32::from_rgb(18, 20, 26))
                    .inner_margin(Margin::symmetric(32, 26)),
            )
            .show(root_ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let full_w = ui.available_width();

                    // Clean header bar
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("BlueOrb — Native AOT Setup")
                                    .size(21.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new("Static AOT recompilation into an independent native desktop binary (no external emulator).")
                                    .size(13.5)
                                    .color(Color32::from_rgb(146, 154, 170)),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add_enabled(!is_running, egui::Button::new(RichText::new("Clean Cache").size(13.0)))
                                .clicked()
                            {
                                self.clean_build_cache();
                            }
                        });
                    });

                    if let Some(note) = &self.status_note {
                        ui.add_space(8.0);
                        ui.label(RichText::new(note).size(13.0).color(Color32::from_rgb(134, 239, 172)));
                    }

                    ui.add_space(18.0);

                    // Zone ROM
                    let rom_ok = self.rom_info.as_ref().is_some_and(|r| r.valid_ncch);
                    let border = if self.hovering_drop {
                        Stroke::new(1.5_f32, Color32::from_rgb(59, 130, 246))
                    } else if rom_ok && self.rom_info.as_ref().is_some_and(|r| r.is_alpha_sapphire) {
                        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 222, 128, 95))
                    } else {
                        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20))
                    };

                    egui::Frame::NONE
                        .fill(if self.hovering_drop {
                            Color32::from_rgb(24, 32, 48)
                        } else {
                            Color32::from_rgb(25, 28, 36)
                        })
                        .stroke(border)
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::same(20))
                        .show(ui, |ui| {
                            let cw = (full_w - 42.0).max(220.0);
                            ui.set_width(cw);

                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_max_width((cw - 190.0).max(160.0));
                                    match (&self.rom_path, &self.rom_info) {
                                        (Some(path), Some(info)) => {
                                            let fname = path
                                                .file_name()
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| path.display().to_string());
                                            ui.label(RichText::new(fname).size(16.0).strong().color(Color32::WHITE));
                                            ui.add_space(3.0);
                                            let col = if info.valid_ncch {
                                                Color32::from_rgb(134, 239, 172)
                                            } else {
                                                Color32::from_rgb(252, 165, 165)
                                            };
                                            ui.label(
                                                RichText::new(format!(
                                                    "{}  ·  {:.0} MB  ·  {} ({})",
                                                    info.note, info.size_mb, info.product_code, info.title_id_hex
                                                ))
                                                .size(13.0)
                                                .color(col),
                                            );
                                        }
                                        _ => {
                                            ui.label(
                                                RichText::new("No ROM selected")
                                                    .size(16.0)
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            );
                                            ui.add_space(3.0);
                                            ui.label(
                                                RichText::new("Drop a decrypted .3ds / .cci file into this window or click Browse.")
                                                    .size(13.5)
                                                    .color(Color32::from_rgb(146, 154, 170)),
                                            );
                                        }
                                    }
                                });

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add_enabled_ui(!is_running, |ui| {
                                        if ui
                                            .add_sized([165.0, 40.0], egui::Button::new(RichText::new("Browse…").size(14.5)))
                                            .clicked()
                                        {
                                            if let Some(picked) = rfd::FileDialog::new()
                                                .set_title("Select Decrypted Nintendo 3DS ROM (.3ds)")
                                                .add_filter("Nintendo 3DS ROM", &["3ds", "cci", "cxi"])
                                                .pick_file()
                                            {
                                                self.set_rom(picked);
                                            }
                                        }
                                    });
                                });
                            });
                        });

                    ui.add_space(16.0);

                    // Initial Options
                    egui::Frame::NONE
                        .fill(Color32::from_rgb(25, 28, 36))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 18)))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::same(20))
                        .show(ui, |ui| {
                            let cw = (full_w - 42.0).max(220.0);
                            ui.set_width(cw);

                            ui.add_enabled_ui(!is_running, |ui| {
                                ui.label(RichText::new("Default Display Aspect Ratio").size(15.0).strong().color(Color32::WHITE));
                                ui.add_space(6.0);
                                let aspects = [
                                    (0usize, "5:3 Native"),
                                    (1, "16:9 Widescreen"),
                                    (2, "21:9 Ultrawide"),
                                    (3, "32:9 Super Wide"),
                                ];
                                if let Some(a) = segmented_row(ui, cw, 40.0, self.aspect_idx, &aspects, "inst_aspect") {
                                    self.aspect_idx = a;
                                }

                                ui.add_space(14.0);
                                ui.label(RichText::new("Internal 3D Resolution").size(15.0).strong().color(Color32::WHITE));
                                ui.add_space(6.0);
                                let res_opts = [(3u32, "3x (720p)"), (5, "5x (1440p)"), (8, "8x (4K)")];
                                if let Some(r) = segmented_row(ui, cw, 40.0, self.resolution, &res_opts, "inst_res") {
                                    self.resolution = r;
                                }

                                ui.add_space(14.0);
                                ui.label(RichText::new("System Language").size(15.0).strong().color(Color32::WHITE));
                                ui.add_space(6.0);
                                let langs = [
                                    (0usize, "English"),
                                    (1, "Français"),
                                    (2, "日本語"),
                                    (3, "Deutsch"),
                                    (4, "Español"),
                                    (5, "Italiano"),
                                ];
                                if let Some(l) = segmented_row(ui, cw, 38.0, self.lang_idx, &langs, "inst_lang_seg") {
                                    self.lang_idx = l;
                                }

                                ui.add_space(10.0);
                                ui.separator();
                                ui.add_space(8.0);
                                toggle_row(
                                    ui,
                                    cw,
                                    "Remove black outlines (HD)",
                                    "Disables 3DS dark toon outlines around 3D models (toggle anytime in Esc menu).",
                                    &mut self.no_outlines,
                                );

                                ui.add_space(8.0);
                                toggle_row(
                                    ui,
                                    cw,
                                    "Start in Dual Window mode",
                                    "Opens the touch screen in a detached second window (toggle anytime with F7).",
                                    &mut self.dual_windows,
                                );

                                ui.add_space(8.0);
                                toggle_row(
                                    ui,
                                    cw,
                                    "Portable full copy",
                                    "Copies ROM data files directly into dist/ instead of hardlinking.",
                                    &mut self.portable,
                                );
                            });
                        });

                    ui.add_space(18.0);

                    let can_install = rom_ok && !is_running;
                    match &phase {
                        BuildPhase::Idle | BuildPhase::Failed(_) => {
                            if primary_cta(ui, full_w, 48.0, "Compile and build native application", can_install) {
                                self.start_installation(ctx.clone());
                            }
                            if let BuildPhase::Failed(err) = &phase {
                                ui.add_space(10.0);
                                egui::Frame::NONE
                                    .fill(Color32::from_rgb(52, 22, 26))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(248, 113, 113, 85)))
                                    .corner_radius(CornerRadius::same(10))
                                    .inner_margin(Margin::same(14))
                                    .show(ui, |ui| {
                                        ui.set_width((full_w - 30.0).max(180.0));
                                        ui.label(RichText::new(err).size(13.5).color(Color32::from_rgb(254, 202, 202)));
                                    });
                            }
                        }
                        BuildPhase::Running => {
                            egui::Frame::NONE
                                .fill(Color32::from_rgb(25, 28, 36))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 22)))
                                .corner_radius(CornerRadius::same(12))
                                .inner_margin(Margin::same(18))
                                .show(ui, |ui| {
                                    ui.set_width((full_w - 38.0).max(200.0));
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(RichText::new(&step_label).size(14.5).strong().color(Color32::WHITE));
                                    });
                                    ui.add_space(8.0);
                                    ui.add(egui::ProgressBar::new(progress).animate(true).show_percentage());
                                });
                        }
                        BuildPhase::Success(bundle_path) => {
                            egui::Frame::NONE
                                .fill(Color32::from_rgb(22, 36, 30))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 222, 128, 90)))
                                .corner_radius(CornerRadius::same(12))
                                .inner_margin(Margin::same(18))
                                .show(ui, |ui| {
                                    let cw = (full_w - 38.0).max(200.0);
                                    ui.set_width(cw);
                                    ui.label(
                                        RichText::new("Application ready")
                                            .size(16.5)
                                            .strong()
                                            .color(Color32::from_rgb(134, 239, 172)),
                                    );
                                    ui.label(
                                        RichText::new(bundle_path.display().to_string())
                                            .size(12.5)
                                            .color(Color32::from_rgb(180, 195, 186)),
                                    );
                                    ui.add_space(12.0);
                                    let is_macos_bundle = cfg!(target_os = "macos")
                                        && bundle_path.extension().and_then(|e| e.to_str()) == Some("app");
                                    let w1 = cw * if is_macos_bundle { 0.34 } else { 0.48 };
                                    let rem_cols = if is_macos_bundle { 3.0 } else { 2.0 };
                                    let w2 = (cw - w1 - 10.0 * rem_cols) / rem_cols;
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 10.0;
                                        if primary_cta(ui, w1, 42.0, "Launch Game", true) {
                                            launch_built_game(bundle_path);
                                        }
                                        if is_macos_bundle
                                            && ui
                                                .add_sized([w2, 42.0], egui::Button::new("Install to /Applications"))
                                                .clicked()
                                        {
                                            if let Some(note) = install_macos_bundle_to_applications(bundle_path) {
                                                self.status_note = Some(note);
                                            }
                                        }
                                        if ui
                                            .add_sized([w2, 42.0], egui::Button::new("Open Folder"))
                                            .clicked()
                                        {
                                            reveal_folder(bundle_path.parent().unwrap_or(bundle_path));
                                        }
                                        if ui
                                            .add_sized([w2, 42.0], egui::Button::new("Rebuild"))
                                            .clicked()
                                        {
                                            self.start_installation(ctx.clone());
                                        }
                                    });
                                });
                        }
                    }

                    if !logs_snapshot.is_empty() {
                        ui.add_space(12.0);
                        let lbl = if self.show_logs {
                            "Hide build log"
                        } else {
                            "Show build log"
                        };
                        if ui.button(RichText::new(lbl).size(13.0)).clicked() {
                            self.show_logs = !self.show_logs;
                        }
                        if self.show_logs {
                            ui.add_space(6.0);
                            egui::Frame::NONE
                                .fill(Color32::from_rgb(12, 14, 18))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 16)))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(Margin::same(12))
                                .show(ui, |ui| {
                                    ui.set_width((full_w - 26.0).max(180.0));
                                    egui::ScrollArea::vertical()
                                        .max_height(130.0)
                                        .stick_to_bottom(true)
                                        .show(ui, |ui| {
                                            for line in &logs_snapshot {
                                                ui.label(
                                                    RichText::new(line)
                                                        .monospace()
                                                        .size(12.0)
                                                        .color(Color32::from_rgb(176, 184, 198)),
                                                );
                                            }
                                        });
                                });
                        }
                    }
                });
            });
    }
}

fn segmented_row<T: Copy + PartialEq>(
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
        ui.painter().rect_filled(outer_rect, CornerRadius::same(9), Color32::from_rgb(14, 16, 21));
        ui.painter().rect_stroke(
            outer_rect,
            CornerRadius::same(9),
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 16)),
            StrokeKind::Inside,
        );
    }
    let inner = outer_rect.shrink(4.0);
    let n = options.len().max(1) as f32;
    let gap = 4.0;
    let item_w = ((inner.width() - gap * (n - 1.0)) / n).max(12.0);
    for (idx, &(val, label)) in options.iter().enumerate() {
        let x0 = inner.left() + (idx as f32) * (item_w + gap);
        let item_rect = egui::Rect::from_min_size(egui::pos2(x0, inner.top()), Vec2::new(item_w, inner.height()));
        let resp = ui.interact(item_rect, ui.make_persistent_id((id_salt, idx)), egui::Sense::click());
        let selected = val == current;
        if ui.is_rect_visible(item_rect) {
            if selected {
                ui.painter().rect_filled(item_rect, CornerRadius::same(6), Color32::from_rgb(37, 99, 235));
            } else if resp.hovered() {
                ui.painter().rect_filled(item_rect, CornerRadius::same(6), Color32::from_rgb(28, 32, 42));
            }
            let col = if selected || resp.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(172, 180, 194)
            };
            ui.painter().text(
                item_rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                FontId::proportional(13.5),
                col,
            );
        }
        if resp.clicked() && !selected {
            picked = Some(val);
        }
    }
    picked
}

fn toggle_row(ui: &mut egui::Ui, width: f32, title: &str, subtitle: &str, value: &mut bool) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 46.0), egui::Sense::click());
    if resp.clicked() {
        *value = !*value;
    }
    if ui.is_rect_visible(rect) {
        let switch_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 46.0, rect.center().y - 13.0),
            Vec2::new(46.0, 26.0),
        );
        let track = if *value {
            Color32::from_rgb(37, 99, 235)
        } else if resp.hovered() {
            Color32::from_rgb(56, 62, 76)
        } else {
            Color32::from_rgb(42, 47, 58)
        };
        ui.painter().rect_filled(switch_rect, CornerRadius::same(13), track);
        let knob_x = if *value { switch_rect.right() - 13.0 } else { switch_rect.left() + 13.0 };
        ui.painter().circle_filled(egui::pos2(knob_x, switch_rect.center().y), 9.5, Color32::WHITE);

        let text_clip = egui::Rect::from_min_max(rect.min, egui::pos2(switch_rect.left() - 14.0, rect.bottom()));
        let tp = ui.painter().with_clip_rect(text_clip);
        tp.text(
            egui::pos2(rect.left(), rect.top() + 12.0),
            egui::Align2::LEFT_CENTER,
            title,
            FontId::proportional(14.5),
            Color32::from_rgb(238, 240, 245),
        );
        tp.text(
            egui::pos2(rect.left(), rect.top() + 33.0),
            egui::Align2::LEFT_CENTER,
            subtitle,
            FontId::proportional(12.5),
            Color32::from_rgb(142, 150, 166),
        );
    }
}

fn primary_cta(ui: &mut egui::Ui, width: f32, height: f32, label: &str, enabled: bool) -> bool {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    if ui.is_rect_visible(rect) {
        let fill = if !enabled {
            Color32::from_rgb(36, 41, 52)
        } else if resp.is_pointer_button_down_on() {
            Color32::from_rgb(29, 78, 216)
        } else if resp.hovered() {
            Color32::from_rgb(59, 130, 246)
        } else {
            Color32::from_rgb(37, 99, 235)
        };
        let text_col = if enabled {
            Color32::WHITE
        } else {
            Color32::from_rgb(122, 130, 146)
        };
        ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(15.5),
            text_col,
        );
    }
    enabled && resp.clicked()
}

fn apply_clean_dark_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.global_style_mut(|style| {
        style.text_styles.insert(TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Body, FontId::new(14.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Button, FontId::new(14.5, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional));
        style.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);

        let mut v = egui::Visuals::dark();
        v.window_fill = Color32::from_rgb(20, 22, 28);
        v.panel_fill = Color32::from_rgb(18, 20, 26);
        v.faint_bg_color = Color32::from_rgb(26, 29, 36);
        v.extreme_bg_color = Color32::from_rgb(13, 15, 19);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(8);
        v.slider_trailing_fill = true;
        v.interact_cursor = Some(CursorIcon::PointingHand);
        v.selection.bg_fill = Color32::from_rgb(37, 99, 235);

        v.widgets.noninteractive.bg_fill = Color32::from_rgb(26, 29, 36);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(235, 238, 244));
        v.widgets.noninteractive.corner_radius = CornerRadius::same(8);

        v.widgets.inactive.bg_fill = Color32::from_rgb(34, 38, 48);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(34, 38, 48);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 16));
        v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(235, 238, 244));
        v.widgets.inactive.corner_radius = CornerRadius::same(8);

        v.widgets.hovered.bg_fill = Color32::from_rgb(46, 52, 66);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(46, 52, 66);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 36));
        v.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
        v.widgets.hovered.corner_radius = CornerRadius::same(8);

        v.widgets.active.bg_fill = Color32::from_rgb(37, 99, 235);
        v.widgets.active.weak_bg_fill = Color32::from_rgb(37, 99, 235);
        v.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(96, 165, 250));
        v.widgets.active.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
        v.widgets.active.corner_radius = CornerRadius::same(8);

        style.visuals = v;
    });
}

fn inspect_3ds_rom(path: &Path) -> RomSummary {
    let size_mb = fs::metadata(path).map(|m| m.len() as f64 / (1024.0 * 1024.0)).unwrap_or(0.0);
    let Ok(mut f) = File::open(path) else {
        return RomSummary {
            valid_ncch: false,
            is_alpha_sapphire: false,
            product_code: "?".into(),
            title_id_hex: "?".into(),
            size_mb,
            note: "Lecture impossible".into(),
        };
    };

    let mut head = [0u8; 0x200];
    if f.read_exact(&mut head).is_err() {
        return RomSummary {
            valid_ncch: false,
            is_alpha_sapphire: false,
            product_code: "?".into(),
            title_id_hex: "?".into(),
            size_mb,
            note: "Fichier invalide".into(),
        };
    }

    let ncch_offset: u64 = if &head[0x100..0x104] == b"NCSD" {
        let part0_media = u32::from_le_bytes([head[0x120], head[0x121], head[0x122], head[0x123]]) as u64;
        part0_media * 0x200
    } else {
        0
    };

    let mut ncch = [0u8; 0x200];
    if f.seek(SeekFrom::Start(ncch_offset)).is_err() || f.read_exact(&mut ncch).is_err() {
        return RomSummary {
            valid_ncch: false,
            is_alpha_sapphire: false,
            product_code: "?".into(),
            title_id_hex: "?".into(),
            size_mb,
            note: "Partition NCCH introuvable".into(),
        };
    }

    if &ncch[0x100..0x104] != b"NCCH" {
        return RomSummary {
            valid_ncch: false,
            is_alpha_sapphire: false,
            product_code: "?".into(),
            title_id_hex: "?".into(),
            size_mb,
            note: "Format non reconnu".into(),
        };
    }

    let tid = u64::from_le_bytes([
        ncch[0x118], ncch[0x119], ncch[0x11A], ncch[0x11B], ncch[0x11C], ncch[0x11D], ncch[0x11E], ncch[0x11F],
    ]);
    let prod_end = ncch[0x150..0x160].iter().position(|&b| b == 0).unwrap_or(16);
    let product_code = String::from_utf8_lossy(&ncch[0x150..0x150 + prod_end]).into_owned();
    let encrypted = (ncch[0x18F] & 0x04) == 0;
    let is_as = tid == 0x0004_0000_0011_C500 || product_code.contains("ECLA");

    let note = if encrypted {
        "Encrypted ROM (provide a decrypted dump)"
    } else if is_as {
        "Verified Pokémon Alpha Sapphire ROM"
    } else {
        "Decrypted Nintendo 3DS ROM"
    };

    RomSummary {
        valid_ncch: !encrypted,
        is_alpha_sapphire: is_as,
        product_code,
        title_id_hex: format!("{tid:016X}"),
        size_mb,
        note: note.into(),
    }
}

fn locate_workspace_and_cli() -> (PathBuf, Option<PathBuf>) {
    let cli_name = if cfg!(target_os = "windows") {
        "blueorb-cli.exe"
    } else {
        "blueorb-cli"
    };
    let repo_cli_name = if cfg!(target_os = "windows") {
        "blueorb.exe"
    } else {
        "blueorb"
    };

    let exe_path = env::current_exe().ok();
    let exe_dir = exe_path.as_ref().and_then(|p| p.parent().map(Path::to_path_buf));

    let bundled_cli = exe_dir.as_ref().map(|d| d.join(cli_name)).filter(|p| p.is_file());

    let mut candidates = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd);
    }
    if let Some(ed) = &exe_dir {
        candidates.push(ed.clone());
    }

    for start in candidates {
        let mut cur: Option<&Path> = Some(&start);
        for _ in 0..7 {
            let Some(dir) = cur else { break };
            if dir.join("blueorb.toml").is_file() {
                let target_cli = dir.join("tools").join("blueorb").join("target").join("release").join(repo_cli_name);
                let found_cli = bundled_cli.clone().or_else(|| target_cli.is_file().then_some(target_cli));
                return (dir.to_path_buf(), found_cli);
            }
            cur = dir.parent();
        }
    }

    let fallback_root = if let Some(ed) = &exe_dir {
        if ed.ends_with("Contents/MacOS") {
            ed.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .unwrap_or(ed)
                .to_path_buf()
        } else {
            ed.clone()
        }
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    (fallback_root, bundled_cli)
}

fn auto_detect_rom(root: &Path) -> Option<PathBuf> {
    let default_as = root.join("Pokemon Alpha Sapphire (USA) (En,Ja,Fr,De,Es,It,Ko) (Rev 2).3ds");
    if default_as.is_file() {
        return Some(default_as);
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if matches!(ext.to_ascii_lowercase().as_str(), "3ds" | "cci" | "cxi") {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn build_cli_command(root: &Path, cli_bin: Option<&Path>) -> Command {
    if let Some(bin) = cli_bin {
        return Command::new(bin);
    }
    #[cfg(target_os = "windows")]
    {
        let bat = root.join("blueorb.bat");
        if bat.is_file() {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(bat);
            return c;
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let sh = root.join("blueorb");
        if sh.is_file() {
            return Command::new(sh);
        }
    }
    let mut cargo = Command::new("cargo");
    cargo.args([
        "run",
        "--release",
        "--manifest-path",
        &root.join("tools/blueorb/Cargo.toml").to_string_lossy(),
        "--",
    ]);
    cargo
}

fn prepend_standard_paths(cmd: &mut Command) {
    let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
    let current = env::var("PATH").unwrap_or_default();
    let mut extra = Vec::new();
    if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
        let h = PathBuf::from(home);
        extra.push(h.join(".cargo").join("bin").display().to_string());
    }
    #[cfg(target_os = "macos")]
    {
        extra.push("/opt/homebrew/bin".into());
        extra.push("/usr/local/bin".into());
    }
    if !extra.is_empty() {
        cmd.env("PATH", format!("{}{}{}", extra.join(sep), sep, current));
    }
}

fn update_progress_from_line(job: &mut SharedJob, line: &str) {
    if line.contains("[1/5]") {
        job.progress = 0.12;
        job.step_label = "Step 1/5 — Reading metadata & game icon".into();
    } else if line.contains("[2/5]") {
        job.progress = 0.25;
        job.step_label = "Step 2/5 — Preparing AOT static recompiler".into();
    } else if line.contains("[3/5]") {
        job.progress = 0.48;
        job.step_label = "Step 3/5 — Translating 75,219 functions to native code".into();
    } else if line.contains("[4/5]") {
        job.progress = 0.80;
        job.step_label = "Step 4/5 — Linking native standalone executable".into();
    } else if line.contains("[5/5]") {
        job.progress = 0.94;
        job.step_label = "Step 5/5 — Packaging bundle in dist/".into();
    }
}

fn dir_byte_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .map(|entries| entries.flatten().map(|e| dir_byte_size(&e.path())).sum())
        .unwrap_or(0)
}

fn install_macos_bundle_to_applications(bundle_path: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let name = bundle_path.file_name()?;
        let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let copy_to = |dir: &Path| -> Option<PathBuf> {
            let dest = dir.join(name);
            if dest.exists() && fs::remove_dir_all(&dest).is_err() {
                return None;
            }
            let ok = Command::new("/usr/bin/ditto")
                .arg(bundle_path)
                .arg(&dest)
                .status()
                .is_ok_and(|s| s.success());
            ok.then_some(dest)
        };
        let dest = copy_to(Path::new("/Applications")).or_else(|| {
            let user_apps = home.join("Applications");
            let _ = fs::create_dir_all(&user_apps);
            copy_to(&user_apps)
        });
        return match dest {
            Some(p) => Some(format!("Installed to {}", p.display())),
            None => Some("Could not copy bundle to /Applications.".into()),
        };
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle_path;
        None
    }
}

fn render_diveball_window_icon() -> egui::IconData {
    let size = 64usize;
    let szf = size as f32;
    let mut rgba = vec![0u8; size * size * 4];
    for y in 0..size {
        let uy = ((y as f32 + 0.5) / szf) * 2.0 - 1.0;
        for x in 0..size {
            let ux = ((x as f32 + 0.5) / szf) * 2.0 - 1.0;
            let d = (ux * ux + uy * uy).sqrt();
            if d > 0.90 {
                continue;
            }
            let aa = ((0.90 - d) * szf * 0.5).clamp(0.0, 1.0);
            let nx = ux / 0.90;
            let ny = -uy / 0.90;
            let nz = (1.0 - (nx * nx + ny * ny).min(1.0)).sqrt();
            let py = ny * 0.98 - nz * (-0.19);
            let pz = ny * (-0.19) + nz * 0.98;
            let clasp = pz.clamp(-1.0, 1.0).acos();
            let shade = (0.4 + 0.6 * (nx * -0.4 + ny * 0.56 + nz * 0.72).max(0.0)).clamp(0.0, 1.0);
            let (r, g, b) = if clasp < 0.12 {
                (56.0, 189.0, 255.0)
            } else if clasp < 0.22 {
                (240.0 * shade, 244.0 * shade, 250.0 * shade)
            } else if clasp < 0.29 || py.abs() < 0.07 {
                (24.0, 28.0, 38.0)
            } else if py > 0.0 {
                let wave = 0.23 + 0.08 * (nx * 5.0).cos();
                if (py - wave).abs() < 0.05 {
                    (210.0, 242.0, 255.0)
                } else if py > 0.55 {
                    (66.0 * shade + 20.0, 170.0 * shade + 30.0, 248.0)
                } else {
                    (18.0 * shade + 8.0, 90.0 * shade + 24.0, 214.0 * shade + 40.0)
                }
            } else {
                (238.0 * shade, 242.0 * shade, 248.0 * shade)
            };
            let idx = (y * size + x) * 4;
            rgba[idx] = r.clamp(0.0, 255.0) as u8;
            rgba[idx + 1] = g.clamp(0.0, 255.0) as u8;
            rgba[idx + 2] = b.clamp(0.0, 255.0) as u8;
            rgba[idx + 3] = (aa * 255.0) as u8;
        }
    }
    egui::IconData { rgba, width: size as u32, height: size as u32 }
}

fn expected_output_bundle(root: &Path) -> PathBuf {
    let dist = root.join("dist");
    if cfg!(target_os = "macos") {
        dist.join("Pokemon Alpha Sapphire.app")
    } else {
        dist.join("Pokemon Alpha Sapphire")
    }
}

fn launch_built_game(bundle_path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let bin = bundle_path.join("Contents").join("MacOS").join("PokemonAlphaSapphire");
        if bin.is_file() {
            let _ = Command::new(bin).spawn();
        } else {
            let _ = Command::new("open").arg(bundle_path).spawn();
        }
    }
    #[cfg(target_os = "windows")]
    {
        let exe = bundle_path.join("Pokemon Alpha Sapphire.exe");
        let _ = Command::new(exe).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let bin = bundle_path.join("PokemonAlphaSapphire");
        let _ = Command::new(bin).spawn();
    }
}

fn reveal_folder(folder: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("open").arg(folder).spawn();
    #[cfg(target_os = "windows")]
    let _ = Command::new("explorer").arg(folder).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("xdg-open").arg(folder).spawn();
}

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("BlueOrb — Native AOT Setup")
        .with_icon(Arc::new(render_diveball_window_icon()))
        .with_inner_size([820.0, 620.0])
        .with_min_inner_size([640.0, 500.0])
        .with_drag_and_drop(true);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "BlueOrb Installer",
        options,
        Box::new(|cc| Ok(Box::new(InstallerApp::new(cc)))),
    )
}
