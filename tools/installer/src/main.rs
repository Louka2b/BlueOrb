#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui::{self, Align, Color32, CursorIcon, Layout, Margin, RichText, Rounding, Stroke, Vec2};

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
            step_label: "Prêt à recompiler Pokémon Saphir Alpha.".into(),
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

    // Options de génération (par défaut : overlay classique 5:3 + 1 fenêtre)
    aspect_idx: usize,     // 0 = 5:3 Base, 1 = 16:9, 2 = 21:9 UW, 3 = 32:9
    dual_windows: bool,    // false par défaut (modifiable en jeu avec F7 ou Échap)
    resolution: u32,       // 5 = QHD par défaut
    no_outlines: bool,     // true par défaut
    lang_idx: usize,       // 0 = Français, 1 = English, 2 = 日本語, 3 = Deutsch, 4 = Español, 5 = Italiano
    portable: bool,
    show_logs: bool,

    job: Arc<Mutex<SharedJob>>,
}

impl InstallerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_sapphire_installer_theme(&cc.egui_ctx);

        let (project_root, cli_bin) = locate_workspace_and_cli();
        let initial_rom = auto_detect_rom(&project_root);
        let rom_info = initial_rom.as_deref().map(inspect_3ds_rom);

        Self {
            project_root,
            cli_bin,
            rom_path: initial_rom,
            rom_info,
            hovering_drop: false,
            aspect_idx: 0, // 5:3 Base par défaut !
            dual_windows: false,
            resolution: 5,
            no_outlines: true,
            lang_idx: 0, // Français
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
            j.phase = BuildPhase::Failed(format!("Fichier ROM introuvable : {}", rom.display()));
            return;
        }

        // Si lancé hors du dépôt Git, préparer un espace de travail autonome avec `blueorb.toml` et `overrides/`
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
            j.progress = 0.04;
            j.step_label = "Démarrage du pipeline de recompilation native AOT...".into();
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

            // S'assurer que Homebrew / Cargo / LLVM sont dans le PATH quand lancé par double-clic macOS/Linux/Windows
            prepend_standard_paths(&mut cmd);

            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(err) => {
                    let mut j = job.lock().unwrap();
                    j.phase = BuildPhase::Failed(format!("Impossible de lancer le compilateur BlueOrb : {err}"));
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
                    j.step_label = "Installation terminée ! Votre jeu natif est prêt.".into();
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
                        .unwrap_or_else(|| format!("Code de sortie {}", status));
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Détection du Glisser-Déposer (Drag & Drop) d'une ROM .3ds / .cci sur la fenêtre
        let hovered_any = ctx.input(|i| !i.raw.hovered_files.is_empty());
        self.hovering_drop = hovered_any;
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
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
                egui::Frame::none()
                    .fill(Color32::from_rgb(8, 15, 30))
                    .inner_margin(Margin::same(20.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let full_w = ui.available_width();

                    // En-tête BlueOrb Installer
                    ui.horizontal(|ui| {
                        egui::Frame::none()
                            .fill(Color32::from_rgb(20, 88, 178))
                            .rounding(Rounding::same(12.0))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(105, 200, 255)))
                            .inner_margin(Margin::symmetric(12.0, 9.0))
                            .show(ui, |ui| {
                                ui.label(RichText::new("◆").size(20.0).strong().color(Color32::WHITE));
                            });
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("BlueOrb — Installateur & Recompilateur Natif")
                                    .size(18.5)
                                    .strong()
                                    .color(Color32::WHITE),
                            );
                            ui.label(
                                RichText::new("Pokémon Saphir Alpha  •  100% Natif AOT (Zéro Émulateur)  •  macOS / Windows / Linux")
                                    .size(12.0)
                                    .color(Color32::from_rgb(138, 184, 232)),
                            );
                        });
                    });

                    ui.add_space(14.0);

                    // ÉTAPE 1 : Sélection ou Glisser-Déposer de la ROM .3ds
                    let drop_border = if self.hovering_drop {
                        Stroke::new(2.0_f32, Color32::from_rgb(100, 220, 255))
                    } else if self.rom_info.as_ref().is_some_and(|r| r.valid_ncch && r.is_alpha_sapphire) {
                        Stroke::new(1.2_f32, Color32::from_rgba_unmultiplied(72, 210, 155, 140))
                    } else {
                        Stroke::new(1.2_f32, Color32::from_rgba_unmultiplied(74, 156, 245, 95))
                    };

                    egui::Frame::none()
                        .fill(if self.hovering_drop {
                            Color32::from_rgb(18, 44, 84)
                        } else {
                            Color32::from_rgb(13, 25, 48)
                        })
                        .stroke(drop_border)
                        .rounding(Rounding::same(14.0))
                        .inner_margin(Margin::same(14.0))
                        .show(ui, |ui| {
                            ui.set_width((full_w - 30.0).max(200.0));
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("1. FICHIER ROM (.3DS / .CCI)")
                                        .size(11.0)
                                        .strong()
                                        .color(Color32::from_rgb(108, 188, 255)),
                                );
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add_enabled_ui(!is_running, |ui| {
                                        if ui
                                            .button(RichText::new("📂 Choisir la ROM…").strong())
                                            .clicked()
                                        {
                                            if let Some(picked) = rfd::FileDialog::new()
                                                .set_title("Sélectionnez votre ROM Pokémon Saphir Alpha (.3ds)")
                                                .add_filter("ROM Nintendo 3DS décryptée", &["3ds", "cci", "cxi"])
                                                .pick_file()
                                            {
                                                self.set_rom(picked);
                                            }
                                        }
                                    });
                                });
                            });

                            ui.add_space(6.0);

                            match (&self.rom_path, &self.rom_info) {
                                (Some(path), Some(info)) => {
                                    let file_name = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| path.display().to_string());
                                    ui.label(RichText::new(file_name).size(14.0).strong().color(Color32::WHITE));
                                    let status_col = if info.valid_ncch {
                                        Color32::from_rgb(98, 226, 164)
                                    } else {
                                        Color32::from_rgb(245, 175, 90)
                                    };
                                    ui.label(
                                        RichText::new(format!(
                                            "{}  •  {:.0} Mo  •  Code : {}  •  Title ID : {}",
                                            info.note, info.size_mb, info.product_code, info.title_id_hex
                                        ))
                                        .size(11.5)
                                        .color(status_col),
                                    );
                                }
                                _ => {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(8.0);
                                        ui.label(
                                            RichText::new("Glissez-déposez votre fichier .3ds décrypté ici")
                                                .size(14.0)
                                                .strong()
                                                .color(Color32::from_rgb(210, 228, 250)),
                                        );
                                        ui.label(
                                            RichText::new("ou cliquez sur « Choisir la ROM… » en haut à droite")
                                                .size(12.0)
                                                .color(Color32::from_rgb(138, 170, 210)),
                                        );
                                        ui.add_space(8.0);
                                    });
                                }
                            }
                        });

                    ui.add_space(12.0);

                    // ÉTAPE 2 : Options rapides préconfigurées
                    egui::Frame::none()
                        .fill(Color32::from_rgb(13, 25, 48))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(64, 142, 230, 75)))
                        .rounding(Rounding::same(14.0))
                        .inner_margin(Margin::same(14.0))
                        .show(ui, |ui| {
                            let inner_w = (full_w - 30.0).max(200.0);
                            ui.set_width(inner_w);
                            ui.label(
                                RichText::new("2. CONFIGURATION INITIALE (MODIFIABLE EN JEU VIA ÉCHAP / F6 / F7)")
                                    .size(11.0)
                                    .strong()
                                    .color(Color32::from_rgb(108, 188, 255)),
                            );
                            ui.add_space(8.0);

                            ui.add_enabled_ui(!is_running, |ui| {
                                // Sélecteur de format par défaut
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Format au démarrage :").size(12.5));
                                    let aspects = [
                                        "5:3 Classique (Base)",
                                        "16:9 Widescreen",
                                        "21:9 Ultra-Wide",
                                        "32:9 Super UW",
                                    ];
                                    for (idx, label) in aspects.iter().enumerate() {
                                        let sel = self.aspect_idx == idx;
                                        let btn = egui::Button::new(
                                            RichText::new(*label).size(11.5).strong().color(if sel {
                                                Color32::WHITE
                                            } else {
                                                Color32::from_rgb(190, 212, 238)
                                            }),
                                        )
                                        .fill(if sel {
                                            Color32::from_rgb(28, 116, 218)
                                        } else {
                                            Color32::from_rgb(18, 34, 64)
                                        });
                                        if ui.add(btn).clicked() {
                                            self.aspect_idx = idx;
                                        }
                                    }
                                });

                                ui.add_space(6.0);

                                // Résolution & Langue
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("Résolution 3D :").size(12.5));
                                    for (res, txt) in [(3u32, "3x (1080p)"), (5, "5x (QHD ★)"), (8, "8x (4K)")] {
                                        let sel = self.resolution == res;
                                        let btn = egui::Button::new(
                                            RichText::new(txt).size(11.5).strong().color(if sel {
                                                Color32::WHITE
                                            } else {
                                                Color32::from_rgb(190, 212, 238)
                                            }),
                                        )
                                        .fill(if sel {
                                            Color32::from_rgb(28, 116, 218)
                                        } else {
                                            Color32::from_rgb(18, 34, 64)
                                        });
                                        if ui.add(btn).clicked() {
                                            self.resolution = res;
                                        }
                                    }

                                    ui.add_space(12.0);
                                    ui.label(RichText::new("Langue :").size(12.5));
                                    let langs = ["Français", "English", "日本語", "Deutsch", "Español", "Italiano"];
                                    egui::ComboBox::from_id_source("installer_lang")
                                        .selected_text(langs[self.lang_idx])
                                        .width(105.0)
                                        .show_ui(ui, |ui| {
                                            for (i, l) in langs.iter().enumerate() {
                                                ui.selectable_value(&mut self.lang_idx, i, *l);
                                            }
                                        });
                                });

                                ui.add_space(6.0);

                                // Cases à cocher rapides
                                ui.horizontal_wrapped(|ui| {
                                    ui.checkbox(
                                        &mut self.no_outlines,
                                        "Sans contours noirs (Rendu HD net)",
                                    );
                                    ui.add_space(10.0);
                                    ui.checkbox(
                                        &mut self.dual_windows,
                                        "Détacher les 2 écrans dès le lancement",
                                    );
                                    ui.add_space(10.0);
                                    ui.checkbox(
                                        &mut self.portable,
                                        "Mode portable (copier la ROM dans dist/)",
                                    );
                                });
                            });
                        });

                    ui.add_space(14.0);

                    // ÉTAPE 3 : Bouton d'installation / Barre de progression / Lancement
                    let can_install = self.rom_path.is_some()
                        && self.rom_info.as_ref().is_some_and(|r| r.valid_ncch)
                        && !is_running;

                    match &phase {
                        BuildPhase::Idle | BuildPhase::Failed(_) => {
                            let cta = egui::Button::new(
                                RichText::new("⚡   Installer & Compiler Pokémon Saphir Alpha (Natif)")
                                    .size(15.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(if can_install {
                                Color32::from_rgb(24, 118, 224)
                            } else {
                                Color32::from_rgb(28, 44, 72)
                            })
                            .stroke(Stroke::new(
                                1.0_f32,
                                if can_install {
                                    Color32::from_rgb(120, 206, 255)
                                } else {
                                    Color32::from_rgb(60, 90, 130)
                                },
                            ))
                            .rounding(Rounding::same(12.0));

                            if ui.add_enabled(can_install, cta.min_size(Vec2::new(full_w, 44.0))).clicked() {
                                self.start_installation(ctx.clone());
                            }

                            if let BuildPhase::Failed(err) = &phase {
                                ui.add_space(8.0);
                                egui::Frame::none()
                                    .fill(Color32::from_rgba_unmultiplied(88, 22, 32, 220))
                                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(240, 95, 110)))
                                    .rounding(Rounding::same(10.0))
                                    .inner_margin(Margin::same(10.0))
                                    .show(ui, |ui| {
                                        ui.set_width((full_w - 22.0).max(180.0));
                                        ui.label(
                                            RichText::new(format!("Échec de l'installation : {err}"))
                                                .color(Color32::from_rgb(255, 205, 210))
                                                .strong(),
                                        );
                                    });
                            }
                        }
                        BuildPhase::Running => {
                            egui::Frame::none()
                                .fill(Color32::from_rgb(13, 28, 56))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(76, 174, 255)))
                                .rounding(Rounding::same(12.0))
                                .inner_margin(Margin::same(14.0))
                                .show(ui, |ui| {
                                    ui.set_width((full_w - 30.0).max(200.0));
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label(RichText::new(&step_label).strong().color(Color32::WHITE));
                                    });
                                    ui.add_space(6.0);
                                    ui.add(
                                        egui::ProgressBar::new(progress)
                                            .animate(true)
                                            .show_percentage(),
                                    );
                                });
                        }
                        BuildPhase::Success(bundle_path) => {
                            egui::Frame::none()
                                .fill(Color32::from_rgba_unmultiplied(14, 58, 52, 230))
                                .stroke(Stroke::new(1.2_f32, Color32::from_rgb(86, 228, 168)))
                                .rounding(Rounding::same(12.0))
                                .inner_margin(Margin::same(14.0))
                                .show(ui, |ui| {
                                    let inner_w = (full_w - 30.0).max(200.0);
                                    ui.set_width(inner_w);
                                    ui.label(
                                        RichText::new("✓ Application Native Générée avec Succès !")
                                            .size(15.0)
                                            .strong()
                                            .color(Color32::from_rgb(125, 245, 190)),
                                    );
                                    ui.label(
                                        RichText::new(bundle_path.display().to_string())
                                            .size(11.5)
                                            .color(Color32::from_rgb(195, 230, 220)),
                                    );
                                    ui.add_space(8.0);
                                    let btn_w = (inner_w - 16.0) / 3.0;
                                    ui.horizontal(|ui| {
                                        let play_btn = egui::Button::new(
                                            RichText::new("▶  Jouer maintenant").size(13.5).strong().color(Color32::WHITE),
                                        )
                                        .fill(Color32::from_rgb(22, 148, 104))
                                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(115, 245, 185)));
                                        if ui.add_sized([btn_w * 1.45, 36.0], play_btn).clicked() {
                                            launch_built_game(bundle_path);
                                        }

                                        if ui
                                            .add_sized([btn_w * 0.9, 36.0], egui::Button::new("📂 Ouvrir dist/"))
                                            .clicked()
                                        {
                                            let folder = bundle_path.parent().unwrap_or(bundle_path);
                                            reveal_folder(folder);
                                        }

                                        if ui
                                            .add_sized([btn_w * 0.65, 36.0], egui::Button::new("↻ Recompiler"))
                                            .clicked()
                                        {
                                            self.start_installation(ctx.clone());
                                        }
                                    });
                                });
                        }
                    }

                    // Journal de compilation dépliable
                    if !logs_snapshot.is_empty() {
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let toggle_lbl = if self.show_logs {
                                "▾ Masquer les détails techniques"
                            } else {
                                "▸ Afficher les détails techniques"
                            };
                            if ui.small_button(toggle_lbl).clicked() {
                                self.show_logs = !self.show_logs;
                            }
                        });
                        if self.show_logs {
                            egui::Frame::none()
                                .fill(Color32::from_rgb(5, 10, 20))
                                .stroke(Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(54, 110, 185, 70)))
                                .rounding(Rounding::same(8.0))
                                .inner_margin(Margin::same(10.0))
                                .show(ui, |ui| {
                                    ui.set_width((full_w - 22.0).max(180.0));
                                    egui::ScrollArea::vertical()
                                        .max_height(130.0)
                                        .stick_to_bottom(true)
                                        .show(ui, |ui| {
                                            for line in &logs_snapshot {
                                                ui.label(
                                                    RichText::new(line)
                                                        .monospace()
                                                        .size(11.0)
                                                        .color(Color32::from_rgb(176, 204, 236)),
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

fn apply_sapphire_installer_theme(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(11.0, 6.0);

    let mut v = egui::Visuals::dark();
    v.window_fill = Color32::from_rgb(9, 18, 36);
    v.panel_fill = Color32::from_rgb(8, 15, 30);
    v.faint_bg_color = Color32::from_rgb(14, 28, 54);
    v.extreme_bg_color = Color32::from_rgb(5, 10, 20);
    v.window_rounding = Rounding::same(14.0);
    v.menu_rounding = Rounding::same(10.0);
    v.slider_trailing_fill = true;
    v.interact_cursor = Some(CursorIcon::PointingHand);
    v.selection.bg_fill = Color32::from_rgb(26, 118, 220);

    v.widgets.noninteractive.bg_fill = Color32::from_rgb(14, 26, 50);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(224, 236, 252));
    v.widgets.noninteractive.rounding = Rounding::same(8.0);

    v.widgets.inactive.bg_fill = Color32::from_rgb(19, 36, 66);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(19, 36, 66);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(74, 150, 240, 70));
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(228, 240, 255));
    v.widgets.inactive.rounding = Rounding::same(8.0);

    v.widgets.hovered.bg_fill = Color32::from_rgb(28, 62, 114);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(28, 62, 114);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(96, 192, 255));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
    v.widgets.hovered.rounding = Rounding::same(8.0);

    v.widgets.active.bg_fill = Color32::from_rgb(32, 122, 224);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(32, 122, 224);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(150, 220, 255));
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
    v.widgets.active.rounding = Rounding::same(8.0);

    style.visuals = v;
    ctx.set_style(style);
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
            note: "Impossible de lire le fichier".into(),
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
            note: "Fichier trop court".into(),
        };
    }

    // Image NCSD (.3ds / .cci) -> Partition 0 (généralement à 0x4000) ou image CXI directe (`NCCH` à 0x100)
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
            note: "Partition NCCH illisible".into(),
        };
    }

    if &ncch[0x100..0x104] != b"NCCH" {
        return RomSummary {
            valid_ncch: false,
            is_alpha_sapphire: false,
            product_code: "?".into(),
            title_id_hex: "?".into(),
            size_mb,
            note: "Ce fichier n'est pas une ROM 3DS NCCH valide".into(),
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
        "⚠ ROM chiffrée détectée (utilisez une ROM décryptée)"
    } else if is_as {
        "✓ ROM Pokémon Saphir Alpha vérifiée"
    } else {
        "✓ ROM 3DS décryptée détectée"
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

    // 1. Chercher un binaire `blueorb-cli` embarqué juste à côté de l'installateur (cas `BlueOrb Installer.app` ou dossier release)
    let bundled_cli = exe_dir
        .as_ref()
        .map(|d| d.join(cli_name))
        .filter(|p| p.is_file());

    // 2. Remonter depuis `current_dir` puis `current_exe` pour trouver la racine du projet (`blueorb.toml`)
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
                let target_cli = dir
                    .join("tools")
                    .join("blueorb")
                    .join("target")
                    .join("release")
                    .join(repo_cli_name);
                let found_cli = bundled_cli
                    .clone()
                    .or_else(|| target_cli.is_file().then_some(target_cli));
                return (dir.to_path_buf(), found_cli);
            }
            cur = dir.parent();
        }
    }

    // 3. Mode autonome hors du dépôt : utiliser le dossier contenant l'installateur (.app parent sur macOS)
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
        job.step_label = "[1/5] Extraction de l'en-tête NCCH et de l'icône SMDH...".into();
    } else if line.contains("[2/5]") {
        job.progress = 0.25;
        job.step_label = "[2/5] Préparation du SDK 3dsrecomp & moteur HD/Ultra-Wide...".into();
    } else if line.contains("[3/5]") {
        job.progress = 0.45;
        job.step_label = "[3/5] Recompilation statique AOT des 75 219 fonctions ARM11 en C natif...".into();
    } else if line.contains("[4/5]") {
        job.progress = 0.78;
        job.step_label = "[4/5] Liaison statique de l'exécutable natif autonome...".into();
    } else if line.contains("[5/5]") {
        job.progress = 0.92;
        job.step_label = "[5/5] Assemblage & signature du bundle applicatif dans dist/...".into();
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
        let bin = bundle_path
            .join("Contents")
            .join("MacOS")
            .join("PokemonAlphaSapphire");
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
        .with_title("BlueOrb — Installateur Pokémon Saphir Alpha")
        .with_inner_size([670.0, 560.0])
        .with_min_inner_size([540.0, 460.0])
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
