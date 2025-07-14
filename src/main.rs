#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backup;
mod helpers;
mod restore;

use backup::backup_gui;
use helpers::CompressionLevel;
use helpers::Progress;
use helpers::build_human_tree;
use helpers::collect_paths;
use helpers::fix_skip;
use helpers::load_icon_image;
use helpers::parse_fingerprint;
use helpers::render_tree;
use restore::restore_backup;

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
};

use eframe::egui;
use rfd::FileDialog;
use serde::{Deserialize, Serialize};

type RestoreMsg = Result<(FolderTreeNode, PathBuf), String>; // Result type for restore operations

// Define the structure of the backup template
#[derive(Serialize, Deserialize)]
struct BackupTemplate {
    paths: Vec<PathBuf>,
}

// Implement a function to fix paths that are skipped
#[derive(Default)]
struct FolderTreeNode {
    children: HashMap<String, FolderTreeNode>,
    checked: bool,
    is_file: bool,
}

#[allow(dead_code)]
// Function to build a folder tree from a list of paths
fn build_tree_from_paths(paths: &[String]) -> FolderTreeNode {
    // Create a root node for the folder tree
    let mut root = FolderTreeNode::default();
    for path in paths {
        // Split the path into components and build the tree
        let mut current = &mut root;
        for part in Path::new(path).components() {
            // Convert the component to a string and insert it into the tree
            let key = part.as_os_str().to_string_lossy().to_string();
            current = current
                .children
                .entry(key.clone())
                .or_insert(FolderTreeNode {
                    // Initialize the new node with an empty children map, checked state, and is_file flag
                    children: HashMap::new(),
                    checked: true,
                    is_file: false,
                });
        }
        current.is_file = true;
    }
    root
}

fn main() -> Result<(), eframe::Error> {
    // Initialize the logger
    println!("[DEBUG] main: Starting application");

    dotenv::dotenv().ok();
    // Load environment variables from .env file if present
    println!("[DEBUG] .env loaded (if present)");

    let icon = load_icon_image();
    // Load the application icon
    println!("[DEBUG] Icon loaded");

    let options = eframe::NativeOptions {
        // Set the initial window size
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([410.0, 450.0])
            .with_resizable(false)
            .with_icon(icon),
        ..Default::default()
    };
    println!("[DEBUG] NativeOptions configured");

    println!("[DEBUG] Launching GUI with run_native");
    eframe::run_native(
        "Konserve",
        options,
        Box::new(|_cc| {
            println!("[DEBUG] GUIApp::default() instantiated");
            Ok(Box::new(GUIApp::default()))
        }),
    )
}

// Define the main tab enum to switch between Home and Settings
#[derive(PartialEq)]
enum MainTab {
    Home,
    Settings,
}

// Define the main application structure
struct GUIApp {
    status: Arc<Mutex<String>>,
    selected_folders: Vec<PathBuf>,
    template_editor: bool,
    template_paths: Vec<PathBuf>,
    restore_editor: bool,
    restore_zip_path: Option<PathBuf>,
    restore_tree: FolderTreeNode,
    _saved_path_map: Option<HashMap<String, PathBuf>>,
    backup_progress: Option<Progress>,
    restore_progress: Option<Progress>,
    restore_opening: bool,
    restore_rx: Option<mpsc::Receiver<RestoreMsg>>,
    tab: MainTab,
    compression_enabled: bool,
    compression_level: CompressionLevel,
    default_backup_location: Option<PathBuf>,
}

// Implement the Default trait for GUIApp to initialize the application state
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
            _saved_path_map: None,
            backup_progress: None,
            restore_progress: None,
            restore_opening: false,
            restore_rx: None,
            tab: MainTab::Home,
            compression_enabled: false,
            compression_level: CompressionLevel::Normal,
            default_backup_location: None,
        }
    }
}

// Implement the eframe::App trait for GUIApp to handle the application logic
impl eframe::App for GUIApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // === Tabs ===
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.tab == MainTab::Home, "Home")
                    .clicked()
                {
                    self.tab = MainTab::Home;
                }
                if ui
                    .selectable_label(self.tab == MainTab::Settings, "Settings")
                    .clicked()
                {
                    self.tab = MainTab::Settings;
                }
            });

            if self.template_editor {
                ui.label("Editing Template");

                ui.add_space(4.0);

                // === Template Path Editor ===
                egui::ScrollArea::vertical()
                    .max_height(285.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut to_remove = None;

                        // --- Path Rows ---
                        for (i, path) in self.template_paths.iter_mut().enumerate() {
                            let mut path_str = path.display().to_string();

                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    [240.0, 20.0],
                                    egui::TextEdit::singleline(&mut path_str),
                                );

                                // Update the path if the text edit changes
                                if path_str != path.display().to_string() {
                                    *path = PathBuf::from(path_str.clone());
                                }

                                // Check if the path exists and display a checkmark or cross
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
                        // --- Save to JSON ---
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

            if self.restore_editor {
                ui.label("Restore Selection");

                ui.add_space(4.0);

                // === Restore Tree ===
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        let mut current_path = vec![];
                        render_tree(ui, &mut current_path, &mut self.restore_tree)
                    });

                ui.separator();

                if ui.button("Restore selected").clicked() {
                    if let Some(zip_path) = &self.restore_zip_path.clone() {
                        // Collect selected paths from the restore tree
                        let selected = collect_paths(&self.restore_tree);
                        let zip_path = zip_path.clone();
                        let status = self.status.clone();

                        let progress = Progress::default();
                        self.restore_progress = Some(progress.clone());
                        self.restore_opening = false;

                        thread::spawn(move || {
                            // Show spinner right away
                            if let Err(e) =
                                restore_backup(&zip_path, Some(selected), status.clone(), &progress)
                            {
                                *status.lock().unwrap() = format!("❌ Restore failed: {e}");
                            }
                        });

                        self.restore_editor = false;
                    }
                }

                if ui.button("Cancel").clicked() {
                    self.restore_editor = false;
                    self.restore_zip_path = None;
                    self.restore_tree = FolderTreeNode::default();
                }

                return;
            }

            match self.tab {
                // Everything home related ui
                MainTab::Home => {
                    if let Some(finished_msg) =
                        self.restore_rx.as_ref().and_then(|rx| rx.try_recv().ok())
                    {
                        // === Restore Result Handling ===
                        match finished_msg {
                            Ok((mut tree, zip)) => {
                                // NEW: mark everything checked
                                fn check_all(n: &mut FolderTreeNode) {
                                    n.checked = true;
                                    for c in n.children.values_mut() {
                                        check_all(c);
                                    }
                                }
                                check_all(&mut tree);

                                self.restore_tree = tree;
                                self.restore_zip_path = Some(zip);
                                self.restore_editor = true;
                            }
                            Err(e) => {
                                *self.status.lock().unwrap() = format!("Failed: {e}");
                            }
                        }
                        self.restore_rx = None;
                    }

                    ui.heading("Konserve");
                    ui.separator();

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

                        // === Selected Items List ===
                        // This will allow users to see all selected folders
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

                        // --- Clear Selection ---
                        if ui.button("Clear All").clicked() {
                            self.selected_folders.clear();
                        }
                    }

                    ui.separator();

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            let btn_size = egui::vec2(95.0, 17.0);
                            // Load and Save Template buttons
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

                                                // Sort and deduplicate the paths
                                                self.selected_folders = valid;
                                                // Sort the paths
                                                let msg = if skipped.is_empty() {
                                                    "✅ Template loaded".into()
                                                } else {
                                                    // If there are skipped paths, show how many were skipped
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

                            // Save Template button
                            ui.add_sized(btn_size, egui::Button::new("Save Template"))
                                .clicked()
                                .then(|| {
                                    // Open file dialog to save the template
                                    if let Some(path) =
                                        FileDialog::new().add_filter("JSON", &["json"]).save_file()
                                    {
                                        // --- Build Template Struct ---
                                        let template = BackupTemplate {
                                            paths: self.selected_folders.clone(),
                                        };

                                        // --- Save to JSON ---
                                        if let Ok(json) = serde_json::to_string_pretty(&template) {
                                            if fs::write(&path, json).is_ok() {
                                                *self.status.lock().unwrap() =
                                                    "✅ Template saved.".into();
                                            } else {
                                                *self.status.lock().unwrap() =
                                                    "❌ Failed to write template.".into();
                                            }
                                        }
                                    }
                                });
                        });
                        // === Backup / Restore Buttons ===
                        ui.vertical(|ui| {
                            let btn_size = egui::vec2(95.0, 17.0);
                            // Create Backup button
                            ui.add_sized(btn_size, egui::Button::new("Create Backup"))
                                .clicked()
                                .then(|| {
                                    // Check if any folders are selected
                                    let folders = self.selected_folders.clone();
                                    let status = self.status.clone();

                                    if folders.is_empty() {
                                        *status.lock().unwrap() = "❌ Nothing selected.".into();
                                        return;
                                    }

                                    *status.lock().unwrap() = "Packing into .tar".into();

                                    let progress = Progress::default();
                                    self.backup_progress = Some(progress.clone());

                                    thread::spawn(move || {
                                        if let Some(out_dir) = FileDialog::new()
                                            .set_title("Choose backup destination")
                                            .pick_folder()
                                        {
                                            match backup_gui(&folders, &out_dir, &progress) {
                                                Ok(path) => {
                                                    *status.lock().unwrap() = format!(
                                                        "✅ Backup created:\n{}",
                                                        path.display()
                                                    );
                                                }
                                                Err(e) => {
                                                    *status.lock().unwrap() =
                                                        format!("❌ Backup failed: {e}");
                                                }
                                            }
                                        } else {
                                            *status.lock().unwrap() = "❌ Cancelled.".into();
                                        }
                                    });
                                });
                            // Restore Backup button
                            ui.add_sized(btn_size, egui::Button::new("Restore Backup"))
                                .clicked()
                                .then(|| {
                                    let status = self.status.clone();
                                    // Check if any folders are selected
                                    if let Some(zip_file) =
                                        FileDialog::new().add_filter("tar", &["tar"]).pick_file()
                                    {
                                        // If a zip file is selected, start the restore process
                                        self.restore_opening = true;
                                        *status.lock().unwrap() = "Opening archive…".into();

                                        // Create a progress channel
                                        // This will be used to send the result of the restore operation
                                        let (tx, rx) = mpsc::channel::<RestoreMsg>();
                                        self.restore_rx = Some(rx);

                                        thread::spawn(move || {
                                            // Parse the fingerprint of the zip file
                                            let result: RestoreMsg = parse_fingerprint(&zip_file)
                                                .map(|(entries, map)| {
                                                    (
                                                        build_human_tree(entries, map),
                                                        zip_file.clone(),
                                                    )
                                                });
                                            // Send the result back to the main thread
                                            let _ = tx.send(result);
                                        });
                                    }
                                });
                        });
                    });

                    if self.restore_opening {
                        // Show a spinner while the restore archive is being opened
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(16.0)); // 16 px is default
                            ui.label("Opening archive…");
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(30));
                    }

                    for opt in [&mut self.backup_progress, &mut self.restore_progress]
                        .into_iter()
                        .enumerate()
                    {
                        let (i, p_opt) = opt;
                        if let Some(p) = p_opt {
                            let pct = p.get(); // 101 = done
                            match p.get() {
                                0..=100 => {
                                    // Show progress bar and percentage
                                    ui.add(
                                        egui::ProgressBar::new((p.get() as f32) / 100.0)
                                            .fill(egui::Color32::from_rgb(80, 160, 240))
                                            .desired_height(6.0)
                                            .animate(true)
                                            .desired_width(ui.available_width()),
                                    );
                                    ui.add_space(1.0);
                                    ui.label(format!("{pct}%"));
                                    ui.add_space(1.0);
                                    let progress_status = if i == 0 {
                                        "Backing up..."
                                    } else {
                                        "Restoring..."
                                    };
                                    ui.label(progress_status);
                                    ctx.request_repaint_after(std::time::Duration::from_millis(4));
                                }
                                _ => {
                                    *p_opt = None;
                                }
                            }
                        }
                    }
                }

                // Settings tab
                MainTab::Settings => {
                    ui.heading("Settings");
                    ui.separator();

                    let btn_size = egui::vec2(95.0, 17.0);
                    ui.add_sized(btn_size, egui::Button::new("Edit Template"))
                        .clicked()
                        .then(|| {
                            // Open the template editor
                            if let Some(path) =
                                FileDialog::new().add_filter("JSON", &["json"]).pick_file()
                            {
                                // --- Load Template File ---
                                if let Ok(data) = fs::read_to_string(&path) {
                                    if let Ok(template) =
                                        serde_json::from_str::<BackupTemplate>(&data)
                                    {
                                        // --- Parse & Open Editor ---
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

                    ui.separator();

                    ui.checkbox(&mut self.compression_enabled, "Enable Compression (WIP)");

                    if self.compression_enabled {
                        egui::ComboBox::from_label("Compression Level (WIP)")
                            .selected_text(match self.compression_level {
                                CompressionLevel::Fast => "Fast",
                                CompressionLevel::Normal => "Normal",
                                CompressionLevel::Maximum => "Maximum",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.compression_level,
                                    CompressionLevel::Fast,
                                    "Fast",
                                );
                                ui.selectable_value(
                                    &mut self.compression_level,
                                    CompressionLevel::Normal,
                                    "Normal",
                                );
                                ui.selectable_value(
                                    &mut self.compression_level,
                                    CompressionLevel::Maximum,
                                    "Maximum",
                                );
                            });
                    }

                    let mut loc_str = self
                        .default_backup_location
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();

                    ui.separator();

                    ui.label("Default backup location:");
                    ui.horizontal(|ui| {
                        ui.add_sized([240.0, 20.0], egui::TextEdit::singleline(&mut loc_str));

                        use std::path::Path;
                        if !loc_str.is_empty() {
                            if Path::new(&loc_str).is_dir() {
                                ui.label("✅").on_hover_text("This path exists");
                            } else {
                                ui.label("❌").on_hover_text("This path does not exist");
                            }
                        }

                        if ui.button("Browse").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                loc_str = folder.display().to_string();
                            }
                        }

                        if !loc_str.is_empty() && ui.button("Clear").clicked() {
                            loc_str.clear();
                        }
                    });

                    // === Wiring Placeholder ===
                    // When logic is implemented (in helpers.rs),
                    // use self.default_backup_location in your backup functions.

                    // --- Save/Load Config ---
                    // serialization/deserialization

                    let should_update = match &self.default_backup_location {
                        Some(p) => loc_str != p.display().to_string(),
                        None => !loc_str.is_empty(),
                    };
                    if should_update {
                        if !loc_str.is_empty() {
                            self.default_backup_location = Some(std::path::PathBuf::from(&loc_str));
                            // TODO: Call a helper here to persist the setting, e.g.:
                            // helpers::save_seggings(self);
                        } else {
                            self.default_backup_location = None;
                            // TODO: Call a helper here to clear the saved location if needed.
                        }
                    }
                }
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}
