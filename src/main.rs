#![windows_subsystem = "windows"]

mod backup;
mod restore;
mod helpers;

use helpers::fix_skip;
use backup::backup_gui;
use restore::restore_backup;

use std::{ fs, path::PathBuf, sync::{ Arc, Mutex }, thread };

use eframe::egui;
use egui::IconData;
use rfd::FileDialog;
use serde::{ Deserialize, Serialize };

// if !icon then fn fuck you
pub fn load_icon_image() -> Arc<IconData> {
    let image_bytes = include_bytes!("../assets/icon.png");
    let image = image
        ::load_from_memory(image_bytes)
        .expect("Icon image couldn't be loaded")
        .into_rgba8();
    let (w, h) = image.dimensions();

    Arc::new(IconData {
        rgba: image.into_raw(),
        width: w,
        height: h,
    })
}

#[derive(Serialize, Deserialize)]
struct BackupTemplate {
    paths: Vec<PathBuf>,
}

fn main() -> Result<(), eframe::Error> {
    dotenv::dotenv().ok();

    let icon = load_icon_image();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder
            ::default()
            .with_inner_size([410.0, 450.0])
            .with_resizable(false)
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "VanManen Backup Tool",
        options,
        Box::new(|_cc| Ok(Box::new(GUIApp::default())))
    )
}

struct GUIApp {
    status: Arc<Mutex<String>>,
    selected_folders: Vec<PathBuf>,
    template_editor: bool,
    template_paths: Vec<PathBuf>,
}

impl Default for GUIApp {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new("Waiting...".to_string())),
            selected_folders: Vec::new(),
            template_editor: false,
            template_paths: Vec::new(),
        }
    }
}

impl eframe::App for GUIApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("VanManen Backup Tool");
            ui.separator();

            if self.template_editor {
                ui.label("Editing Template");

                ui.add_space(4.0);

                egui::ScrollArea
                    ::vertical()
                    .max_height(290.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut to_remove = None;

                        for (i, path) in self.template_paths.iter_mut().enumerate() {
                            let mut path_str = path.display().to_string();

                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [240.0, 20.0],
                                    egui::TextEdit::singleline(&mut path_str)
                                );

                                if path_str != path.display().to_string() {
                                    *path = PathBuf::from(path_str.clone());
                                }

                                if path.exists() {
                                    ui.label("✅");
                                } else {
                                    ui.label("❌");
                                }

                                if ui.button("Browse").clicked() {
                                    if let Some(p) = FileDialog::new().pick_file() {
                                        *path = p;
                                    }
                                }

                                if ui.button("Remove").clicked() {
                                    to_remove = Some(i);
                                }
                            });
                        }

                        if let Some(i) = to_remove {
                            self.template_paths.remove(i);
                        }
                    });

                ui.separator();

                if ui.button("Add Path").clicked() {
                    self.template_paths.push(PathBuf::new());
                }

                if ui.button("Save Template").clicked() {
                    if let Some(path) = FileDialog::new().add_filter("JSON", &["json"]).save_file() {
                        let tpl = BackupTemplate {
                            paths: self.template_paths.clone(),
                        };

                        match serde_json::to_string_pretty(&tpl) {
                            Ok(json) => {
                                if fs::write(&path, json).is_ok() {
                                    *self.status.lock().unwrap() = "✅ Template saved".into();
                                    self.template_editor = false;
                                } else {
                                    *self.status.lock().unwrap() = "❌ Couldn't write file.".into();
                                }
                            }
                            Err(_) => {
                                *self.status.lock().unwrap() = "❌ Failed to serialize.".into();
                            }
                        }
                    }
                }

                if ui.button("Cancel").clicked() {
                    self.template_editor = false;
                }

                return;
            }

            ui.horizontal(|ui| {
                if ui.button("Add Folders").clicked() {
                    if let Some(folders) = FileDialog::new().pick_folders() {
                        self.selected_folders.extend(folders);
                        self.selected_folders.sort();
                        self.selected_folders.dedup();
                    }
                }

                if ui.button("Add Files").clicked() {
                    if let Some(files) = FileDialog::new().pick_files() {
                        self.selected_folders.extend(files);
                        self.selected_folders.sort();
                        self.selected_folders.dedup();
                    }
                }
            });

            ui.add_space(4.0);

            // selected paths
            let mut to_remove = None;
            egui::ScrollArea
                ::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, path) in self.selected_folders.iter().enumerate() {
                        if ui.button(path.display().to_string()).clicked() {
                            to_remove = Some(i);
                        }
                    }
                });

            if let Some(i) = to_remove {
                self.selected_folders.remove(i);
            }

            ui.add_space(4.0);

            if ui.button("Clear All").clicked() {
                self.selected_folders.clear();
            }

            ui.separator();

            if ui.button("Load Template").clicked() {
                if let Some(path) = FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                    if let Ok(data) = fs::read_to_string(&path) {
                        if let Ok(template) = serde_json::from_str::<BackupTemplate>(&data) {
                            let mut valid = Vec::new();
                            let mut skipped = Vec::new();

                            for p in template.paths {
                                match fix_skip(&p) {
                                    Some(adjusted) => valid.push(adjusted),
                                    None => skipped.push(p),
                                }
                            }

                            self.selected_folders = valid;

                            let msg = if skipped.is_empty() {
                                "✅ Template loaded".into()
                            } else {
                                format!("✅ Loaded with {} paths skipped", skipped.len())
                            };

                            *self.status.lock().unwrap() = msg;
                        } else {
                            *self.status.lock().unwrap() = "❌ Bad template format.".into();
                        }
                    }
                }
            }

            if ui.button("Save Template").clicked() {
                if let Some(path) = FileDialog::new().add_filter("JSON", &["json"]).save_file() {
                    let template = BackupTemplate {
                        paths: self.selected_folders.clone(),
                    };

                    if let Ok(json) = serde_json::to_string_pretty(&template) {
                        if fs::write(&path, json).is_ok() {
                            *self.status.lock().unwrap() = "✅ Template saved.".into();
                        } else {
                            *self.status.lock().unwrap() = "❌ Failed to write template.".into();
                        }
                    }
                }
            }

            if ui.button("Edit Template").clicked() {
                if let Some(path) = FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                    if let Ok(data) = fs::read_to_string(&path) {
                        if let Ok(template) = serde_json::from_str::<BackupTemplate>(&data) {
                            self.template_paths = template.paths
                                .into_iter()
                                .map(|p| fix_skip(&p).unwrap_or(p))
                                .collect();
                            self.template_editor = true;
                        } else {
                            *self.status.lock().unwrap() = "❌ Couldn't parse template.".into();
                        }
                    }
                }
            }

            if ui.button("Create Backup").clicked() {
                let folders = self.selected_folders.clone();
                let status = self.status.clone();

                if folders.is_empty() {
                    *status.lock().unwrap() = "❌ Nothing selected.".into();
                    return;
                }

                *status.lock().unwrap() = "Compressing into zip...".into();

                thread::spawn(move || {
                    if
                        let Some(out_dir) = FileDialog::new()
                            .set_title("Choose backup destination")
                            .pick_folder()
                    {
                        match backup_gui(&folders, &out_dir) {
                            Ok(path) => {
                                *status.lock().unwrap() = format!(
                                    "✅ Backup created:\n{}",
                                    path.display()
                                );
                            }
                            Err(e) => {
                                *status.lock().unwrap() = format!("❌ Backup failed: {}", e);
                            }
                        }
                    } else {
                        *status.lock().unwrap() = "❌ Cancelled.".into();
                    }
                });
            }

            if ui.button("Restore Backup").clicked() {
                let status = self.status.clone();

                *status.lock().unwrap() = "Starting restore...".into();

                thread::spawn(move || {
                    if
                        let Some(zip_file) = FileDialog::new()
                            .add_filter("zip", &["zip"])
                            .pick_file()
                    {
                        match restore_backup(&zip_file) {
                            Ok(_) => {
                                *status.lock().unwrap() = "✅ Restore complete.".into();
                            }
                            Err(e) => {
                                *status.lock().unwrap() = format!("❌ Restore failed: {}", e);
                            }
                        }
                    } else {
                        *status.lock().unwrap() = "❌ No file chosen.".into();
                    }
                });
            }

            ui.separator();
            ui.label(&*self.status.lock().unwrap());
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}
