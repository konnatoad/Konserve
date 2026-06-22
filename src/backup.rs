//! # Backup Module
//!
//! Handles creation of `.tar` backup archives
//!
//! - Accepts a list of user-selected files and folders.
//! - Packages them into a `.tar` archive (with optional compression planned).
//! - Embeds a `fingerprint.txt` file that maps UUIDs to original paths,
//!   ensuring that restores can accurately reconstruct file locations.
//! - Tracks progress using the [`Progress`] helper
//!   so the GUI can display live status updates.
//!
//! ## Notes
//! - Current format is `.tar`. `.tar.gz` support is planned but not yet active.
//! - Old `.zip` format is deprecated and left as commented legacy code.
use crate::helpers::{Progress, get_fingered};
use crate::dlog;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

use chrono::Local;
use tar::{Builder, Header};
use uuid::Uuid;
use walkdir::WalkDir;


/// Create a `.tar` backup archive of the given folders or files.
///
/// This function is used by the GUI to build a `.tar` archive
/// from user-selected folders and files.  
/// It embeds a `fingerprint.txt` metadata file inside the archive,
/// which contains:
/// - a unique identifier for the backup session
/// - a mapping of randomly generated UUIDs to original paths
///
/// The backup progress is reported via a shared [`Progress`] counter,
/// which allows the GUI to update a progress bar.
///
/// # Arguments
/// - `folders`: A list of file or folder paths to include in the backup.
/// - `output_dir`: The directory where the `.tar` archive should be created.
/// - `progress`: A [`Progress`] instance used to report completion percentage.
///
/// # Returns
/// - `Ok(PathBuf)` containing the path to the created `.tar` file on success.
/// - `Err(String)` with an error message if the backup failed.
///
/// # Example
/// ```rust,no_run
/// use std::path::PathBuf;
/// use konserve::helpers::Progress;
/// use konserve::backup::backup_gui;
///
/// let folders = vec![PathBuf::from("Documents"), PathBuf::from("Pictures")];
/// let output = PathBuf::from("Backups");
/// let progress = Progress::default();
///
/// let result = backup_gui(&folders, &output, &progress);
/// if let Ok(archive) = result {
///     println!("Backup created at {}", archive.display());
/// }
/// ```
pub fn backup_gui(
    folders: &[PathBuf],
    output_dir: &Path,
    filename: &str,
    progress: &Progress,
    verbose: bool,
    skip_locked: bool,
) -> Result<PathBuf, String> {
    if verbose {
        dlog!("[DEBUG] backup_gui: Started");
        dlog!("[DEBUG] Output directory: {}", output_dir.display());
    }

    let zip_path = output_dir.join(filename);
    if verbose { dlog!("[DEBUG] Creating backup archive: {}", zip_path.display()); }

    let tar_file = File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut tar_builder = Builder::new(tar_file);

    // Start the fingerprint with identifier + info section
    let mut fingerprint_content = format!("{}\n[Backup Info]\n", get_fingered());

    // Generate stable UUID mapping for top-level input
    let folder_uuid: Vec<(Uuid, &PathBuf)> = folders
        .iter()
        .map(|folder| {
            let uuid = Uuid::new_v4();
            if verbose { dlog!("[DEBUG] Assigned UUID {} to {}", uuid, folder.display()); }
            (uuid, folder)
        })
        .collect();

    let mut done = 0u32;

    // Write UUID ↔ original path mappings to fingerprint section
    for (uuid, original_path) in &folder_uuid {
        fingerprint_content.push_str(&format!("{}: {}\n", uuid, original_path.display()));
    }

    // Construct and append fingerprint.txt metadata file
    let mut fingerprint_header = Header::new_gnu();
    fingerprint_header.set_size(fingerprint_content.len() as u64);
    fingerprint_header.set_mode(0o644);
    fingerprint_header.set_mtime(Local::now().timestamp() as u64);
    fingerprint_header.set_cksum();

    tar_builder
        .append_data(
            &mut fingerprint_header,
            "fingerprint.txt",
            fingerprint_content.as_bytes(),
        )
        .map_err(|e| e.to_string())?;
    if verbose { dlog!("[DEBUG] fingerprint.txt added to archive"); }

    // Pre-collect all entries so we count and iterate in one filesystem pass.
    // Each element is (uuid, original_path, walk_entries_or_none).
    let mut all_entries: Vec<(Uuid, &PathBuf, Vec<walkdir::DirEntry>)> = Vec::new();
    let mut total_files: u32 = 0;

    for (uuid, original_path) in &folder_uuid {
        if original_path.is_file() {
            total_files += 1;
            all_entries.push((*uuid, original_path, Vec::new()));
        } else {
            let entries: Vec<_> = WalkDir::new(original_path)
                .into_iter()
                .filter_map(Result::ok)
                .collect();
            total_files += entries.iter().filter(|e| e.file_type().is_file()).count() as u32;
            all_entries.push((*uuid, original_path, entries));
        }
    }
    let total_files = total_files.max(1);

    // === Main archive population ===
    for (uuid, original_path, walk_entries) in all_entries {
        if original_path.is_file() {
            if verbose { dlog!("[DEBUG] Adding single file: {}", original_path.display()); }

            let metadata = original_path.metadata().map_err(|e| e.to_string())?;
            let mut header = Header::new_gnu();
            header.set_metadata(&metadata);
            header.set_cksum();

            let mut f = match File::open(original_path) {
                Ok(f) => f,
                Err(e) => {
                    if skip_locked {
                        dlog!("[WARN] Skipping inaccessible file {}: {e}", original_path.display());
                        done += 1;
                        progress.set(done * 100 / total_files);
                        continue;
                    }
                    return Err(e.to_string());
                }
            };

            let entry_name = match original_path.extension().and_then(|e| e.to_str()) {
                Some(ext) => format!("{uuid}.{ext}"),
                None => uuid.to_string(),
            };
            if verbose { dlog!("[DEBUG] -> Entry name in tar: {entry_name}"); }

            if let Err(e) = tar_builder.append_data(&mut header, entry_name, &mut f) {
                if skip_locked {
                    dlog!("[WARN] Skipping file {} (write error: {e})", original_path.display());
                    done += 1;
                    progress.set(done * 100 / total_files);
                    continue;
                }
                return Err(e.to_string());
            }

            done += 1;
            progress.set(done * 100 / total_files);

            continue;
        }

        if verbose { dlog!("[DEBUG] Walking folder: {}", original_path.display()); }

        for entry in walk_entries {
            let entry_path = entry.path();
            let metadata = entry.metadata().map_err(|e| e.to_string())?;

            let relative_path = entry_path.strip_prefix(original_path).unwrap();
            let tar_entry_path = Path::new(&uuid.to_string()).join(relative_path);

            let mut header = Header::new_gnu();
            header.set_metadata(&metadata);
            header.set_cksum();

            if metadata.is_file() {
                if verbose { dlog!("[DEBUG] Adding file: {}", entry_path.display()); }
                let mut file = match File::open(entry_path) {
                    Ok(f) => f,
                    Err(e) => {
                        if skip_locked {
                            dlog!("[WARN] Skipping inaccessible file {}: {e}", entry_path.display());
                            done += 1;
                            progress.set(done * 100 / total_files);
                            continue;
                        }
                        return Err(e.to_string());
                    }
                };
                if let Err(e) = tar_builder.append_data(&mut header, tar_entry_path, &mut file) {
                    if skip_locked {
                        dlog!("[WARN] Skipping file {} (write error: {e})", entry_path.display());
                        done += 1;
                        progress.set(done * 100 / total_files);
                        continue;
                    }
                    return Err(e.to_string());
                }

                done += 1;
                progress.set(done * 100 / total_files);
            } else if metadata.is_dir() {
                if verbose { dlog!("[DEBUG] Adding directory: {}", entry_path.display()); }
                tar_builder
                    .append_data(&mut header, tar_entry_path, io::empty())
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    // Finalize and flush .tar structure to disk
    tar_builder.finish().map_err(|e| e.to_string())?;
    if verbose { dlog!("[DEBUG] Archive finished: {}", zip_path.display()); }

    progress.done();

    Ok(zip_path)
}

// --- Legacy ZIP format (deprecated) ---
//
//
// let file = File::create(&zip_path).map_err(|e| e.to_string())?;
// let mut zip = ZipWriter::new(file);
// let options: FileOptions<'_, ()> = FileOptions::default().compression_method(
//     CompressionMethod::Deflated
// );
//
// zip.start_file("fingerprint.txt", options).unwrap();
// let mut fingerprint = format!("{}\n[Backup Info]\n", get_fingered());
// for folder in folders {
//     if let Some(name) = folder.file_name() {
//         fingerprint.push_str(&format!(
//             "{}: {}\n",
//             name.to_string_lossy(),
//             folder.display()
//         ));
//     }
// }
//
// zip.write_all(fingerprint.as_bytes()).unwrap();
//
// for path in folders {
//     if path.is_file() {
//         let filename = path.file_name().unwrap().to_string_lossy();
//         zip.start_file(filename, options).unwrap();
//         let mut f = File::open(path).unwrap();
//         io::copy(&mut f, &mut zip).unwrap();
//     } else if path.is_dir() {
//         for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
//             let entry_path = entry.path();
//             let relative = match entry_path.strip_prefix(path) {
//                 Ok(r) => r,
//                 Err(_) => {
//                     continue;
//                 }
//             };
//
//             let zip_folder = path.file_name().unwrap();
//             let final_path = Path::new(zip_folder).join(relative);
//
//             if entry_path.is_file() {
//                 zip.start_file(final_path.to_string_lossy(), options)
//                     .unwrap();
//                 let mut f = File::open(entry_path).unwrap();
//                 io::copy(&mut f, &mut zip).unwrap();
//             } else if !relative.as_os_str().is_empty() {
//                 zip.add_directory(final_path.to_string_lossy(), options)
//                     .unwrap();
//             }
//         }
//     }
//     }
//
//     zip.finish().unwrap();
//     Ok(zip_path)
// }

