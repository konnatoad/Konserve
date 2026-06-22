//! Restore module
//!
//! Handles extraction of `.tar` backups
//!
//! Validates the archive using fingerprint.txt
//! Reconstructs file paths from UUID mappings
//! Supports restoring either the entire backup or a subset chosen in the UI
use crate::helpers::{Progress, adjust_path, get_fingered};
use crate::dlog;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tar::Archive;

/// Normalize a string path to a canonical form.
///
/// Converts Windows-style backslashes (`\`) into forward slashes (`/`)
/// to make path comparison consistent across platforms.
///
/// # Arguments
/// - `s`: Path-like string slice.
///
/// # Returns
/// - A `String` with normalized separators.
///
/// # Example
/// ```
/// let p = canon("C:\\Users\\Jootu\\Documents");
/// assert_eq!(p, "C:/Users/Jootu/Documents");
/// ```
fn canon<S: AsRef<str>>(s: S) -> String {
    s.as_ref().replace('\\', "/")
}

/// Restore files from a `.tar` backup archive.
///
/// Reads a `.tar` file created by [`backup_gui`](crate::backup::backup_gui),
/// validates its fingerprint, and restores files to their original locations.
/// Optionally, only a subset of files chosen by the user is restored.
///
/// # Arguments
/// - `zip_path`: Path to the `.tar` archive.
/// - `selected`: Optional list of human-readable file paths chosen by the user.
///   If `None`, all files in the archive are restored.
/// - `status`: Shared string for UI status updates.
/// - `progress`: [`Progress`] counter to update GUI progress bars.
///
/// # Returns
/// - `Ok(())` if the restore completed successfully.
/// - `Err(String)` with an error message if restore failed.
///
/// # Notes
/// - The function looks for a `fingerprint.txt` file inside the archive
///   to validate the backup and reconstruct UUID mappings.
/// - Paths are adapted to the current user's home directory where needed.
pub fn restore_backup(
    zip_path: &PathBuf,
    selected: Option<Vec<String>>,
    status: Arc<Mutex<String>>,
    progress: &Progress,
    verbose: bool,
) -> Result<(), String> {
    *status.lock().unwrap() = "Restoring backup…".into();

    // Open archive and locate fingerprint
    let mut archive = Archive::new(File::open(zip_path).map_err(|e| e.to_string())?);
    let mut path_map: HashMap<String, PathBuf> = HashMap::new();
    let mut valid_fingerprint = false;

    for entry_res in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry_res.map_err(|e| e.to_string())?;
        let header_path = entry.path().map_err(|e| e.to_string())?;
        let entry_name = header_path.to_string_lossy();

        // Parse fingerprint.txt to reconstruct UUID mappings
        if entry_name == "fingerprint.txt" {
            let mut txt = String::new();
            entry.read_to_string(&mut txt).map_err(|e| e.to_string())?;

            // Abort if the fingerprint marker doesn't match the expected build
            if txt.contains(get_fingered()) {
                valid_fingerprint = true;

                for line in txt.lines().filter(|l| l.contains(": ")) {
                    if let Some((uuid, p)) = line.split_once(": ") {
                        path_map.insert(uuid.to_string(), PathBuf::from(p.trim()));
                    }
                }
            }
            break;
        }
    }

    if !valid_fingerprint {
        return Err("Invalid backup fingerprint.".into());
    }

    if verbose { dlog!("[fingerprint] loaded, {} uuids", path_map.len()); }

    let mut to_extract: HashSet<String> = HashSet::new();

    if let Some(human_sel_raw) = &selected {
        let human_sel: HashSet<String> = human_sel_raw.iter().map(canon).collect();

        for (uuid, orig) in &path_map {
            let parent_c = canon(orig.parent().unwrap_or(orig).display().to_string());
            let item_name = orig.file_name().unwrap_or_default().to_string_lossy();
            let base = format!("{parent_c}/{item_name}");
            let base_slash = format!("{base}/");

            if human_sel.contains(&base) {
                to_extract.insert(uuid.clone());

                if let Some(ext) = orig.extension().and_then(|e| e.to_str()) {
                    to_extract.insert(format!("{uuid}.{ext}"));
                }
            }

            for h in &human_sel {
                if let Some(rest) = h.strip_prefix(&base_slash) {
                    to_extract.insert(format!("{uuid}/{rest}"));
                }
            }
        }
    }

    // Count is tracked during extraction to avoid a second full archive pass.
    let mut total_files: u32 = 1;
    let mut done: u32 = 0;

    if verbose { dlog!("[select]  to_extract = {to_extract:?}"); }

    // Begin extraction
    let current_home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("C:\\"));
    let mut archive = Archive::new(File::open(zip_path).map_err(|e| e.to_string())?);

    if verbose { dlog!("[extract] scanning archive…"); }
    let mut restored_count = 0;

    for entry_res in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry_res.map_err(|e| e.to_string())?;
        let tar_path_ref = entry.path().map_err(|e| e.to_string())?;
        let path_in_tar = tar_path_ref.to_string_lossy().into_owned();

        if path_in_tar == "fingerprint.txt" {
            continue;
        }

        // If selection is active, skip entries that don't match exactly or aren't
        // inside a selected top-level folder (uuid/ prefix).
        if selected.is_some()
            && !to_extract.contains(&path_in_tar)
            && !to_extract.iter().any(|s| path_in_tar.starts_with(&format!("{s}/")))
        {
            if verbose { dlog!("[skip]    {path_in_tar}  (not selected)"); }
            continue;
        }

        total_files += 1;

        let tar_path = Path::new(&path_in_tar);
        let root_component = match tar_path.components().next() {
            Some(c) => c.as_os_str().to_string_lossy().into_owned(),
            None => {
                if verbose { dlog!("[skip]    {path_in_tar}  (empty path)"); }
                continue;
            }
        };

        // Case 1: UUID prefix = folder root
        if let Some(orig_base) = path_map.get(&root_component) {
            let adjusted_base = adjust_path(orig_base, &current_home, verbose);
            let rel = tar_path
                .strip_prefix(Path::new(&root_component))
                .unwrap_or_else(|_| Path::new(""));

            let unpack_to = adjusted_base.join(rel);
            if verbose { dlog!("[write] dir {path_in_tar}  →  {}", unpack_to.display()); }

            if let Some(dir) = unpack_to.parent() {
                fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            entry.unpack(&unpack_to).map_err(|e| e.to_string())?;
            restored_count += 1;
            done += 1;
            progress.set((done * 100) / total_files);
        }
        // Case 2: UUID.ext = standalone file
        else if let Some((uuid_part, _ext)) = root_component.split_once('.') {
            if let Some(orig_file) = path_map.get(uuid_part) {
                let unpack_to = adjust_path(orig_file, &current_home, verbose);
                if verbose { dlog!("[write] file {path_in_tar}  →  {}", unpack_to.display()); }

                if let Some(dir) = unpack_to.parent() {
                    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                }
                entry.unpack(&unpack_to).map_err(|e| e.to_string())?;
                restored_count += 1;
                done += 1;
                progress.set((done * 100) / total_files);
            } else {
                if verbose { dlog!("[skip]    {path_in_tar}  (uuid not in map)"); }
            }
        } else {
            if verbose { dlog!("[skip]    {path_in_tar}  (no handler)"); }
        }
    }

    if verbose { dlog!("[done]   restored {restored_count} entries"); }
    *status.lock().unwrap() = "✅ Restore complete.".into();
    progress.done();
    Ok(())
}

