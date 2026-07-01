//! konserve, backs up your stuff and restores it later
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod backup;
mod helpers;
mod restore;

use backup::backup_gui;
use helpers::BackupNameMode;
use helpers::ConflictResolutionMode;
use helpers::Progress;
use helpers::build_human_tree;
use helpers::collect_paths;
use helpers::exe_dir;
use helpers::fix_skip;
use helpers::init_crash_log;
use helpers::load_icon_image;
use helpers::parse_fingerprint;
use helpers::render_tree;
use helpers::set_status;
use helpers::verbose_log_path;
use restore::{ConflictAnswer, restore_backup};

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
};

use chrono::Local;
use eframe::egui;
use rfd::FileDialog;
use serde::{Deserialize, Serialize};

/// app that might be locking files during backup
struct KnownApp {
    name: &'static str,
    process: &'static str,
}

const KNOWN_APPS: &[KnownApp] = &[
    KnownApp {
        name: "Discord / Vesktop",
        process: "vesktop.exe",
    },
    KnownApp {
        name: "Discord",
        process: "Discord.exe",
    },
    KnownApp {
        name: "Steam",
        process: "steam.exe",
    },
    KnownApp {
        name: "OBS Studio",
        process: "obs64.exe",
    },
    KnownApp {
        name: "Zen Browser",
        process: "zen.exe",
    },
    KnownApp {
        name: "Spotify",
        process: "Spotify.exe",
    },
    KnownApp {
        name: "ShareX",
        process: "ShareX.exe",
    },
];

struct ClosedApp {
    known_index: usize,
    /// exe path to relaunch after backup, windows only
    exe_path: Option<PathBuf>,
}

/// backup job waiting on the app-conflict prompt
struct PendingBackup {
    folders: Vec<PathBuf>,
    out_dir: PathBuf,
    filename: String,
    /// apps detected running: index into KNOWN_APPS + captured exe path
    detected: Vec<(usize, Option<PathBuf>)>,
}

/// restore preview result: tree + archive path on success, error string on fail
type RestoreMsg = Result<(FolderTreeNode, PathBuf), String>;

/// paths back from a background file dialog
type FileDialogMsg = Vec<PathBuf>;

/// result from the background app-detection thread
type DetectResult = (Vec<(usize, Option<PathBuf>)>, Vec<PathBuf>, PathBuf, String);

/// saved paths you can reload for later backups
#[derive(Serialize, Deserialize)]
struct BackupTemplate {
    paths: Vec<PathBuf>,
}

/// one node in the restore tree, either a file or a folder with kids
#[derive(Default)]
struct FolderTreeNode {
    children: HashMap<String, FolderTreeNode>,
    checked: bool,
    is_file: bool,
}

/// entry point, sets up env vars + icon + eframe and launches the gui
fn main() -> Result<(), eframe::Error> {
    dotenv::dotenv().ok();

    init_crash_log();

    // catch panics and dump them to the crash log before we die
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        helpers::write_crash_log(&format!("PANIC: {msg}"));
        eprintln!("PANIC: {msg}");
    }));

    let icon = load_icon_image();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 600.0])
            .with_resizable(false)
            .with_icon(icon),
        ..Default::default()
    };

    let result = eframe::run_native(
        "Konserve",
        options,
        Box::new(|_cc| Ok(Box::new(GUIApp::default()))),
    );

    if let Err(ref e) = result {
        helpers::write_crash_log(&format!("eframe error: {e}"));
    }

    result
}

#[derive(PartialEq)]
enum MainTab {
    Home,
    Settings,
}

/// all the app state: settings, selected paths, progress, active tab
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
    // async filedialog handling for linux being fuck and freezing.
    file_dialog_rx: Option<mpsc::Receiver<FileDialogMsg>>,
    file_dialog_opening: bool,
    tab: MainTab,
    default_backup_location: Option<PathBuf>,
    conflict_resolution_enabled: bool,
    conflict_resolution_mode: ConflictResolutionMode,
    verbose_logging: bool,
    automatic_updates: bool,
    file_size_summary: bool,
    save_to_exe_dir: bool,
    save_template_exe_dir: bool,
    load_templates_from_exe_dir: bool,
    backup_name_mode: BackupNameMode,
    // scratch buffer for the name input in settings
    backup_name_input: String,
    overwrite_confirm: Option<PathBuf>,
    conflict_rx: Option<mpsc::Receiver<PathBuf>>,
    conflict_answer_tx: Option<mpsc::Sender<ConflictAnswer>>,
    conflict_file: Option<PathBuf>,
    pending_backup: Option<PendingBackup>,
    detecting_apps: bool,
    detect_rx: Option<mpsc::Receiver<DetectResult>>,
    closed_apps: Vec<ClosedApp>,
    relaunch_prompt: bool,
    relaunch_rx: Option<mpsc::Receiver<Vec<ClosedApp>>>,
    config: helpers::KonserveConfig,
    drop_zone_rect: Option<egui::Rect>,
}

impl Default for GUIApp {
    fn default() -> Self {
        let config = helpers::KonserveConfig::load();
        let app = Self {
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
            default_backup_location: config.default_backup_location.clone(),
            conflict_resolution_enabled: config.conflict_resolution_enabled,
            conflict_resolution_mode: config.conflict_resolution_mode,
            verbose_logging: config.verbose_logging,
            automatic_updates: config.automatic_updates,
            file_size_summary: false,
            save_to_exe_dir: config.save_to_exe_dir,
            save_template_exe_dir: config.save_template_exe_dir,
            load_templates_from_exe_dir: config.load_templates_from_exe_dir,
            backup_name_input: match &config.backup_name_mode {
                BackupNameMode::Timestamp(s) | BackupNameMode::Fixed(s) => s.clone(),
            },
            backup_name_mode: config.backup_name_mode.clone(),
            overwrite_confirm: None,
            conflict_rx: None,
            conflict_answer_tx: None,
            conflict_file: None,
            pending_backup: None,
            detecting_apps: false,
            detect_rx: None,
            closed_apps: Vec::new(),
            relaunch_prompt: false,
            relaunch_rx: None,
            config,
            drop_zone_rect: None,
        };
        if app.verbose_logging {
            helpers::init_verbose_log();
        }
        app
    }
}

impl GUIApp {
    /// spawns a thread to check for conflicting apps then kicks off the backup
    fn spawn_detect_and_backup(
        &mut self,
        folders: Vec<PathBuf>,
        out_dir: PathBuf,
        filename: String,
    ) {
        let (tx, rx) = mpsc::channel();
        self.detect_rx = Some(rx);
        self.detecting_apps = true;

        let verbose = self.verbose_logging;
        thread::spawn(move || {
            // ask restart manager what's holding locks on files in the selected folders,
            // ignore anything not relevant
            let locked_names = helpers::processes_locking_paths(&folders, verbose);

            let process_names: Vec<&'static str> = KNOWN_APPS.iter().map(|a| a.process).collect();

            // only keep apps that are both running and actually locking something we're backing up
            let detected = helpers::detect_known_processes(&process_names)
                .into_iter()
                .filter(|(i, _)| {
                    let exe_stem = KNOWN_APPS[*i]
                        .process
                        .trim_end_matches(".exe")
                        .to_lowercase();
                    locked_names.iter().any(|locked| {
                        locked.contains(&exe_stem) || exe_stem.contains(locked.as_str())
                    })
                })
                .collect::<Vec<_>>();

            let _ = tx.send((detected, folders, out_dir, filename));
        });
    }

    /// kills apps, waits for them to exit, then starts the backup, all on a background thread
    fn start_backup_after_kill(
        &mut self,
        folders: Vec<PathBuf>,
        out_dir: PathBuf,
        filename: String,
        apps: Vec<ClosedApp>,
    ) {
        let status = self.status.clone();
        let progress = Progress::default();
        self.backup_progress = Some(progress.clone());
        let verbose = self.verbose_logging;

        set_status(&status, "Closing apps…");

        let (done_tx, done_rx) = mpsc::channel::<Vec<ClosedApp>>();
        self.relaunch_rx = Some(done_rx);

        std::thread::Builder::new()
            .name("konserve-backup".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut actually_closed: Vec<ClosedApp> = Vec::new();
                for app in apps {
                    let proc = KNOWN_APPS[app.known_index].process;
                    if helpers::kill_process(proc) {
                        actually_closed.push(app);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(800));

                set_status(&status, "Packing into .tar");
                match backup_gui(&folders, &out_dir, &filename, &progress, verbose, false) {
                    Ok(path) => {
                        set_status(&status, format!("✅ Backup created:\n{}", path.display()));
                    }
                    Err(e) => {
                        elog!("ERROR: backup failed: {e}");
                        set_status(&status, format!("❌ Backup failed: {e}"));
                    }
                }

                let _ = done_tx.send(actually_closed);
            })
            .expect("failed to spawn backup thread");
    }

    /// spawns the backup thread, called once the app-conflict prompt is resolved
    fn start_backup(
        &mut self,
        folders: Vec<PathBuf>,
        out_dir: PathBuf,
        filename: String,
        skip_locked: bool,
    ) {
        let status = self.status.clone();
        let progress = Progress::default();
        self.backup_progress = Some(progress.clone());
        let verbose = self.verbose_logging;

        set_status(&status, "Packing into .tar");

        std::thread::Builder::new()
            .name("konserve-backup".into())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                match backup_gui(
                    &folders,
                    &out_dir,
                    &filename,
                    &progress,
                    verbose,
                    skip_locked,
                ) {
                    Ok(path) => {
                        set_status(&status, format!("✅ Backup created:\n{}", path.display()));
                    }
                    Err(e) => {
                        elog!("ERROR: backup failed: {e}");
                        set_status(&status, format!("❌ Backup failed: {e}"));
                    }
                }
            })
            .expect("failed to spawn backup thread");
    }
}

impl eframe::App for GUIApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                for (label, tab) in [("Home", MainTab::Home), ("Settings", MainTab::Settings)] {
                    let active = self.tab == tab;
                    let text = if active {
                        egui::RichText::new(label).strong()
                    } else {
                        egui::RichText::new(label)
                    };
                    if ui.selectable_label(active, text).clicked() {
                        self.tab = tab;
                        *self.status.lock().unwrap() = String::new();
                    }
                }
            });
            ui.add_space(2.0);

            // overwrite confirm for fixed backup names
            if let Some(ref dest) = self.overwrite_confirm.clone() {
                ui.separator();
                ui.colored_label(egui::Color32::YELLOW, format!("⚠ '{}' already exists. Overwrite?", dest.file_name().unwrap_or_default().to_string_lossy()));
                ui.horizontal(|ui| {
                    if ui.button("Yes, overwrite").clicked() {
                        let dest = dest.clone();
                        let folders = self.selected_folders.clone();
                        let status = self.status.clone();
                        let progress = Progress::default();
                        self.backup_progress = Some(progress.clone());
                        let verbose = self.verbose_logging;
                        let Some(out_dir) = dest.parent().map(|p| p.to_path_buf()) else {
                elog!("ERROR: overwrite confirm: dest has no parent: {}", dest.display());
                set_status(&self.status, "❌ Internal error: invalid path.");
                self.overwrite_confirm = None;
                return;
            };
            let Some(filename) = dest.file_name().map(|f| f.to_string_lossy().into_owned()) else {
                elog!("ERROR: overwrite confirm: dest has no filename: {}", dest.display());
                set_status(&self.status, "❌ Internal error: invalid path.");
                self.overwrite_confirm = None;
                return;
            };
                        self.overwrite_confirm = None;
                        set_status(&status, "Packing into .tar");
                        std::thread::Builder::new()
                            .name("konserve-backup".into())
                            .stack_size(8 * 1024 * 1024)
                            .spawn(move || {
                                match backup_gui(&folders, &out_dir, &filename, &progress, verbose, false) {
                                    Ok(path) => { set_status(&status, format!("✅ Backup created:\n{}", path.display())); }
                                    Err(e) => {
                                        elog!("ERROR: backup failed: {e}");
                                        set_status(&status, format!("❌ Backup failed: {e}"));
                                    }
                                }
                            })
                            .expect("failed to spawn backup thread");
                    }
                    if ui.button("Cancel").clicked() {
                        self.overwrite_confirm = None;
                        *self.status.lock().unwrap() = "❌ Cancelled.".into();
                    }
                });
                ui.separator();
            }

            // app-conflict prompt
            if let Some(ref pending) = self.pending_backup {
                ui.separator();
                ui.colored_label(egui::Color32::YELLOW, "⚠ The following apps may be locking files:");
                for &(i, _) in &pending.detected {
                    ui.label(format!("  • {}", KNOWN_APPS[i].name));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Close apps & backup").clicked() {
                        let pending = self.pending_backup.take().unwrap();
                        let apps: Vec<ClosedApp> = pending.detected.iter()
                            .map(|&(i, ref path)| ClosedApp {
                                known_index: i,
                                exe_path: path.clone(),
                            })
                            .collect();
                        self.start_backup_after_kill(pending.folders, pending.out_dir, pending.filename, apps);
                    }
                    if ui.button("Skip locked files").clicked() {
                        let pending = self.pending_backup.take().unwrap();
                        self.start_backup(pending.folders, pending.out_dir, pending.filename, true);
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_backup = None;
                        *self.status.lock().unwrap() = "❌ Cancelled.".into();
                    }
                });
                ui.separator();
            }

            if self.relaunch_prompt {
                ui.separator();
                ui.colored_label(egui::Color32::LIGHT_BLUE, "Backup finished. Relaunch apps?");
                for app in &self.closed_apps {
                    let note = if app.exe_path.is_some() { "" } else { "Can't determine installation path" };
                    ui.label(format!("  • {}{}", KNOWN_APPS[app.known_index].name, note));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("yes").clicked() {
    let mut failed = Vec::new();
    for app in &self.closed_apps {
        if let Some(path) = &app.exe_path
             && let Err(e) = std::process::Command::new(path).spawn() {
                 elog!("ERROR: failed to relaunch {}: {e}", path.display());
                 failed.push(KNOWN_APPS[app.known_index].name);
             }
    }
    if failed.is_empty() {
        set_status(&self.status, "");
    } else {
        set_status(&self.status, format!("⚠ Couldn't relaunch: {}", failed.join(", ")));
    }
    self.closed_apps.clear();
    self.relaunch_prompt = false;
}
                    if ui.button("no").clicked() {
                        self.closed_apps.clear();
                        self.relaunch_prompt = false;
                    }
                });
                ui.separator();
            }

            // poll the restore conflict channel, show the per-file prompt
            if self.conflict_file.is_none()
                && let Some(path) = self.conflict_rx.as_ref().and_then(|rx| rx.try_recv().ok())
            {
                self.conflict_file = Some(path);
            }
            if let Some(ref path) = self.conflict_file.clone() {
                ui.separator();
                ui.colored_label(egui::Color32::YELLOW, "⚠ File already exists at restore destination:");
                ui.label(path.display().to_string());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Overwrite").clicked() {
                        if let Some(tx) = &self.conflict_answer_tx {
                            let _ = tx.send(ConflictAnswer::Overwrite);
                        }
                        self.conflict_file = None;
                    }
                    if ui.button("Skip").clicked() {
                        if let Some(tx) = &self.conflict_answer_tx {
                            let _ = tx.send(ConflictAnswer::Skip);
                        }
                        self.conflict_file = None;
                    }
                    if ui.button("Rename").clicked() {
                        if let Some(tx) = &self.conflict_answer_tx {
                            let _ = tx.send(ConflictAnswer::Rename);
                        }
                        self.conflict_file = None;
                    }
                });
                ui.separator();
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
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

                                if ui.button("Browse").clicked()
                                    && let Some(p) = FileDialog::new().set_directory(exe_dir()).pick_folder()
                                {
                                    *path = p;
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
                    let save_path = if self.save_template_exe_dir {
                    std::env::current_exe().ok()
                        .and_then(|p| p.parent().map(|d| d.join("template.json")))
                } else {
                    None
                };

                if ui.button("Save Template").clicked() {
                    let path = if self.save_template_exe_dir {
                        save_path.clone()
                    } else {
                        FileDialog::new().set_directory(exe_dir()).add_filter("JSON", &["json"]).save_file()
                    };

                    if let Some(path) = path {
                        let tpl = BackupTemplate {
                            paths: self.template_paths.clone(),
                        };
                        match serde_json::to_string_pretty(&tpl) {
                            Ok(json) => match fs::write(&path, json) {
                                Ok(()) => {
                                    *self.status.lock().unwrap() = "✅ Template saved".into();
                                    self.template_editor = false;
                                }
                                Err(e) => {
                                    elog!("ERROR: failed to write template {}: {e}", path.display());
                                    *self.status.lock().unwrap() = "❌ Couldn't write file.".into();
                                }
                            },
                            Err(e) => {
                                elog!("ERROR: failed to serialize template: {e}");
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
                        render_tree(ui, &mut current_path, &mut self.restore_tree, self.verbose_logging)
                    });

                ui.separator();

                if ui.button("Restore selected").clicked()
                    && let Some(zip_path) = &self.restore_zip_path.clone()
                {
                    let selected = collect_paths(&self.restore_tree, self.verbose_logging);
                    let zip_path = zip_path.clone();
                    let status = self.status.clone();

                    let progress = Progress::default();
                    self.restore_progress = Some(progress.clone());
                    self.restore_opening = false;
                    let verbose = self.verbose_logging;
                    let mode = if self.conflict_resolution_enabled {
                        self.conflict_resolution_mode
                    } else {
                        ConflictResolutionMode::Overwrite
                    };

                    let conflict_ch = if mode == ConflictResolutionMode::Prompt {
                        let (ctx, crx) = mpsc::channel::<PathBuf>();
                        let (atx, arx) = mpsc::channel::<ConflictAnswer>();
                        self.conflict_rx = Some(crx);
                        self.conflict_answer_tx = Some(atx);
                        Some((ctx, arx))
                    } else {
                        self.conflict_rx = None;
                        self.conflict_answer_tx = None;
                        None
                    };

                    thread::spawn(move || {
                        if let Err(e) =
                            restore_backup(&zip_path, Some(selected), status.clone(), &progress, verbose, mode, conflict_ch)
                        {
                            elog!("ERROR: restore failed: {e}");
                            set_status(&status, format!("❌ Restore failed: {e}"));
                        }
                    });

                    self.restore_editor = false;
                }

                if ui.button("Cancel").clicked() {
                    self.restore_editor = false;
                    self.restore_opening = false;
                    self.restore_zip_path = None;
                    self.restore_tree = FolderTreeNode::default();
                    *self.status.lock().unwrap() = String::new();
                }

                return;
            }

            match self.tab {
                MainTab::Home => {
                    // poll the detect-apps thread
                    if let Some((detected, folders, out_dir, filename)) =
                        self.detect_rx.as_ref().and_then(|rx| rx.try_recv().ok())
                    {
                        self.detect_rx = None;
                        self.detecting_apps = false;
                        if detected.is_empty() {
                            self.start_backup(folders, out_dir, filename, false);
                        } else {
                            *self.status.lock().unwrap() = "Waiting…".into();
                            self.pending_backup = Some(PendingBackup { folders, out_dir, filename, detected });
                        }
                    }

                    if let Some(rx) = self.relaunch_rx.as_ref() {
                        use std::sync::mpsc::TryRecvError;
                        match rx.try_recv() {
                            Ok(apps) => {
                                self.relaunch_rx = None;
                                self.closed_apps = apps;
                                self.relaunch_prompt = !self.closed_apps.is_empty();
                            }
                            Err(TryRecvError::Disconnected) => {
                                self.relaunch_rx = None;
                            }
                            Err(TryRecvError::Empty) => {
                                // waiting...
                            }
                        }
                    }

                    // handle the restore preview thread's result
                    if let Some(finished_msg) =
                        self.restore_rx.as_ref().and_then(|rx| rx.try_recv().ok())
                    {
                        match finished_msg {
                            Ok((mut tree, zip)) => {
                                // checks every node in the tree
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
                                self.restore_opening = false;
                                *self.status.lock().unwrap() = String::new();
                            }
                            Err(e) => {
                                elog!("ERROR: failed to open archive: {e}");
                                *self.status.lock().unwrap() = format!("❌ Failed to open archive: {e}");
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

                    ui.horizontal(|ui| {
                        ui.heading("Konserve");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(format!("v{}", env!("CARGO_PKG_VERSION")));
                        });
                    });
                    ui.separator();
                    ui.add_space(2.0);

                    // folder + file pickers
                    egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                        if ui.button("Add Folders").clicked() {
                            #[cfg(target_os = "macos")]
                            {
                                // macos wants dialogs on the main thread
                                if let Some(folders) = FileDialog::new().set_directory(exe_dir()).pick_folders() {
                                    self.selected_folders.extend(folders);
                                    self.selected_folders.sort();
                                    self.selected_folders.dedup();
                                }
                            }

                            #[cfg(not(target_os = "macos"))]
                            {
                                // linux/windows: run the dialog on a background thread
                                if self.file_dialog_rx.is_none() {
                                    self.file_dialog_opening = true;

                                    let (tx, rx) = mpsc::channel::<FileDialogMsg>();
                                    self.file_dialog_rx = Some(rx);

                                    std::thread::spawn(move || {
                                        let folders =
                                            FileDialog::new().set_directory(exe_dir()).pick_folders().unwrap_or_default();
                                        let _ = tx.send(folders);
                                    });
                                }
                            }
                        }

                        if ui.button("Add Files").clicked() {
                            #[cfg(target_os = "macos")]
                            {
                                if let Some(files) = FileDialog::new().set_directory(exe_dir()).pick_files() {
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
                                            FileDialog::new().set_directory(exe_dir()).pick_files().unwrap_or_default();
                                        let _ = tx.send(files);
                                    });
                                }
                            }
                        }
                        });
                    }); // end picker frame
                    ui.add_space(2.0);

                    if self.detecting_apps {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label("Checking for open apps…");
                        });
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                    }

                    if self.file_dialog_opening {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label("Waiting for file dialog…");
                        });
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                    }

                    let zone_hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
                    if zone_hovering {
                        ui.ctx().request_repaint();
                    }
                    let dropped_paths: Vec<PathBuf> = ui.ctx().input(|i| {
                        i.raw.dropped_files.iter()
                            .filter_map(|f| f.path.clone())
                            .collect()
                    });
                    if !dropped_paths.is_empty() {
                        self.selected_folders.extend(dropped_paths);
                        self.selected_folders.sort();
                        self.selected_folders.dedup();
                    }
                    // selected paths card
                    let stroke = if zone_hovering {
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(80, 160, 240))
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke
                    };

                    let drop_zone = egui::Frame::new()
                        .stroke(stroke)
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(6, 4))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if self.selected_folders.is_empty() {
                                ui.vertical_centered(|ui| {
                                    ui.add_space(18.0);
                                        ui.weak("No files or folders selected.");
                                        ui.weak("Use Add Folders or Add Files above, or drag and drop here.");
                                    ui.add_space(18.0);
                                });
                            } else {
                                ui.horizontal(|ui| {
                                    ui.weak(format!("Selected ({})", self.selected_folders.len()));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui.small_button("Clear All").clicked() {
                                            self.selected_folders.clear();
                                        }
                                    });
                                });
                                ui.separator();
                                let mut to_remove = None;
                                egui::ScrollArea::vertical()
                                    .max_height(200.0)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        for (i, path) in self.selected_folders.iter().enumerate() {
                                            ui.horizontal(|ui| {
                                                ui.weak("•");
                                                if ui.selectable_label(false, path.display().to_string())
                                                    .on_hover_text("Click to remove")
                                                    .clicked()
                                                {
                                                    to_remove = Some(i);
                                                }
                                            });
                                        }
                                    });
                                if let Some(i) = to_remove {
                                    self.selected_folders.remove(i);
                                }
                            }
                        });

                    self.drop_zone_rect = Some(drop_zone.response.rect);

                    ui.add_space(2.0);

                    ui.separator();

                    // template + action buttons
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            let btn_size = egui::vec2(110.0, 24.0);
                            ui.add_sized(btn_size, egui::Button::new("Load Template"))
                                .clicked()
                                .then(|| {
                                    let path = if self.load_templates_from_exe_dir {
                                        std::env::current_exe().ok()
                                            .and_then(|p| p.parent().map(|d| d.join("template.json")))
                                    } else {
                                        FileDialog::new().set_directory(exe_dir()).add_filter("JSON", &["json"]).pick_file()
                                    };

                                    if let Some(path) = path {
                                        match fs::read_to_string(&path) {
                                            Ok(data) => match serde_json::from_str::<BackupTemplate>(&data) {
                                                Ok(template) => {
                                                    let mut valid = Vec::new();
                                                    let mut skipped = Vec::new();

                                                    let verbose = self.verbose_logging;
                                                    for p in template.paths {
                                                        match fix_skip(&p, verbose) {
                                                            Some(adjusted) => valid.push(adjusted),
                                                            None => skipped.push(p),
                                                        }
                                                    }

                                                    self.selected_folders = valid;
                                                    let msg = if skipped.is_empty() {
                                                        "✅ Template loaded".into()
                                                    } else {
                                                        // tell them how many got skipped
                                                        format!(
                                                            "✅ Loaded with {} paths skipped",
                                                            skipped.len()
                                                        )
                                                    };

                                                    *self.status.lock().unwrap() = msg;
                                                }
                                                Err(e) => {
                                                    elog!("ERROR: failed to parse template {}: {e}", path.display());
                                                    *self.status.lock().unwrap() =
                                                        "❌ Bad template format.".into();
                                                }
                                            },
                                            Err(e) => {
                                                elog!("ERROR: failed to read template {}: {e}", path.display());
                                                *self.status.lock().unwrap() =
                                                    "❌ Couldn't read template file.".into();
                                            }
                                        }
                                    }
                                });

                                ui.add_sized(btn_size, egui::Button::new("Save Template"))
                                .clicked()
                                .then(|| {
                                    let path = if self.save_template_exe_dir {
                                        std::env::current_exe().ok()
                                            .and_then(|p| p.parent().map(|d| d.join("template.json")))
                                    } else {
                                        FileDialog::new().set_directory(exe_dir()).add_filter("JSON", &["json"]).save_file()
                                    };

                                    if let Some(path) = path {
                                        let template = BackupTemplate {
                                            paths: self.selected_folders.clone(),
                                        };

                                        match serde_json::to_string_pretty(&template) {
                                            Ok(json) => match fs::write(&path, json) {
                                                Ok(()) => {
                                                    *self.status.lock().unwrap() =
                                                        "✅ Template saved.".into();
                                                }
                                                Err(e) => {
                                                    elog!("ERROR: failed to write template {}: {e}", path.display());
                                                    *self.status.lock().unwrap() =
                                                        "❌ Failed to write template.".into();
                                                }
                                            },
                                            Err(e) => {
                                                elog!("ERROR: failed to serialize template: {e}");
                                                *self.status.lock().unwrap() =
                                                    "❌ Failed to serialize template.".into();
                                            }
                                        }
                                    }
                                });
                        });
                        ui.vertical(|ui| {
                            let btn_size = egui::vec2(115.0, 24.0);
                            ui.add_sized(btn_size, egui::Button::new("Create Backup")
                                .fill(egui::Color32::from_rgb(40, 100, 180)))
                                .clicked()
                                .then(|| {
                                    let folders = self.selected_folders.clone();
                                    let status = self.status.clone();

                                    if folders.is_empty() {
                                        set_status(&status, "❌ Nothing selected.");
                                        return;
                                    }

                                    // figure out where to save it
                                    let out_dir = if self.save_to_exe_dir {
                                        std::env::current_exe().ok()
                                            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                                    } else {
                                        FileDialog::new().set_directory(exe_dir())

                                            .set_title("Choose backup destination")
                                            .pick_folder()
                                    };

                                    let Some(out_dir) = out_dir else {
                                        set_status(&status, "❌ Cancelled.");
                                        return;
                                    };

                                    // figure out the filename
                                    let filename = match &self.backup_name_mode {
                                        BackupNameMode::Timestamp(fmt) => {
                                            format!("backup_{}.tar", Local::now().format(fmt))
                                        }
                                        BackupNameMode::Fixed(name) => {
                                            format!("{name}.tar")
                                        }
                                    };

                                    // check for overwrite if it's a fixed name
                                    let dest = out_dir.join(&filename);
                                    if matches!(self.backup_name_mode, BackupNameMode::Fixed(_)) && dest.exists() {
                                        self.overwrite_confirm = Some(dest);
                                        return;
                                    }

                                    set_status(&status, "Checking for open apps…");
                                    self.spawn_detect_and_backup(folders, out_dir, filename);
    });
                            ui.add_sized(btn_size, egui::Button::new("Restore Backup"))
                                .on_hover_text("⚠ Only restore archives you created yourself. Restoring untrusted archives can overwrite files on your system.")
                                .clicked()
                                .then(|| {
                                    let status = self.status.clone();
                                    if let Some(zip_file) = FileDialog::new().set_directory(exe_dir())
                                        .add_filter("Tar archives", &["tar", "tar.gz"])
                                        .pick_file()
                                    {
                                        self.restore_opening = true;
                                        set_status(&status, "⚠ Only restore archives you created yourself — opening archive…");

                                        let (tx, rx) = mpsc::channel::<RestoreMsg>();
                                        self.restore_rx = Some(rx);
                                        let verbose = self.verbose_logging;

                                        thread::spawn(move || {
                                            let result: RestoreMsg = parse_fingerprint(&zip_file, verbose)
                                                .map(|(entries, map)| {
                                                    (
                                                        build_human_tree(entries, map, verbose),
                                                        zip_file.clone(),
                                                    )
                                                });
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
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(30));
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
                                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
                                }
                                _ => {
                                    *p_opt = None;
                                }
                            }
                        }
                    }
                    ui.add_space(2.0);
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let status_text = self.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
                            ui.label(status_text.as_str());
                        });
                }

                MainTab::Settings => {
                    ui.horizontal(|ui| {
                        ui.heading("Settings");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(format!("v{}", env!("CARGO_PKG_VERSION")));
                        });
                    });
                    ui.separator();

                    let btn_size = egui::vec2(95.0, 17.0);
                    ui.add_sized(btn_size, egui::Button::new("Edit Template"))
                        .clicked()
                        .then(|| {
                            let path = if self.load_templates_from_exe_dir {
                                std::env::current_exe().ok()
                                    .and_then(|p| p.parent().map(|d| d.join("template.json")))
                            } else {
                                FileDialog::new().set_directory(exe_dir()).add_filter("JSON", &["json"]).pick_file()
                            };

                            if let Some(path) = path {
                                match fs::read_to_string(&path) {
                                    Ok(data) => match serde_json::from_str::<BackupTemplate>(&data) {
                                        Ok(template) => {
                                            self.template_paths = template
                                                .paths
                                                .into_iter()
                                                .map(|p| fix_skip(&p, self.verbose_logging).unwrap_or(p))
                                                .collect();
                                            self.template_editor = true;
                                        }
                                        Err(e) => {
                                            elog!("ERROR: failed to parse template {}: {e}", path.display());
                                            *self.status.lock().unwrap() =
                                                "❌ Couldn't parse template.".into();
                                        }
                                    },
                                    Err(e) => {
                                        elog!("ERROR: failed to read template {}: {e}", path.display());
                                        *self.status.lock().unwrap() =
                                            "❌ Couldn't read template file.".into();
                                    }
                                }
                            }
                        });

                    ui.add_space(4.0);

                    let frame = egui::Frame::new()
                        .fill(ui.visuals().faint_bg_color)
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(8, 6));

                    let mut loc_str = self
                        .default_backup_location
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();

                    // --- general ---
                    frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new("General").weak().small());
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            let resp = ui.checkbox(&mut self.verbose_logging, "Verbose Logging");
                            if resp.changed() {
                                if self.verbose_logging { helpers::init_verbose_log(); }
                                else { helpers::close_verbose_log(); }
                            }
                            if self.verbose_logging && ui.small_button("Open Log").clicked() {
                                let path = verbose_log_path();
                                #[cfg(target_os = "windows")]
                                let _ = std::process::Command::new("explorer").arg(&path).spawn();
                                #[cfg(not(target_os = "windows"))]
                                let _ = std::process::Command::new("open").arg(&path).spawn();
                            }
                        });
                        ui.checkbox(&mut self.automatic_updates, "Check for Updates on Startup (WIP)");
                        ui.checkbox(&mut self.file_size_summary, "File Size Summary (WIP)");
                    });

                    ui.add_space(4.0);

                    // --- conflict resolution ---
                    frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new("Conflict Resolution").weak().small());
                        ui.add_space(2.0);
                        ui.checkbox(&mut self.conflict_resolution_enabled, "Enable Conflict Resolution");
                        if self.conflict_resolution_enabled {
                            egui::ComboBox::from_id_salt("conflict_mode")
                                .selected_text(match self.conflict_resolution_mode {
                                    ConflictResolutionMode::Prompt => "Prompt",
                                    ConflictResolutionMode::Overwrite => "Overwrite",
                                    ConflictResolutionMode::Skip => "Skip",
                                    ConflictResolutionMode::Rename => "Rename",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.conflict_resolution_mode, ConflictResolutionMode::Prompt, "Prompt");
                                    ui.selectable_value(&mut self.conflict_resolution_mode, ConflictResolutionMode::Overwrite, "Overwrite");
                                    ui.selectable_value(&mut self.conflict_resolution_mode, ConflictResolutionMode::Skip, "Skip");
                                    ui.selectable_value(&mut self.conflict_resolution_mode, ConflictResolutionMode::Rename, "Rename");
                                });
                        }
                    });

                    ui.add_space(4.0);

                    // --- backup location & naming ---
                    frame.show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(egui::RichText::new("Backup Location & Naming").weak().small());
                        ui.add_space(2.0);

                        ui.checkbox(&mut self.save_to_exe_dir, "Save backups to exe directory");
                        ui.checkbox(&mut self.save_template_exe_dir, "Save templates to exe directory");
                        ui.checkbox(&mut self.load_templates_from_exe_dir, "Load templates from exe directory");
                        ui.add_space(2.0);

                        ui.label("Default backup location:");
                        ui.add_sized([ui.available_width(), 20.0], egui::TextEdit::singleline(&mut loc_str));
                        ui.horizontal(|ui| {
                            if ui.small_button("Browse").clicked()
                                && let Some(folder) = rfd::FileDialog::new().set_directory(exe_dir()).pick_folder()
                            {
                                loc_str = folder.display().to_string();
                            }
                            if !loc_str.is_empty() && ui.small_button("Clear").clicked() {
                                loc_str.clear();
                            }
                            if !loc_str.is_empty() {
                                if Path::new(&loc_str).is_dir() {
                                    ui.label("✅").on_hover_text("Path exists");
                                } else {
                                    ui.label("❌").on_hover_text("Path does not exist");
                                }
                            }
                        });

                        ui.add_space(4.0);

                        const TS_PRESETS: &[(&str, &str)] = &[
                            ("%Y-%m-%d_%H-%M-%S", "YYYY-MM-DD_HH-MM-SS"),
                            ("%Y-%m-%d_%H-%M",    "YYYY-MM-DD_HH-MM"),
                            ("%Y-%m-%d",          "YYYY-MM-DD"),
                            ("%d-%m-%Y_%H-%M-%S", "DD-MM-YYYY_HH-MM-SS"),
                            ("%d-%m-%Y_%H-%M",    "DD-MM-YYYY_HH-MM"),
                            ("%d-%m-%Y",          "DD-MM-YYYY"),
                            ("%m-%d-%Y_%H-%M-%S", "MM-DD-YYYY_HH-MM-SS"),
                            ("%m-%d-%Y_%H-%M",    "MM-DD-YYYY_HH-MM"),
                            ("%m-%d-%Y",          "MM-DD-YYYY"),
                            ("%y-%m-%d_%H-%M-%S", "YY-MM-DD_HH-MM-SS"),
                            ("%y-%m-%d_%H-%M",    "YY-MM-DD_HH-MM"),
                            ("%y-%m-%d",          "YY-MM-DD"),
                            ("%d-%m-%y_%H-%M-%S", "DD-MM-YY_HH-MM-SS"),
                            ("%d-%m-%y_%H-%M",    "DD-MM-YY_HH-MM"),
                            ("%d-%m-%y",          "DD-MM-YY"),
                            ("%m-%d-%y_%H-%M-%S", "MM-DD-YY_HH-MM-SS"),
                            ("%m-%d-%y_%H-%M",    "MM-DD-YY_HH-MM"),
                            ("%m-%d-%y",          "MM-DD-YY"),
                        ];

                        ui.label("Backup filename:");
                        let is_fixed = matches!(self.backup_name_mode, BackupNameMode::Fixed(_));
                        ui.horizontal(|ui| {
                            if ui.radio(!is_fixed, "Timestamp").clicked() {
                                self.backup_name_mode = BackupNameMode::Timestamp(TS_PRESETS[0].0.to_string());
                            }
                            if ui.radio(is_fixed, "Fixed name").clicked() {
                                self.backup_name_mode = BackupNameMode::Fixed(self.backup_name_input.clone());
                            }
                        });

                        if is_fixed {
                            let resp = ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.backup_name_input).desired_width(160.0));
                                ui.weak(format!("→ {}.tar", self.backup_name_input));
                            });
                            if resp.response.changed() {
                                self.backup_name_mode = BackupNameMode::Fixed(self.backup_name_input.clone());
                            }
                        } else {
                            let current_fmt = match &self.backup_name_mode {
                                BackupNameMode::Timestamp(f) => f.clone(),
                                _ => TS_PRESETS[0].0.to_string(),
                            };
                            let selected_label = TS_PRESETS.iter()
                                .find(|(f, _)| *f == current_fmt)
                                .map(|(_, l)| *l)
                                .unwrap_or(TS_PRESETS[0].1);
                            egui::ComboBox::from_id_salt("ts_format")
                                .selected_text(selected_label)
                                .width(180.0)
                                .show_ui(ui, |ui| {
                                    for (fmt, label) in TS_PRESETS {
                                        let preview = Local::now().format(fmt).to_string();
                                        ui.selectable_value(
                                            &mut self.backup_name_mode,
                                            BackupNameMode::Timestamp(fmt.to_string()),
                                            format!("{label}  ({preview})"),
                                        );
                                    }
                                });
                            let preview = Local::now().format(&current_fmt).to_string();
                            ui.weak(format!("→ backup_{preview}.tar"));
                        }
                    });

                    // apply the default backup location change
                    let should_update = match &self.default_backup_location {
                        Some(p) => loc_str != p.display().to_string(),
                        None => !loc_str.is_empty(),
                    };
                    if should_update {
                        self.default_backup_location = if loc_str.is_empty() {
                            None
                        } else {
                            Some(std::path::PathBuf::from(&loc_str))
                        };
                    }
                    ui.add_space(4.0);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui.add(egui::Button::new("  Save  ")
                            .fill(egui::Color32::from_rgb(40, 100, 180)))
                            .clicked()
                        {
                            self.config.verbose_logging = self.verbose_logging;
                            self.config.conflict_resolution_enabled = self.conflict_resolution_enabled;
                            self.config.conflict_resolution_mode = self.conflict_resolution_mode;
                            self.config.default_backup_location = self.default_backup_location.clone();
                            self.config.automatic_updates = self.automatic_updates;
                            self.config.file_size_summary = self.file_size_summary;
                            self.config.save_to_exe_dir = self.save_to_exe_dir;
                            self.config.save_template_exe_dir = self.save_template_exe_dir;
                            self.config.load_templates_from_exe_dir = self.load_templates_from_exe_dir;
                            self.config.backup_name_mode = self.backup_name_mode.clone();
                            let msg = if self.config.save() { "✅ Settings saved" } else { "❌ Failed to save settings" };
                            *self.status.lock().unwrap() = msg.into();
                            ui.ctx().request_repaint();
                        }
                    });

                }
            }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }); // end margin frame
    }
}
