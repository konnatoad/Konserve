//! Creates `.tar` backup archives with embedded `fingerprint.txt` path mappings.
use crate::helpers::{Progress, get_fingered};
use crate::{clog, dlog};
use std::io::BufWriter;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

use chrono::Local;
use tar::{Builder, Header};
use uuid::Uuid;
use walkdir::WalkDir;

/// Pack the given files/folders into a `.tar` archive with a `fingerprint.txt` inside.
/// Returns the path to the created archive.
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
    if verbose {
        dlog!("[DEBUG] Creating backup archive: {}", zip_path.display());
    }

    let tar_file = File::create(&zip_path).map_err(|e| {
        let msg = format!(
            "ERROR: failed to create archive {}: {e}",
            zip_path.display()
        );
        clog!("{msg}");
        msg
    })?;
    let mut tar_builder = Builder::new(BufWriter::new(tar_file));

    // Start the fingerprint with identifier + info section
    let mut fingerprint_content = format!("{}\n[Backup Info]\n", get_fingered());

    // Generate stable UUID mapping for top-level input
    let folder_uuid: Vec<(Uuid, &PathBuf)> = folders
        .iter()
        .map(|folder| {
            let uuid = Uuid::new_v4();
            if verbose {
                dlog!("[DEBUG] Assigned UUID {} to {}", uuid, folder.display());
            }
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
    if verbose {
        dlog!("[DEBUG] fingerprint.txt added to archive");
    }

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
            if verbose {
                dlog!("[DEBUG] Adding single file: {}", original_path.display());
            }

            let metadata = match original_path.metadata() {
                Ok(m) => m,
                Err(e) => {
                    if skip_locked {
                        done += 1;
                        progress.set(done * 100 / total_files);
                        continue;
                    }
                    clog!("ERROR: cannot stat file {}: {e}", original_path.display());
                    return Err(e.to_string());
                }
            };
            let mut header = Header::new_gnu();
            header.set_metadata(&metadata);
            header.set_cksum();

            let mut f = match File::open(original_path) {
                Ok(f) => f,
                Err(e) => {
                    if skip_locked {
                        dlog!(
                            "[WARN] Skipping inaccessible file {}: {e}",
                            original_path.display()
                        );
                        done += 1;
                        progress.set(done * 100 / total_files);
                        continue;
                    }
                    clog!("ERROR: cannot open file {}: {e}", original_path.display());
                    return Err(e.to_string());
                }
            };

            let entry_name = match original_path.extension().and_then(|e| e.to_str()) {
                Some(ext) => format!("{uuid}.{ext}"),
                None => uuid.to_string(),
            };
            if verbose {
                dlog!("[DEBUG] -> Entry name in tar: {entry_name}");
            }

            if let Err(e) = tar_builder.append_data(&mut header, entry_name, &mut f) {
                if skip_locked {
                    dlog!(
                        "[WARN] Skipping file {} (write error: {e})",
                        original_path.display()
                    );
                    done += 1;
                    progress.set(done * 100 / total_files);
                    continue;
                }
                clog!(
                    "ERROR: failed to write {} to archive: {e}",
                    original_path.display()
                );
                return Err(e.to_string());
            }

            done += 1;
            progress.set(done * 100 / total_files);

            continue;
        }

        if verbose {
            dlog!("[DEBUG] Walking folder: {}", original_path.display());
        }

        for entry in walk_entries {
            let entry_path = entry.path();
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    if skip_locked {
                        continue;
                    }
                    clog!("ERROR: cannot stat {}: {e}", entry_path.display());
                    return Err(e.to_string());
                }
            };

            let relative_path = match entry_path.strip_prefix(original_path) {
                Ok(p) => p,
                Err(_) => {
                    if verbose {
                        dlog!(
                            "[WARN] skipping entry outside original_path: {}",
                            entry_path.display()
                        );
                    }
                    continue;
                }
            };
            let tar_entry_path = Path::new(&uuid.to_string()).join(relative_path);

            let mut header = Header::new_gnu();
            header.set_metadata(&metadata);
            header.set_cksum();

            if metadata.is_file() {
                if verbose {
                    dlog!("[DEBUG] Adding file: {}", entry_path.display());
                }
                let mut file = match File::open(entry_path) {
                    Ok(f) => f,
                    Err(e) => {
                        if skip_locked {
                            dlog!(
                                "[WARN] Skipping inaccessible file {}: {e}",
                                entry_path.display()
                            );
                            done += 1;
                            progress.set(done * 100 / total_files);
                            continue;
                        }
                        clog!("ERROR: cannot open file {}: {e}", entry_path.display());
                        return Err(e.to_string());
                    }
                };
                if let Err(e) = tar_builder.append_data(&mut header, tar_entry_path, &mut file) {
                    if skip_locked {
                        dlog!(
                            "[WARN] Skipping file {} (write error: {e})",
                            entry_path.display()
                        );
                        done += 1;
                        progress.set(done * 100 / total_files);
                        continue;
                    }
                    clog!(
                        "ERROR: failed to write {} to archive: {e}",
                        entry_path.display()
                    );
                    return Err(e.to_string());
                }

                done += 1;
                progress.set(done * 100 / total_files);
            } else if metadata.is_dir() {
                if verbose {
                    dlog!("[DEBUG] Adding directory: {}", entry_path.display());
                }
                if let Err(e) = tar_builder.append_data(&mut header, tar_entry_path, io::empty())
                    && !skip_locked
                {
                    return Err(e.to_string());
                }
            }
        }
    }

    // Finalize and flush .tar structure to disk
    tar_builder.finish().map_err(|e| {
        let msg = format!(
            "ERROR: failed to finalize archive {}: {e}",
            zip_path.display()
        );
        clog!("{msg}");
        msg
    })?;
    if verbose {
        dlog!("[DEBUG] Archive finished: {}", zip_path.display());
    }

    progress.done();

    Ok(zip_path)
}
