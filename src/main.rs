//! # Konserve
//!
//! Konserve is a simple desktop backup and restore tool
//!
//! - Create `.tar` archives, with optional `.tar.gz` compression (WIP)
//! - Select files and folders manually via reusable templates.
//! - Restore backups to their original destination with a tree view with selections
//!
//! Most settings related features are still work in progress.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backup;
mod helpers;
mod restore;
mod zigffi;

use backup::backup_gui;
use helpers::ConflictResolutionMode;
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

/// Type alias for messages exchanged during restore operations.
///
/// Used internally to communicate results of parsing a backup archive.
///
/// - On success: Contains a tuple of the root [`FolderTreeNode`] and the  original [`FolderTreeNode`] pointing to the `.tar` file.
/// - On failure: Contains an error string describing why parsing failed
type RestoreMsg = Result<(FolderTreeNode, PathBuf), String>; // Result type for restore operations

/// Result of a background file dialog.
type FileDialogMsg = Vec<PathBuf>;

/// A template representing a reusable set of file and folder paths.
///
/// Templates are serialized as JSON and can be saved/loaded by the user
/// to quickly restore backup selections.
///
/// # Fields
/// - `paths`: The list of filesystem paths that user selected to be part of a backup.
#[derive(Serialize, Deserialize)]
struct BackupTemplate {
    paths: Vec<PathBuf>,
}

/// A node in the restore/backup folder tree.
///
/// Each node may represent a file or folder.
/// Used to build a checkbox tree UI for selecting what to back up or restore.
///
/// # Fields
/// - `children`: A mapping of child names (file or folder) to their nodes.
/// - `checked`: Whether this node is currently selected in the UI.
/// - `is_file`: True if this node represents a file, false if a directory.
#[derive(Default)]
struct FolderTreeNode {
    children: HashMap<String, FolderTreeNode>,
    checked: bool,
    is_file: bool,
}

/// Builds a hierarchical tree structure from a list of file system paths.
///
/// This function constructs a [`FolderTreeNode`] tree where each node
/// represents a directory or file. It is used to display the contents
/// of a backup archive in a checkbox tree, so users can select which
/// files to restore.
///
/// # Arguments
/// - `paths` – A slice of paths (as strings) that should be added to the tree.
///
/// # Returns
/// - A [`FolderTreeNode`] representing the root of the constructed tree.
///
/// # Example
/// ```
/// let paths = vec![
///     "Documents/report.docx".to_string(),
///     "Pictures/vacation/photo1.jpg".to_string(),
/// ];
/// let tree = build_tree_from_paths(&paths);
/// assert!(tree.children.contains_key("Documents"));
/// ```
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

/// Entry point
///
/// Initializes environment variables, loads the application icon,
/// configures [`eframe::NativeOptions`], and launches the GUI.
///
/// Returns an [`eframe::Error`] if the GUI fails to start.
fn main() -> Result<(), eframe::Error> {
    println!("[DEBUG] main: Starting application");

    dotenv::dotenv().ok(); // Load environment variables from .env if available
    println!("[DEBUG] .env loaded (if present)");

    let icon = load_icon_image(); // Load application image
    println!("[DEBUG] Icon loaded");

    let options = eframe::NativeOptions {
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

/// Tabs available in the Konserve user interface.
///
/// Used for switching between different screens of the app.
#[derive(PartialEq)]
enum MainTab {
    /// Main screen for selecting files/folders and performing backup/restore.
    Home,
    /// Settings screen for configuring preferences such as compression
    /// or conflict resolution.
    Settings,
}

/// Main application state
///
/// Holds user settings, selected backup paths, progress indicators,
/// and the active tab. Implements [`eframe::App`] to render the GUI.
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
    // async file dialog handling for linux being fuck and freezing.
    file_dialog_rx: Option<mpsc::Receiver<FileDialogMsg>>,
    file_dialog_opening: bool,
    tab: MainTab,
    compression_enabled: bool,
    default_backup_location: Option<PathBuf>,
    conflict_resolution_enabled: bool,
    conflict_resolution_mode: ConflictResolutionMode,
    verbose_logging: bool,
    automatic_updates: bool,
    file_size_summary: bool,
    config: helpers::KonserveConfig,
}

/// Default initialization for [`GUIApp`].
///
/// Loads user configuration from [`helpers::KonserveConfig`],
/// applies it to the struct fields, and sets sensible defaults
/// for everything else (like "Waiting..." as the initial status).
impl Default for GUIApp {
    fn default() -> Self {
        let config = helpers::KonserveConfig::load();
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
            file_dialog_rx: None,
            file_dialog_opening: false,
            tab: MainTab::Home,
            compression_enabled: config.compression_enabled,
            default_backup_location: config.default_backup_location.clone(),
            conflict_resolution_enabled: config.conflict_resolution_enabled,
            conflict_resolution_mode: config.conflict_resolution_mode,
            verbose_logging: config.verbose_logging,
            automatic_updates: config.automatic_updates,
            file_size_summary: false,
            config,
        }
    }
}

/// Implements the main event loop and UI rendering
///
/// - **Home tab**: Add folders/files, load or save templates, create backups, and restore from existing archives.
/// - **Settings tab**: Configure various settings and preferences.
/// - Handles template editing and restore selection views as modal sub-screens.
impl eframe::App for GUIApp {
    /// Main application update loop.
    ///
    /// Called every frame by `eframe`.
    ///
    /// Manages background backup/restore threads.
    ///
    /// # Parameters
    /// - `ctx`: egui context used to render the UI.
    /// - `_frame`: Frame handle (unused here).
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
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

                egui::ScrollArea::vertical()
                    .max_height(285.0)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut to_remove = None;

                        for (i, path) in self.template_paths.iter_mut().enumerate() {
                            let mut path_str = path.display().to_string();

                            ui.horizontal(|ui| {
                                // Editable path text field
                                ui.add_sized(
                                    [240.0, 20.0],
                                    egui::TextEdit::singleline(&mut path_str),
                                );

                                if path_str != path.display().to_string() {
                                    *path = PathBuf::from(path_str.clone());
                                }

                                // Excistance indicator
                                if path.exists() {
                                    ui.label("✅").on_hover_text("This path exists");
                                } else {
                                    ui.label("❌").on_hover_text("This path does not exist");
                                }

                                // Browse for folder
                                if ui.button("Browse").clicked() {
                                    if let Some(p) = FileDialog::new().pick_folder() {
                                        *path = p;
                                    }
                                }

                                // Remove path
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
                MainTab::Home => {
                    // Handle async result from restore preview thread
                    if let Some(finished_msg) =
                        self.restore_rx.as_ref().and_then(|rx| rx.try_recv().ok())
                    {
                        match finished_msg {
                            Ok((mut tree, zip)) => {
                                // Recursively check all nodes in the tree
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

                    if let Some(rx) = self.file_dialog_rx.as_ref() {
                        use std::sync::mpsc::TryRecvError;

                        match rx.try_recv() {
                            Ok(mut paths) => {
                                self.selected_folders.append(&mut paths);
                                self.selected_folders.sort();
                                self.selected_folders.dedup();
                                self.file_dialog_rx = None;
                                self.file_dialog_opening = false;
                            }
                            Err(TryRecvError::Disconnected) => {
                                self.file_dialog_rx = None;
                                self.file_dialog_opening = false;
                            }
                            Err(TryRecvError::Empty) => {
                                // waiting...
                            }
                        }
                    }

                    ui.heading("Konserve");
                    ui.separator();

                    // Folder and File Pickers
                    ui.horizontal(|ui| {
                        if ui.button("Add Folders").clicked() {
                            #[cfg(target_os = "macos")]
                            {
                                // macOS wants dialogs on the main thread
                                if let Some(folders) = FileDialog::new().pick_folders() {
                                    self.selected_folders.extend(folders);
                                    self.selected_folders.sort();
                                    self.selected_folders.dedup();
                                }
                            }

                            #[cfg(not(target_os = "macos"))]
                            {
                                // Linux / Windows: run dialog in a background thread
                                if self.file_dialog_rx.is_none() {
                                    self.file_dialog_opening = true;

                                    let (tx, rx) = mpsc::channel::<FileDialogMsg>();
                                    self.file_dialog_rx = Some(rx);

                                    std::thread::spawn(move || {
                                        let folders =
                                            FileDialog::new().pick_folders().unwrap_or_default();
                                        let _ = tx.send(folders);
                                    });
                                }
                            }
                        }

                        if ui.button("Add Files").clicked() {
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(files) = FileDialog::new().pick_files() {
                                    self.selected_folders.extend(files);
                                    self.selected_folders.sort();
                                    self.selected_folders.dedup();
                                }
                            }

                            #[cfg(not(target_os = "macos"))]
                            {
                                if self.file_dialog_rx.is_none() {
                                    self.file_dialog_opening = true;

                                    let (tx, rx) = mpsc::channel::<FileDialogMsg>();
                                    self.file_dialog_rx = Some(rx);

                                    std::thread::spawn(move || {
                                        let files =
                                            FileDialog::new().pick_files().unwrap_or_default();
                                        let _ = tx.send(files);
                                    });
                                }
                            }
                        }
                    });

                    if self.file_dialog_opening {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label("Waiting for file dialog…");
                        });
                        ctx.request_repaint_after(std::time::Duration::from_millis(50));
                    }

                    // Show selected paths
                    if !self.selected_folders.is_empty() {
                        ui.add_space(4.0);

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

                    // Template and Action Buttons
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            let btn_size = egui::vec2(95.0, 17.0);
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
                        ui.vertical(|ui| {
                            let btn_size = egui::vec2(100.0, 17.0);
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

                                    if self.compression_enabled {
                                        *status.lock().unwrap() =
                                            "Packing into .tar and compressing (gzip)...".into();
                                    } else {
                                        *status.lock().unwrap() = "Packing into .tar".into();
                                    }

                                    let progress = Progress::default();
                                    self.backup_progress = Some(progress.clone());

                                    let compression_enabled = self.compression_enabled;

                                    let out_dir = FileDialog::new()
                                        .set_title("Choose backup destination")
                                        .pick_folder();

                                    // Use a Builder to give the compression thread a bigger stack
                                        std::thread::Builder::new()
                                        .name("konserve-backup".into())
                                        .stack_size(8 * 1024 * 1024) // 8 MiB
                                    .spawn(move || {
                                            if let Some(out_dir) = out_dir {
                                                match backup_gui(&folders, &out_dir, &progress) {
                                                    Ok(path) => {
                                                        if compression_enabled {
                                use std::ffi::CString;
                                let targz_path = path.with_extension("tar.gz");
                                let c_in  = CString::new(path.to_string_lossy().as_bytes()).unwrap();
                                let c_out = CString::new(targz_path.to_string_lossy().as_bytes()).unwrap();

                                unsafe {
                                    let rc = zigffi::konserve_gzip_tar(c_in.as_ptr(), c_out.as_ptr());
                                    if rc == 0 {
                                        let _ = std::fs::remove_file(&path);
                                        *status.lock().unwrap() = format!("✅ Backup created:\n{}", targz_path.display());
                                    } else {
                                        *status.lock().unwrap() = format!("❌ Gzip step failed (code {rc})");
                                    }
                                }
                            } else {
                                *status.lock().unwrap() = format!("✅ Backup created:\n{}", path.display());
                            }
                        }
                        Err(e) => {
                            *status.lock().unwrap() =
                                format!("❌ Backup failed: {e}");
                        }
                    }
                } else {
                    *status.lock().unwrap() = "❌ Cancelled.".into();
                }
            })
            .expect("failed to spawn backup thread");
    });
                            ui.add_sized(btn_size, egui::Button::new("Restore Backup"))
                                .clicked()
                                .then(|| {
                                    let status = self.status.clone();
                                    if let Some(zip_file) = FileDialog::new()
                                        .add_filter("Tar archives", &["tar", "tar.gz"])
                                        .pick_file()
                                    {
                                        self.restore_opening = true;
                                        *status.lock().unwrap() = "Opening archive…".into();

                                        // Create a progress channel
                                        // This will be used to send the result of the restore operation
                                        let (tx, rx) = mpsc::channel::<RestoreMsg>();
                                        self.restore_rx = Some(rx);

                                        thread::spawn(move || {
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

                MainTab::Settings => {
                    ui.heading("Settings");
                    ui.separator();

                    let btn_size = egui::vec2(95.0, 17.0);
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

                    ui.separator();

                    ui.checkbox(&mut self.compression_enabled, "Enable Compression (WIP)");

                    let mut loc_str = self
                        .default_backup_location
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();

                    ui.checkbox(
                        &mut self.conflict_resolution_enabled,
                        "Enable Conflict Resolution Mode (WIP)",
                    );

                    if self.conflict_resolution_enabled {
                        egui::ComboBox::from_label("Conflict resolution mode (WIP)")
                            .selected_text(match self.conflict_resolution_mode {
                                ConflictResolutionMode::Prompt => "Prompt",
                                ConflictResolutionMode::Overwrite => "Overwrite",
                                ConflictResolutionMode::Skip => "Skip",
                                ConflictResolutionMode::Rename => "Rename",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.conflict_resolution_mode,
                                    ConflictResolutionMode::Prompt,
                                    "Prompt",
                                );
                                ui.selectable_value(
                                    &mut self.conflict_resolution_mode,
                                    ConflictResolutionMode::Overwrite,
                                    "Overwrite",
                                );
                                ui.selectable_value(
                                    &mut self.conflict_resolution_mode,
                                    ConflictResolutionMode::Skip,
                                    "Skip",
                                );
                                ui.selectable_value(
                                    &mut self.conflict_resolution_mode,
                                    ConflictResolutionMode::Rename,
                                    "Rename",
                                );
                            });
                    }

                    ui.checkbox(&mut self.verbose_logging, "Enable Verbose Logging (WIP)");

                    ui.checkbox(
                        &mut self.automatic_updates,
                        "Enable Updates on Startup (WIP)",
                    );

                    ui.checkbox(
                        &mut self.file_size_summary,
                        "Enable File Size Summary (WIP)",
                    );

                    ui.separator();

                    ui.label("Default backup location: (WIP)");
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
                    //
                    // --- Save/Load Config ---
                    // serialization/deserialization

                    // Apply changes to default backup location
                    let should_update = match &self.default_backup_location {
                        Some(p) => loc_str != p.display().to_string(),
                        None => !loc_str.is_empty(),
                    };
                    if should_update {
                        if !loc_str.is_empty() {
                            self.default_backup_location = Some(std::path::PathBuf::from(&loc_str));
                            // TODO: Call a helper here to persist the setting, e.g.:
                            // helpers::save_settings(self);
                        } else {
                            self.default_backup_location = None;
                            // TODO: Call a helper here to clear the saved location if needed.
                        }
                    }

                    ui.separator();

                    if ui.button("Save").clicked() {
                        self.config.verbose_logging = self.verbose_logging;
                        self.config.compression_enabled = self.compression_enabled;
                        self.config.conflict_resolution_enabled = self.conflict_resolution_enabled;
                        self.config.conflict_resolution_mode = self.conflict_resolution_mode;
                        self.config.default_backup_location = self.default_backup_location.clone();
                        self.config.automatic_updates = self.automatic_updates;
                        self.config.file_size_summary = self.file_size_summary;

                        self.config.save();
                        *self.status.lock().unwrap() = "Settings saved".into();
                        ctx.request_repaint();
                    }

                    ui.separator();

                    ui.label(format!("Status: {}", self.status.lock().unwrap()));
                }
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}
