#![windows_subsystem = "windows"]

use std::collections::HashMap;
use std::fs::{ self, File };
use std::io::{ self, Read, Write };
use std::path::{ Path, PathBuf };
use std::sync::{ Arc, Mutex };
use std::thread;

use chrono::Local;
use eframe::egui;
use rfd::FileDialog;
use walkdir::WalkDir;
use zip::{ write::FileOptions, CompressionMethod, ZipArchive, ZipWriter };

use egui::viewport::IconData;

fn load_icon_image() -> Arc<IconData> {
    let image_bytes = include_bytes!("assets/icon.png");
    let image = image::load_from_memory(image_bytes).expect("Invalid image").into_rgba8();
    let (width, height) = image.dimensions();
    let pixels = image.into_raw();

    Arc::new(IconData {
        rgba: pixels,
        width,
        height,
    })
}

fn main() -> Result<(), eframe::Error> {
    let icon = load_icon_image();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder
            ::default()
            .with_inner_size([400.0, 400.0])
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
}

impl Default for GUIApp {
    fn default() -> Self {
        Self {
            status: Arc::new(Mutex::new("Waiting...".into())),
            selected_folders: Vec::new(),
        }
    }
}

impl eframe::App for GUIApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("VanManen Backup Tool");
            ui.separator();

            if ui.button("Add Folders").clicked() {
                if
                    let Some(folders) = FileDialog::new()
                        .set_title("Select folders to back up")
                        .pick_folders()
                {
                    self.selected_folders.extend(folders);
                    self.selected_folders.sort();
                    self.selected_folders.dedup();
                }
            }

            ui.add_space(4.0);

            let mut to_remove: Option<usize> = None;

            egui::ScrollArea
                ::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, folder) in self.selected_folders.iter().enumerate() {
                        let label = folder.display().to_string();
                        if ui.button(&label).clicked() {
                            to_remove = Some(i);
                        }
                    }
                });

            if let Some(index) = to_remove {
                self.selected_folders.remove(index);
            }

            ui.separator();

            if ui.button("Create Backup").clicked() {
                let status = self.status.clone();
                let folders = self.selected_folders.clone();

                if folders.is_empty() {
                    *status.lock().unwrap() = "❌ No folders selected.".into();
                    return;
                }

                thread::spawn(move || {
                    let output_dir = FileDialog::new()
                        .set_title("Select output directory for backup")
                        .pick_folder();

                    if let Some(out) = output_dir {
                        match create_temp_backup_gui(&folders, &out) {
                            Ok(path) => {
                                *status.lock().unwrap() = format!(
                                    "✅ Backup saved to:\n{}",
                                    path.display()
                                );
                            }
                            Err(e) => {
                                *status.lock().unwrap() = format!("❌ Backup failed: {}", e);
                            }
                        }
                    } else {
                        *status.lock().unwrap() = "❌ Output directory not selected.".into();
                    }
                });
            }

            if ui.button("Restore Backup").clicked() {
                let status = self.status.clone();
                thread::spawn(move || {
                    let zip_file = FileDialog::new().add_filter("zip", &["zip"]).pick_file();
                    if let Some(file) = zip_file {
                        match restore_backup_gui(&file) {
                            Ok(_) => {
                                *status.lock().unwrap() = "✅ Restore complete.".into();
                            }
                            Err(e) => {
                                *status.lock().unwrap() = format!("❌ Restore failed: {}", e);
                            }
                        }
                    } else {
                        *status.lock().unwrap() = "❌ No backup selected.".into();
                    }
                });
            }

            ui.separator();
            ui.label(format!("{}", self.status.lock().unwrap()));
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}
fn create_temp_backup_gui(folders: &[PathBuf], output_dir: &PathBuf) -> Result<PathBuf, String> {
    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let zip_name = format!("backup_{}.zip", timestamp);
    let zip_path = output_dir.join(&zip_name);

    let file = File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options: FileOptions<'_, ()> = FileOptions::default().compression_method(
        CompressionMethod::Deflated
    );

    zip.start_file("fingerprint.txt", options).unwrap();
    let mut fingerprint = String::from("pillupaa\n[Backup Info]\n");
    for (i, folder) in folders.iter().enumerate() {
        fingerprint.push_str(&format!("Folder {}: {}\n", i + 1, folder.display()));
    }
    zip.write_all(fingerprint.as_bytes()).unwrap();

    for folder in folders {
        let base_path = folder;
        for entry in WalkDir::new(base_path).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            let relative = match path.strip_prefix(base_path) {
                Ok(r) => r,
                Err(_) => {
                    continue;
                }
            };

            let zip_folder = base_path.file_name().unwrap();
            let final_path = Path::new(zip_folder).join(relative);

            if path.is_file() {
                zip.start_file(final_path.to_string_lossy(), options).unwrap();
                let mut f = File::open(path).unwrap();
                io::copy(&mut f, &mut zip).unwrap();
            } else if !relative.as_os_str().is_empty() {
                zip.add_directory(final_path.to_string_lossy(), options).unwrap();
            }
        }
    }

    zip.finish().unwrap();
    Ok(zip_path)
}

fn restore_backup_gui(zip_path: &PathBuf) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;

    let mut folder_map = HashMap::new();
    let mut valid = false;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        if file.name() == "fingerprint.txt" {
            let mut contents = String::new();
            file.read_to_string(&mut contents).unwrap();
            if contents.contains("pillupaa") {
                valid = true;
                for line in contents.lines() {
                    if let Some((_, path)) = line.split_once(": ") {
                        if let Some(name) = Path::new(path).file_name() {
                            folder_map.insert(name.to_string_lossy().to_string(), path.to_string());
                        }
                    }
                }
            }
            break;
        }
    }

    if !valid {
        return Err("Invalid backup fingerprint.".into());
    }

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let internal_path = Path::new(file.name());

        if file.name() == "fingerprint.txt" {
            continue;
        }

        let root = internal_path.components().next().unwrap().as_os_str().to_string_lossy();
        let base = match folder_map.get(&root.to_string()) {
            Some(p) => Path::new(p),
            None => {
                continue;
            }
        };

        let relative = internal_path.strip_prefix(&root.to_string()).unwrap();
        let full_path = base.join(relative);

        if file.name().ends_with('/') {
            fs::create_dir_all(&full_path).unwrap();
        } else {
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            let mut out = File::create(&full_path).unwrap();
            std::io::copy(&mut file, &mut out).unwrap();
        }
    }

    Ok(())
}
