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
            step_label: "En attente".into(),
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
    dual_windows: bool, // false par défaut
    resolution: u32,    // 5 par défaut
    no_outlines: bool,  // true par défaut
    lang_idx: usize,    // 0 = Français
    portable: bool,
    show_logs: bool,

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
            job: Arc::new(Mutex::new(SharedJob::default())),
        }
    }

    fn set_rom(&mut self, path: PathBuf) {
        self.rom_info = Some(inspect_3ds_rom(&path));
        self.rom_path = Some(path);
    }

    fn start_installation(&mut self, ctx: egui::Context) {
        let Some(rom) = self.rom_path.clone() else {
            return;
        };
        if !rom.is_file() {
            let mut j = self.job.lock().unwrap();
            j.phase = BuildPhase::Failed(format!("Fichier introuvable : {}", rom.display()));
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
            0 => "fr",
            1 => "en",
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

        {
            let mut j = job.lock().unwrap();
            j.phase = BuildPhase::Running;
            j.progress = 0.05;
            j.step_label = "Préparation de la recompilation…".into();
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
                    j.phase = BuildPhase::Failed(format!("Impossible de lancer l'outil : {err}"));
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
                    j.step_label = "Terminé".into();
                    j.phase = BuildPhase::Success(out_bundle);
                }
                Ok(status) => {
                    let mut j = job.lock().unwrap();
                    let last_err = j
                        .logs
                        .iter()
                        .rev()
                        .find(|l| l.contains("erreur") || l.contains("error") || l.contains("Échec"))
                        .cloned()
                        .unwrap_or_else(|| format!("Code de sortie {status}"));
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

                    // En-tête sobre
                    ui.label(
                        RichText::new("Pokémon Saphir Alpha — Installation Native")
                            .size(23.0)
                            .strong()
                            .color(Color32::WHITE),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("Recompilation statique AOT vers un exécutable natif autonome (sans émulateur).")
                            .size(14.0)
                            .color(Color32::from_rgb(146, 154, 170)),
                    );

                    ui.add_space(20.0);

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
                                                    "{}  ·  {:.0} Mo  ·  {} ({})",
                                                    info.note, info.size_mb, info.product_code, info.title_id_hex
                                                ))
                                                .size(13.0)
                                                .color(col),
                                            );
                                        }
                                        _ => {
                                            ui.label(
                                                RichText::new("Aucune ROM sélectionnée")
                                                    .size(16.0)
                                                    .strong()
                                                    .color(Color32::WHITE),
                                            );
                                            ui.add_space(3.0);
                                            ui.label(
                                                RichText::new("Glissez votre fichier .3ds décrypté dans cette fenêtre ou cliquez sur Parcourir.")
                                                    .size(13.5)
                                                    .color(Color32::from_rgb(146, 154, 170)),
                                            );
                                        }
                                    }
                                });

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add_enabled_ui(!is_running, |ui| {
                                        if ui
                                            .add_sized([165.0, 40.0], egui::Button::new(RichText::new("Parcourir…").size(14.5)))
                                            .clicked()
                                        {
                                            if let Some(picked) = rfd::FileDialog::new()
                                                .set_title("Sélectionner la ROM Pokémon Saphir Alpha (.3ds)")
                                                .add_filter("ROM Nintendo 3DS", &["3ds", "cci", "cxi"])
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

                    // Options initiales
                    egui::Frame::NONE
                        .fill(Color32::from_rgb(25, 28, 36))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 18)))
                        .corner_radius(CornerRadius::same(12))
                        .inner_margin(Margin::same(20))
                        .show(ui, |ui| {
                            let cw = (full_w - 42.0).max(220.0);
                            ui.set_width(cw);

                            ui.add_enabled_ui(!is_running, |ui| {
                                ui.label(RichText::new("Format d'affichage par défaut").size(15.0).strong().color(Color32::WHITE));
                                ui.add_space(6.0);
                                let aspects = [
                                    (0usize, "5:3 Original"),
                                    (1, "16:9 Large"),
                                    (2, "21:9 Ultra-Wide"),
                                    (3, "32:9 Super Large"),
                                ];
                                if let Some(a) = segmented_row(ui, cw, 40.0, self.aspect_idx, &aspects, "inst_aspect") {
                                    self.aspect_idx = a;
                                }

                                ui.add_space(14.0);
                                ui.label(RichText::new("Résolution interne 3D").size(15.0).strong().color(Color32::WHITE));
                                ui.add_space(6.0);
                                let res_opts = [(3u32, "3x (720p)"), (5, "5x (1440p)"), (8, "8x (4K)")];
                                if let Some(r) = segmented_row(ui, cw, 40.0, self.resolution, &res_opts, "inst_res") {
                                    self.resolution = r;
                                }

                                ui.add_space(14.0);
                                ui.label(RichText::new("Langue du jeu").size(15.0).strong().color(Color32::WHITE));
                                ui.add_space(6.0);
                                let langs = [
                                    (0usize, "Français"),
                                    (1, "English"),
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
                                    "Supprimer les contours noirs (HD)",
                                    "Retire l'effet trait noir 3DS autour des modèles 3D (modifiable en jeu avec Échap).",
                                    &mut self.no_outlines,
                                );

                                ui.add_space(8.0);
                                toggle_row(
                                    ui,
                                    cw,
                                    "Séparer les deux écrans dès le démarrage",
                                    "Ouvre l'écran tactile sur une deuxième fenêtre (modifiable à tout moment avec F7 ou Échap).",
                                    &mut self.dual_windows,
                                );

                                ui.add_space(8.0);
                                toggle_row(
                                    ui,
                                    cw,
                                    "Copie complète portable",
                                    "Copie les fichiers de données directement dans dist/ pour pouvoir déplacer le jeu.",
                                    &mut self.portable,
                                );
                            });
                        });

                    ui.add_space(18.0);

                    let can_install = rom_ok && !is_running;
                    match &phase {
                        BuildPhase::Idle | BuildPhase::Failed(_) => {
                            if primary_cta(ui, full_w, 48.0, "Compiler et installer l'application native", can_install) {
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
                                        RichText::new("Application prête")
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
                                    let w1 = cw * 0.48;
                                    let w2 = (cw - w1 - 20.0) / 2.0;
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 10.0;
                                        if primary_cta(ui, w1, 42.0, "Lancer le jeu", true) {
                                            launch_built_game(bundle_path);
                                        }
                                        if ui
                                            .add_sized([w2, 42.0], egui::Button::new("Ouvrir le dossier"))
                                            .clicked()
                                        {
                                            reveal_folder(bundle_path.parent().unwrap_or(bundle_path));
                                        }
                                        if ui
                                            .add_sized([w2, 42.0], egui::Button::new("Recompiler"))
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
                            "Masquer le journal de compilation"
                        } else {
                            "Afficher le journal de compilation"
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
        "ROM chiffrée (fournissez une ROM décryptée)"
    } else if is_as {
        "ROM Pokémon Saphir Alpha vérifiée"
    } else {
        "ROM 3DS décryptée"
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
        job.step_label = "Étape 1/5 — Lecture des métadonnées et de l'icône".into();
    } else if line.contains("[2/5]") {
        job.progress = 0.25;
        job.step_label = "Étape 2/5 — Préparation du recompilateur AOT".into();
    } else if line.contains("[3/5]") {
        job.progress = 0.48;
        job.step_label = "Étape 3/5 — Traduction AOT des 75 219 fonctions en C natif".into();
    } else if line.contains("[4/5]") {
        job.progress = 0.80;
        job.step_label = "Étape 4/5 — Édition de liens de l'exécutable natif".into();
    } else if line.contains("[5/5]") {
        job.progress = 0.94;
        job.step_label = "Étape 5/5 — Création de l'application dans dist/".into();
    }
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
        .with_title("BlueOrb — Installation de Pokémon Saphir Alpha")
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
