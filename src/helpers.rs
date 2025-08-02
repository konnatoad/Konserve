//! # Helpers Module
//!
//! Provides shared utilities for the app, including:
//! - Persistent configuration handling (`KonserveConfig`)
//! - Compression and conflict resolution enums
//! - Progress tracking via atomic counters
//! - Path adjustment and validation helpers
//! - Tree rendering logic for the restore selection UI
//! - Fingerprint parsing for verifying backup archives
//! - Application icon loading
//!
//! This module acts as the core glue between backup/restore logic and the GUI.
use crate::FolderTreeNode;
use eframe::egui;
use eframe::egui::IconData;
use egui::CollapsingHeader;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tar::Archive;

/// Persistent configuration object for Konserve.
///
/// Stores user preferences and feature toggles across application sessions.
/// Automatically serialized/deserialized from a JSON file in the user's
/// configuration directory.
///
/// - Path resolution favors `$XDG_CONFIG_HOME/konserve/config.json`.
/// - If missing or invalid, defaults are used.
///
/// Fields map directly to Settings tab toggles in the GUI.
#[derive(Serialize, Deserialize, Clone)]
pub struct KonserveConfig {
    /// Enables verbose debug logging when true.
    #[serde(default)]
    pub verbose_logging: bool,
    /// Enables backup compression (`.tar.gz`).
    #[serde(default)]
    pub compression_enabled: bool,
    /// Enables conflict resolution when restoring files.
    #[serde(default)]
    pub conflict_resolution_enabled: bool,
    /// Strategy for handling name collisions during restore.
    #[serde(default)]
    pub conflict_resolution_mode: super::ConflictResolutionMode,
    /// Optional default folder where backups will be saved.
    #[serde(default)]
    pub default_backup_location: Option<PathBuf>,
    /// Whether to check for updates automatically on startup.
    #[serde(default)]
    pub automatic_updates: bool,
    /// Show a summary of file sizes during backup/restore.
    #[serde(default)]
    pub file_size_summary: bool,
}

/// Provides default values for [`KonserveConfig`].
///
/// Called automatically when no saved config exists, or
/// when [`KonserveConfig::load`] fails to parse the config file.
/// Ensures the application always starts with a valid configuration.
///
/// # Defaults
/// - `verbose_logging`: `false` — disables detailed debug logs
/// - `compression_enabled`: `false` — compression disabled by default
/// - `conflict_resolution_enabled`: `false` — conflict resolution off
/// - `conflict_resolution_mode`: [`ConflictResolutionMode::Prompt`] (default)
/// - `default_backup_location`: `None` — user must select manually
/// - `automatic_updates`: `false` — no automatic update checks
/// - `file_size_summary`: `false` — skip size summaries during backups
impl Default for KonserveConfig {
    fn default() -> Self {
        Self {
            verbose_logging: false,
            compression_enabled: false,
            conflict_resolution_enabled: false,
            conflict_resolution_mode: super::ConflictResolutionMode::default(),
            default_backup_location: None,
            automatic_updates: false,
            file_size_summary: false,
        }
    }
}

impl KonserveConfig {
    /// Resolves the absolute path to the configuration file.
    ///
    /// Priority order for the base directory:
    /// 1. `$XDG_CONFIG_HOME` (preferred)
    /// 2. `$XDG_DATA_HOME` or equivalent data directory
    /// 3. The user’s home directory
    /// 4. Current working directory (`.`) as a fallback
    ///
    /// The final path will always be:
    /// `<base>/konserve/config.json`
    ///
    /// # Returns
    /// A [`PathBuf`] pointing to the expected config file location.
    fn config_path() -> PathBuf {
        let base = dirs::config_dir()
            .or_else(dirs::data_dir) // fallback
            .or_else(dirs::home_dir)
            .unwrap_or(PathBuf::from("."));

        base.join("konserve").join("config.json")
    }

    /// Loads the configuration from disk.
    ///
    /// - Attempts to read the JSON config file at [`Self::config_path`].
    /// - If the file exists and deserializes correctly, returns the stored config.
    /// - If missing or malformed, returns [`Self::default`].
    ///
    /// # Debug Output
    /// Prints `[DEBUG] Loading config from <path>` when successful.
    ///
    /// # Returns
    /// A [`KonserveConfig`] struct with either persisted values or defaults.
    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str(&data) {
                println!("[DEBUG] Loading config from {}", path.display());
                return cfg;
            }
        }
        Self::default()
    }

    /// Saves the current configuration to disk.
    ///
    /// - Resolves the config path via [`Self::config_path`].
    /// - Creates parent directories if missing.
    /// - Serializes the configuration to pretty-printed JSON.
    /// - Writes to the resolved file.
    ///
    /// # Errors
    /// - If serialization fails, logs `[ERROR] Failed to serialize config`.
    /// - If writing fails, logs `[ERROR] Failed to save config`.
    ///
    /// # Debug Output
    /// Prints `[DEBUG] Saved config to <path>` on success.
    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }

        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = fs::write(&path, json) {
                    eprintln!("[ERROR] Failed to save config: {e}");
                }
                println!("[DEBUG] Saved config to {}", path.display());
            }
            Err(e) => {
                eprintln!("[ERROR] Failed to serialize config: {e}");
            }
        }
    }
}

/// Determines how name collisions are resolved during restore.
///
/// Used when a file to be restored already exists at the target path.
///
/// Variants:
/// - `Prompt`: Ask the user for each conflict (default).
/// - `Overwrite`: Replace the existing file.
/// - `Skip`: Leave the existing file unchanged.
/// - `Rename`: Write the restored file with a new name.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy, Default)]
pub enum ConflictResolutionMode {
    #[default]
    Prompt, // Ask user
    Overwrite, // Overwrite destination
    Skip,      // Skip conflicting file
    Rename,    // Rename on conflict
}

/// Atomic counter for tracking progress percentages.
///
/// Shared across threads via `Arc`.
/// Used to update the GUI progress bar in real-time.
///
/// - `set`: Updates progress to a given percentage (0–100).
/// - `get`: Reads the current percentage.
/// - `done`: Marks the operation as complete (101%).
#[derive(Clone)]
pub struct Progress {
    inner: Arc<AtomicU32>,
}

impl Progress {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn set(&self, pct: u32) {
        // Used relaxed ordering, as exact timing isn't critical
        self.inner.store(pct, Ordering::Relaxed);
    }
    pub fn get(&self) -> u32 {
        self.inner.load(Ordering::Relaxed)
    }
    pub fn done(&self) {
        self.set(101);
    }
}
impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

/// Loads the Konserve application icon into memory for GUI initialization.
///
/// Reads the PNG bytes embedded at compile time (`assets/icon.png`)
/// and converts them into an [`IconData`] suitable for `eframe`.
///
/// # Panics
/// Panics if the icon cannot be decoded.
///
/// # Returns
/// An [`Arc<IconData>`] containing the icon.
pub fn load_icon_image() -> Arc<IconData> {
    println!("[DEBUG] load_icon_image: Start");

    let image_bytes = include_bytes!("../assets/icon.png");
    println!("[DEBUG] Icon bytes loaded: {} bytes", image_bytes.len());

    let image = image::load_from_memory(image_bytes)
        .expect("Icon image couldn't be loaded")
        .into_rgba8();

    let (w, h) = image.dimensions();
    println!("[DEBUG] Icon dimensions: {w}x{h}");

    let icon_data = Arc::new(IconData {
        rgba: image.into_raw(),
        width: w,
        height: h,
    });

    println!("[DEBUG] load_icon_image: Done");
    icon_data
}

/// Recursively toggles the checked state of a folder node and all its children.
///
/// Ensures that when a parent folder is selected/deselected,
/// all contained files/folders update to match.
///
/// - `node`: The current tree node.
/// - `checked`: Desired checkbox state.
fn set_all_checked(node: &mut FolderTreeNode, checked: bool) {
    println!(
        "[DEBUG] set_all_checked: Setting node (is_file: {}) to checked = {}",
        node.is_file, checked
    );

    node.checked = checked;
    for (name, child) in node.children.iter_mut() {
        println!("[DEBUG]   -> Descending into child: \"{name}\"");
        set_all_checked(child, checked);
    }
}

/// Renders a hierarchical folder/file tree in the restore selection UI.
///
/// Uses collapsible folders and checkboxes.
/// Maintains parent-child sync when toggling.
///
/// - `ui`: egui UI handle for rendering.
/// - `path`: Mutable path stack for recursion.
/// - `node`: Current folder node to render.
pub fn render_tree(ui: &mut egui::Ui, path: &mut Vec<String>, node: &mut FolderTreeNode) {
    for (name, child) in node.children.iter_mut() {
        let mut label = name.clone();
        if !child.is_file {
            label.push('/');
        }

        path.push(name.clone());
        let current_path = path.join("/");

        if child.children.is_empty() {
            // Leaf file node
            ui.horizontal(|ui| {
                ui.checkbox(&mut child.checked, "");
                ui.label(label);
            });
        } else {
            // Folder node with children
            ui.horizontal(|ui| {
                if ui.checkbox(&mut child.checked, "").changed() {
                    println!(
                        "[DEBUG] Checkbox changed: setting all children of \"{}\" to {}",
                        current_path, child.checked
                    );
                    set_all_checked(child, child.checked);
                }
                CollapsingHeader::new(label)
                    .default_open(false)
                    .show(ui, |ui| {
                        // Render the children of the current node recursively.
                        render_tree(ui, path, child);
                    });
            });

            // Maintain oarent state if any child is still checked
            child.checked = child.children.values().any(|c| c.checked);
        }

        path.pop();
    }
}

/// Constructs a user-friendly [`FolderTreeNode`] hierarchy from archive data.
///
/// Takes a list of tar entry paths and a UUID → original path map,
/// then builds a tree that mirrors the original folder structure.
///
/// Used in the restore UI after parsing `fingerprint.txt`.
///
/// - `entries`: All archive file paths.
/// - `path_map`: Maps UUIDs to original system paths.
pub fn build_human_tree(
    entries: Vec<String>,
    path_map: HashMap<String, PathBuf>,
) -> FolderTreeNode {
    println!("[DEBUG] build_human_tree: Start");
    let mut root = FolderTreeNode::default();

    for (uuid, original_path) in path_map {
        println!("[DEBUG] Processing UUID: {uuid}, Path: {original_path:?}");

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

        println!("[DEBUG] parent_label = \"{parent_label}\", item_name = \"{item_name}\"");

        let parent_node = root
            .children
            .entry(parent_label.clone())
            .or_insert_with(FolderTreeNode::default);

        parent_node
            .children
            .entry(item_name.clone())
            .or_insert_with(FolderTreeNode::default);

        let dir_prefix = format!("{uuid}/"); // Create a prefix for directory entries based on the UUID.
        let is_dir_backup = entries.iter().any(|e| e.starts_with(&dir_prefix)); // Check if there are any entries that start with the UUID prefix.

        if is_dir_backup {
            println!("[DEBUG] Detected directory backup for UUID: {uuid}");
            parent_node.children.get_mut(&item_name).unwrap().is_file = false;

            for tar_path in entries.iter().filter(|e| e.starts_with(&dir_prefix)) {
                println!("[DEBUG]   tar_path = \"{tar_path}\"");

                let rest = tar_path[dir_prefix.len()..].trim_end_matches('/');
                if rest.is_empty() {
                    println!("[DEBUG]   Skipping empty rest after trim");
                    continue;
                }

                println!("[DEBUG]   Rest path: \"{rest}\"");

                let mut cursor = parent_node.children.get_mut(&item_name).unwrap(); // Get the item
                for part in rest.split('/') {
                    println!("[DEBUG]     Descending into part: \"{part}\"");
                    cursor = cursor
                        .children
                        .entry(part.to_string())
                        .or_insert_with(FolderTreeNode::default);
                }
                cursor.is_file = true;
            }
        } else {
            println!("[DEBUG] Detected file (not dir) for UUID: {uuid}");
            parent_node.children.get_mut(&item_name).unwrap().is_file = true;
        }
    }

    println!("[DEBUG] build_human_tree: Finished building tree");
    root
}

/// Recursively traverses a [`FolderTreeNode`] tree,
/// collecting all checked file paths into a flat list.
pub fn collect_recursive(node: &FolderTreeNode, path: &mut Vec<String>, output: &mut Vec<String>) {
    for (name, child) in &node.children {
        path.push(name.clone());
        if child.is_file && child.checked {
            let full_path = path.join("/");
            println!("[DEBUG] collect_recursive: Adding checked file {full_path}");
            output.push(full_path);
        }

        collect_recursive(child, path, output);
        path.pop();
    }
}

/// Convenience wrapper around [`collect_recursive`].
///
/// Collects all checked paths from the root node.
pub fn collect_paths(root: &FolderTreeNode) -> Vec<String> {
    println!("[DEBUG] collect_paths: Start");
    let mut result = Vec::new();
    let mut path = Vec::new();
    collect_recursive(root, &mut path, &mut result);
    println!(
        "[DEBUG] collect_paths: Done, collected {} paths",
        result.len()
    );
    result
}

/// Reads `fingerprint.txt` from a backup archive to rebuild UUID mappings.
///
/// Returns both:
/// - `entries`: List of archive file paths excluding `fingerprint.txt`.
/// - `path_map`: UUID → original path mappings for restoration.
///
/// # Errors
/// Returns `Err` if the archive is invalid or fingerprint is missing.
pub fn parse_fingerprint(
    zip_path: &PathBuf,
) -> Result<(Vec<String>, HashMap<String, PathBuf>), String> {
    println!(
        "[DEBUG] parse_fingerprint: Opening archive at {}",
        zip_path.display()
    );

    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = Archive::new(file);
    let mut path_map = HashMap::new();

    println!("[DEBUG] Scanning for fingerprint.txt…");

    // Phase 1: extract fingerprint map
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let header_path = entry.path().map_err(|e| e.to_string())?;
        let name = header_path.to_string_lossy();

        if name == "fingerprint.txt" {
            println!("[DEBUG] Found fingerprint.txt");
            let mut txt = String::new();
            entry.read_to_string(&mut txt).map_err(|e| e.to_string())?;

            for line in txt.lines().filter(|l| l.contains(": ")) {
                let (uuid, p) = line.split_once(": ").unwrap();
                println!("[DEBUG]   Parsed fingerprint: {} → {}", uuid, p.trim());
                path_map.insert(uuid.to_string(), PathBuf::from(p.trim()));
            }
            break;
        }
    }

    println!("[DEBUG] Re-opening archive to collect entries");

    // Phase 2: list remaining archive contents
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = Archive::new(file);
    let mut entries = Vec::new();

    for entry in archive.entries().map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry.path().map_err(|e| e.to_string())?;
        let entry_name = entry_path.to_string_lossy().into_owned();

        if entry_name != "fingerprint.txt" {
            entries.push(entry_name.clone());
            println!("[DEBUG]   Found entry: {entry_name}");
        }
    }

    println!(
        "[DEBUG] parse_fingerprint: Done. {} entries, {} fingerprinted",
        entries.len(),
        path_map.len()
    );

    Ok((entries, path_map))
}

/// Returns the Konserve build fingerprint.
///
/// Used to verify that a backup was created by this build variant.
/// Reads from `FINGERPRINT` environment variable at compile time.
/// Defaults to `"DEFAULT_FINGERPRINT"` if not set.
pub fn get_fingered() -> &'static str {
    const DEFAULT: &str = "DEFAULT_FINGERPRINT";

    match option_env!("FINGERPRINT") {
        Some(val) => {
            println!("get_fingered: using embedded fingerprint = \"{val}\"");
            val
        }
        None => {
            println!("get_fingered: no embedded fingerprint found, fallback \"{DEFAULT}\"");
            DEFAULT
        }
    }
}

/// Adjusts a Windows-style path to match the current system's user home.
///
/// Example:
/// - Original: `C:\Users\olduser\Documents\file.txt`
/// - Adjusted: `C:\Users\currentuser\Documents\file.txt`
///
/// - `original`: Path stored in backup.
/// - `current_home`: Current user's home directory.
///
/// Returns the adjusted path.
pub fn adjust_path(original: &Path, current_home: &Path) -> PathBuf {
    let og_str = original.to_string_lossy();
    let current_str = current_home.to_string_lossy();

    println!("[DEBUG] adjust_path: original = {og_str}");
    println!("[DEBUG] adjust_path: current_home = {current_str}");

    if og_str.to_lowercase().starts_with("c:\\users\\") {
        let parts: Vec<&str> = og_str.split('\\').collect();
        if parts.len() > 2 {
            let old_username = parts[2];
            let expected_prefix = format!("C:\\Users\\{old_username}");
            println!("[DEBUG] Detected old user prefix: {expected_prefix}");

            if og_str.starts_with(&expected_prefix) {
                let rel_path = og_str.strip_prefix(&expected_prefix).unwrap_or("");
                let adjusted = format!("{current_str}{rel_path}");
                println!("[DEBUG] Path adjusted: {og_str} → {adjusted}");
                return PathBuf::from(adjusted);
            }
        }
    }

    println!("[DEBUG] No adjustment needed");
    original.to_path_buf()
}

/// Validates whether a stored path exists, adjusting if necessary.
///
/// - Uses [`adjust_path`] if the original is invalid.
/// - Returns `Some(path)` if a usable path is found.
/// - Returns `None` if both original and adjusted paths are invalid.
pub fn fix_skip(p: &Path) -> Option<PathBuf> {
    println!("[DEBUG] fix_skip: Checking path {}", p.display());

    if p.exists() {
        println!("[DEBUG] -> Path exists, using as-is");
        return Some(p.to_path_buf());
    }

    let current_home = dirs::home_dir()?; // Get the current user's home directory.
    let adjusted = adjust_path(p, &current_home); // Adjust the path based on the current home directory.

    if adjusted.exists() {
        println!(
            "[DEBUG] -> Adjusted path exists: using {}",
            adjusted.display()
        );
        Some(adjusted)
    } else {
        println!(
            "[DEBUG] -> Neither original nor adjusted path exists ({} -> {})",
            p.display(),
            adjusted.display()
        );
        None
    }
}
