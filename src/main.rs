#![windows_subsystem = "windows"]

mod backup;
mod helpers;
mod restore;

use backup::backup_gui;
use helpers::collect_paths;
use helpers::fix_skip;
use helpers::parse_fingerprint;
use restore::restore_backup;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

use eframe::egui;
use egui::CollapsingHeader;
use egui::IconData;
use rfd::FileDialog;
use serde::{Deserialize, Serialize};

fn build_human_tree(entries: Vec<String>, path_map: HashMap<String, PathBuf>) -> FolderTreeNode {
    let mut root = FolderTreeNode::default();

    for (uuid, original_path) in path_map {
        let parent_label = original_path
            .parent()
            .unwrap_or(&original_path)
            .display()
            .to_string();
        let item_name = original_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let parent_node = root
            .children
            .entry(parent_label.clone())
            .or_insert_with(FolderTreeNode::default);

        let _ = parent_node
            .children
            .entry(item_name.clone())
            .or_insert_with(FolderTreeNode::default);

        let dir_prefix = format!("{uuid}/");
        let is_dir_backup = entries.iter().any(|e| e.starts_with(&dir_prefix));

        if is_dir_backup {
            parent_node.children.get_mut(&item_name).unwrap().is_file = false;

            for tar_path in entries.iter().filter(|e| e.starts_with(&dir_prefix)) {
                let rest = tar_path[dir_prefix.len()..].trim_end_matches('/');
                if rest.is_empty() {
                    continue;
                }

                let mut cursor = parent_node.children.get_mut(&item_name).unwrap();

                for part in rest.split('/') {
                    cursor = cursor
                        .children
                        .entry(part.to_string())
                        .or_insert_with(FolderTreeNode::default);
                }
                cursor.is_file = true;
            }
        } else {
            parent_node.children.get_mut(&item_name).unwrap().is_file = true;
        }
    }

    root
}

// if !icon then fuck you
fn load_icon_image() -> Arc<IconData> {
    let image_bytes = include_bytes!("../assets/icon.png");
    let image = image::load_from_memory(image_bytes)
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

#[derive(Default)]
struct FolderTreeNode {
    children: HashMap<String, FolderTreeNode>,
    checked: bool,
    is_file: bool,
}

#[allow(dead_code)]
fn build_tree_from_paths(paths: &[String]) -> FolderTreeNode {
    let mut root = FolderTreeNode::default();
    for path in paths {
        let mut current = &mut root;
        for part in Path::new(path).components() {
            let key = part.as_os_str().to_string_lossy().to_string();
            current = current
                .children
                .entry(key.clone())
                .or_insert(FolderTreeNode {
                    children: HashMap::new(),
                    checked: true,
                    is_file: false,
                });
        }
        current.is_file = true;
    }
    root
}

fn set_all_checked(node: &mut FolderTreeNode, checked: bool) {
    node.checked = checked;
    for child in node.children.values_mut() {
        set_all_checked(child, checked);
    }
}

// fn update_folder_check_state(node: &mut FolderTreeNode) -> bool {
//     if node.is_file {
//         return node.checked;
//     }
//     let mut all_checked = true;
//     for child in node.children.values_mut() {
//         let child_checked = update_folder_check_state(child);
//         all_checked &= child_checked;
//     }
//
//     node.checked = all_checked;
//     all_checked
// }

fn render_tree(ui: &mut egui::Ui, path: &mut Vec<String>, node: &mut FolderTreeNode) {
    for (name, child) in node.children.iter_mut() {
        let mut label = name.clone();
        if !child.is_file {
            label.push('/');
        }
        path.push(name.clone());

        if child.children.is_empty() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut child.checked, "");
                ui.label(label);
            });
        } else {
            ui.horizontal(|ui| {
                if ui.checkbox(&mut child.checked, "").changed() {
                    set_all_checked(child, child.checked);
                }
                CollapsingHeader::new(label)
                    .default_open(false)
                    .show(ui, |ui| {
                        render_tree(ui, path, child);
                    });
            });
            child.checked = child.children.values().any(|c| c.checked);
        }

        path.pop();
    }
}

fn main() -> Result<(), eframe::Error> {
    dotenv::dotenv().ok();

    let icon = load_icon_image();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([410.0, 450.0])
            .with_resizable(false)
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "VanManen Backup Tool",
        options,
        Box::new(|_cc| Ok(Box::new(GUIApp::default()))),
    )
}

struct GUIApp {
    status: Arc<Mutex<String>>,
    selected_folders: Vec<PathBuf>,
    template_editor: bool,
    template_paths: Vec<PathBuf>,
    restore_editor: bool,
    restore_zip_path: Option<PathBuf>,
    restore_tree: FolderTreeNode,
    saved_path_map: Option<HashMap<String, PathBuf>>,
}

impl Default for GUIApp {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new("Waiting...".to_string())),
            selected_folders: Vec::new(),
            template_editor: false,
            template_paths: Vec::new(),
            restore_editor: false,
            restore_zip_path: None,
            restore_tree: FolderTreeNode::default(),
            saved_path_map: None,
        }
    }
}

impl eframe::App for GUIApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("VanManen Backup Tool");
            ui.separator();

            if self.restore_editor {
                ui.label("Restore Selection");

                ui.add_space(4.0);

                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        let mut current_path = vec![];
                        render_tree(ui, &mut current_path, &mut self.restore_tree)
                    });

                ui.separator();

                if ui.button("Restore selected").clicked() {
                    if let Some(zip_path) = &self.restore_zip_path.clone() {
                        let selected = collect_paths(&self.restore_tree);
                        let zip_path = zip_path.clone();
                        let status = self.status.clone();
                        self.restore_editor = false;

                        thread::spawn(move || {
                            if let Err(e) =
                                restore_backup(&zip_path, Some(selected), status.clone())
                            {
                                *status.lock().unwrap() = format!("❌ Restore failed: {}", e);
                            }
                        });
                    }
                }

                if ui.button("Cancel").clicked() {
                    self.restore_editor = false;
                    self.restore_zip_path = None;
                    self.restore_tree = FolderTreeNode::default();
                }

                return;
            }

            if self.template_editor {
                ui.label("Editing Template");

                ui.add_space(4.0);

                egui::ScrollArea::vertical()
                    .max_height(285.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut to_remove = None;

                        for (i, path) in self.template_paths.iter_mut().enumerate() {
                            let mut path_str = path.display().to_string();

                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [240.0, 20.0],
                                    egui::TextEdit::singleline(&mut path_str),
                                );

                                if path_str != path.display().to_string() {
                                    *path = PathBuf::from(path_str.clone());
                                }

                                if path.exists() {
                                    ui.label("✅").on_hover_text("This path exists");
                                } else {
                                    ui.label("❌").on_hover_text("This path does not exist");
                                }

                                if ui.button("Browse").clicked() {
                                    if let Some(p) = FileDialog::new().pick_folder() {
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
                    if let Some(path) = FileDialog::new().add_filter("JSON", &["json"]).save_file()
                    {
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
                ui.separator();
                ui.label("File names and extensions have to be manually typed in.");

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

            if !self.selected_folders.is_empty() {
                ui.add_space(4.0);

                // selected paths
                let mut to_remove = None;
                egui::ScrollArea::vertical()
                    .max_height(240.0)
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
            }

            ui.separator();

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let btn_size = egui::vec2(95.0, 17.0);
                    //template
                    ui.add_sized(btn_size, egui::Button::new("Load Template"))
                        .clicked()
                        .then(|| {
                            if let Some(path) =
                                FileDialog::new().add_filter("JSON", &["json"]).pick_file()
                            {
                                if let Ok(data) = fs::read_to_string(&path) {
                                    if let Ok(template) =
                                        serde_json::from_str::<BackupTemplate>(&data)
                                    {
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
                                            format!(
                                                "✅ Loaded with {} paths skipped",
                                                skipped.len()
                                            )
                                        };

                                        *self.status.lock().unwrap() = msg;
                                    } else {
                                        *self.status.lock().unwrap() =
                                            "❌ Bad template format.".into();
                                    }
                                }
                            }
                        });

                    ui.add_sized(btn_size, egui::Button::new("Save Template"))
                        .clicked()
                        .then(|| {
                            if let Some(path) =
                                FileDialog::new().add_filter("JSON", &["json"]).save_file()
                            {
                                let template = BackupTemplate {
                                    paths: self.selected_folders.clone(),
                                };

                                if let Ok(json) = serde_json::to_string_pretty(&template) {
                                    if fs::write(&path, json).is_ok() {
                                        *self.status.lock().unwrap() = "✅ Template saved.".into();
                                    } else {
                                        *self.status.lock().unwrap() =
                                            "❌ Failed to write template.".into();
                                    }
                                }
                            }
                        });

                    ui.add_sized(btn_size, egui::Button::new("Edit Template"))
                        .clicked()
                        .then(|| {
                            if let Some(path) =
                                FileDialog::new().add_filter("JSON", &["json"]).pick_file()
                            {
                                if let Ok(data) = fs::read_to_string(&path) {
                                    if let Ok(template) =
                                        serde_json::from_str::<BackupTemplate>(&data)
                                    {
                                        self.template_paths = template
                                            .paths
                                            .into_iter()
                                            .map(|p| fix_skip(&p).unwrap_or(p))
                                            .collect();
                                        self.template_editor = true;
                                    } else {
                                        *self.status.lock().unwrap() =
                                            "❌ Couldn't parse template.".into();
                                    }
                                }
                            }
                        });
                });

                ui.vertical(|ui| {
                    let btn_size = egui::vec2(95.0, 17.0);
                    //backup
                    ui.add_sized(btn_size, egui::Button::new("Create Backup"))
                        .clicked()
                        .then(|| {
                            let folders = self.selected_folders.clone();
                            let status = self.status.clone();

                            if folders.is_empty() {
                                *status.lock().unwrap() = "❌ Nothing selected.".into();
                                return;
                            }

                            *status.lock().unwrap() = "Packing into .tar".into();

                            thread::spawn(move || {
                                if let Some(out_dir) = FileDialog::new()
                                    .set_title("Choose backup destination")
                                    .pick_folder()
                                {
                                    match backup_gui(&folders, &out_dir) {
                                        Ok(path) => {
                                            *status.lock().unwrap() =
                                                format!("✅ Backup created:\n{}", path.display());
                                        }
                                        Err(e) => {
                                            *status.lock().unwrap() =
                                                format!("❌ Backup failed: {}", e);
                                        }
                                    }
                                } else {
                                    *status.lock().unwrap() = "❌ Cancelled.".into();
                                }
                            });
                        });

                    ui.add_sized(btn_size, egui::Button::new("Restore Backup"))
                        .clicked()
                        .then(|| {
                            let status = self.status.clone();

                            *status.lock().unwrap() = "Starting restore...".into();

                            if let Some(zip_file) =
                                FileDialog::new().add_filter("tar", &["tar"]).pick_file()
                            {
                                match parse_fingerprint(&zip_file) {
                                    Ok((entries, map)) => {
                                        self.restore_zip_path = Some(zip_file.clone());
                                        self.saved_path_map = Some(map.clone()); // ← store it
                                        self.restore_tree = build_human_tree(entries, map);
                                        // Walk the entire tree and set every `checked = true`
                                        fn check_all(node: &mut FolderTreeNode) {
                                            node.checked = true;
                                            for child in node.children.values_mut() {
                                                check_all(child);
                                            }
                                        }
                                        check_all(&mut self.restore_tree);
                                        self.restore_editor = true;
                                    }
                                    Err(e) => {
                                        *self.status.lock().unwrap() =
                                            format!("Failed to read backup: {}", e);
                                    }
                                }
                            }

                            // commented out for now in case of emergency, this may be added back.
                            // this is just old logic and if everything will work as intented then
                            // this may be left commented out or deleted complete
                            //
                            //thread::spawn(move || {
                            //    if
                            //        let Some(zip_file) = FileDialog::new()
                            //            .add_filter("zip", &["zip"])
                            //            .pick_file()
                            //    {
                            //        match restore_backup(&zip_file) {
                            //            Ok(_) => {
                            //                *status.lock().unwrap() = "✅ Restore complete.".into();
                            //            }
                            //            Err(e) => {
                            //                *status.lock().unwrap() =
                            //                    format!("❌ Restore failed: {}", e);
                            //            }
                            //        }
                            //    } else {
                            //        *status.lock().unwrap() = "❌ No file chosen.".into();
                            //    }
                            //});
                        });
                });
            });

            ui.separator();
            ui.label(&*self.status.lock().unwrap());
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}
