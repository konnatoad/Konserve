//! Extracts `.tar` backups, validates the fingerprint, and reconstructs original file paths.
use crate::helpers::{ConflictResolutionMode, Progress, adjust_path, get_fingered};
use crate::{clog, dlog};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};
use tar::Archive;

/// Per-file answer sent from the UI back to a paused restore thread.
pub enum ConflictAnswer {
    Overwrite,
    Skip,
    Rename,
}

/// Return the path to write to, or `None` to skip the file.
fn resolve_conflict(
    dest: &Path,
    mode: ConflictResolutionMode,
    ch: &Option<(mpsc::Sender<PathBuf>, mpsc::Receiver<ConflictAnswer>)>,
) -> Option<PathBuf> {
    if !dest.exists() {
        return Some(dest.to_path_buf());
    }
    match mode {
        ConflictResolutionMode::Overwrite => Some(dest.to_path_buf()),
        ConflictResolutionMode::Skip => None,
        ConflictResolutionMode::Rename => Some(unique_path(dest)),
        ConflictResolutionMode::Prompt => {
            if let Some((tx, rx)) = ch {
                if tx.send(dest.to_path_buf()).is_err() {
                    return None;
                }
                match rx.recv() {
                    Ok(ConflictAnswer::Overwrite) => Some(dest.to_path_buf()),
                    Ok(ConflictAnswer::Skip) => None,
                    Ok(ConflictAnswer::Rename) => Some(unique_path(dest)),
                    Err(_) => None,
                }
            } else {
                Some(dest.to_path_buf())
            }
        }
    }
}

/// Append `_1`, `_2`, … before the extension until a free path is found.
fn unique_path(dest: &Path) -> PathBuf {
    let stem = dest.file_stem().unwrap_or_default().to_string_lossy();
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = dest.parent().unwrap_or_else(|| Path::new(""));
    let mut i = 1u32;
    loop {
        let candidate = parent.join(format!("{stem}_{i}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

/// Normalize path separators to `/` for consistent comparison.
fn canon<S: AsRef<str>>(s: S) -> String {
    s.as_ref().replace('\\', "/")
}

/// Restore files from a `.tar` archive. If `selected` is provided, only those paths are restored.
pub fn restore_backup(
    zip_path: &PathBuf,
    selected: Option<Vec<String>>,
    status: Arc<Mutex<String>>,
    progress: &Progress,
    verbose: bool,
    mode: ConflictResolutionMode,
    conflict_ch: Option<(mpsc::Sender<PathBuf>, mpsc::Receiver<ConflictAnswer>)>,
) -> Result<(), String> {
    *status.lock().unwrap() = "Restoring backup…".into();

    // Open archive and locate fingerprint
    let mut archive = Archive::new(File::open(zip_path).map_err(|e| {
        let msg = format!("ERROR: cannot open archive {}: {e}", zip_path.display());
        clog!("{msg}");
        msg
    })?);
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
        clog!(
            "ERROR: restore aborted — invalid or missing backup fingerprint in {}",
            zip_path.display()
        );
        return Err("Invalid backup fingerprint.".into());
    }

    if verbose {
        dlog!("[fingerprint] loaded, {} uuids", path_map.len());
    }

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

    if verbose {
        dlog!("[select]  to_extract = {to_extract:?}");
    }

    // Begin extraction
    let current_home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("C:\\"));
    let mut archive = Archive::new(File::open(zip_path).map_err(|e| {
        let msg = format!(
            "ERROR: cannot reopen archive for extraction {}: {e}",
            zip_path.display()
        );
        clog!("{msg}");
        msg
    })?);

    if verbose {
        dlog!("[extract] scanning archive…");
    }
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
            && !to_extract.iter().any(|s| {
                path_in_tar.len() > s.len()
                    && path_in_tar.as_bytes()[s.len()] == b'/'
                    && path_in_tar.starts_with(s.as_str())
            })
        {
            if verbose {
                dlog!("[skip]    {path_in_tar}  (not selected)");
            }
            continue;
        }

        total_files += 1;

        let tar_path = Path::new(&path_in_tar);
        let root_component = match tar_path.components().next() {
            Some(c) => c.as_os_str().to_string_lossy().into_owned(),
            None => {
                if verbose {
                    dlog!("[skip]    {path_in_tar}  (empty path)");
                }
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
            if verbose {
                dlog!("[write] dir {path_in_tar}  →  {}", unpack_to.display());
            }

            if let Some(final_path) = resolve_conflict(&unpack_to, mode, &conflict_ch) {
                if let Some(dir) = final_path.parent() {
                    fs::create_dir_all(dir).map_err(|e| {
                        let msg = format!("ERROR: failed to create dir {}: {e}", dir.display());
                        clog!("{msg}");
                        msg
                    })?;
                }
                entry.unpack(&final_path).map_err(|e| {
                    let msg = format!(
                        "ERROR: failed to unpack {} → {}: {e}",
                        path_in_tar,
                        final_path.display()
                    );
                    clog!("{msg}");
                    msg
                })?;
                restored_count += 1;
            } else {
                if verbose {
                    dlog!("[skip] conflict: {}", unpack_to.display());
                }
            }
            done += 1;
            progress.set((done * 100) / total_files);
        }
        // Case 2: UUID.ext = standalone file
        else if let Some((uuid_part, _ext)) = root_component.split_once('.') {
            if let Some(orig_file) = path_map.get(uuid_part) {
                let unpack_to = adjust_path(orig_file, &current_home, verbose);
                if verbose {
                    dlog!("[write] file {path_in_tar}  →  {}", unpack_to.display());
                }

                if let Some(final_path) = resolve_conflict(&unpack_to, mode, &conflict_ch) {
                    if let Some(dir) = final_path.parent() {
                        fs::create_dir_all(dir).map_err(|e| {
                            let msg = format!("ERROR: failed to create dir {}: {e}", dir.display());
                            clog!("{msg}");
                            msg
                        })?;
                    }
                    entry.unpack(&final_path).map_err(|e| {
                        let msg = format!(
                            "ERROR: failed to unpack {} → {}: {e}",
                            path_in_tar,
                            final_path.display()
                        );
                        clog!("{msg}");
                        msg
                    })?;
                    restored_count += 1;
                } else {
                    if verbose {
                        dlog!("[skip] conflict: {}", unpack_to.display());
                    }
                }
                done += 1;
                progress.set((done * 100) / total_files);
            } else {
                if verbose {
                    dlog!("[skip]    {path_in_tar}  (uuid not in map)");
                }
            }
        } else {
            if verbose {
                dlog!("[skip]    {path_in_tar}  (no handler)");
            }
        }
    }

    if verbose {
        dlog!("[done]   restored {restored_count} entries");
    }
    *status.lock().unwrap() = "✅ Restore complete.".into();
    progress.done();
    Ok(())
}
