use crate::FolderTreeNode;
use eframe::egui;
use eframe::egui::IconData;
use egui::CollapsingHeader;
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use tar::Archive;

// Represents the mode for resolving conflicts during file operations.
#[derive(PartialEq, Eq, Clone, Copy, Default)]
pub enum ConflictResolutionMode {
    #[default]
    Prompt,
    Overwrite,
    Skip,
    Rename,
}

// Represents different levels of compression for file operations.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum CompressionLevel {
    Fast,
    #[default]
    Normal,
    Maximum,
}

// Represents the progress of an operation, such as loading or processing data.
#[derive(Clone)]
pub struct Progress {
    inner: Arc<AtomicU32>,
}

// Represents a node in the folder tree structure.
impl Progress {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU32::new(0)),
        }
    }

    // Creates a new Progress instance with an initial value of 0.
    pub fn set(&self, pct: u32) {
        self.inner.store(pct, Ordering::Relaxed);
    }
    // Sets the progress to a specific percentage.
    pub fn get(&self) -> u32 {
        self.inner.load(Ordering::Relaxed)
    }
    // Retrieves the current progress percentage.
    pub fn done(&self) {
        self.set(101);
    }
}
// Marks the progress as done by setting it to 101.
impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

// if !icon.png exists, it will panic at runtime
pub fn load_icon_image() -> Arc<IconData> {
    println!("[DEBUG] load_icon_image: Start");

    // Load the icon image from the embedded bytes.
    let image_bytes = include_bytes!("../assets/icon.png");
    println!("[DEBUG] Icon bytes loaded: {} bytes", image_bytes.len());

    // Decode the image bytes into an RGBA image.
    let image = image::load_from_memory(image_bytes)
        .expect("Icon image couldn't be loaded")
        .into_rgba8();

    // Check the dimensions of the image.
    let (w, h) = image.dimensions();
    println!("[DEBUG] Icon dimensions: {w}x{h}");

    // Create an IconData instance with the image data and dimensions.
    let icon_data = Arc::new(IconData {
        rgba: image.into_raw(),
        width: w,
        height: h,
    });

    println!("[DEBUG] load_icon_image: Done");
    icon_data
}

// Sets the checked state of a node and all its children recursively.
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

// Renders a tree structure in the UI, allowing for nested folders and files.
pub fn render_tree(ui: &mut egui::Ui, path: &mut Vec<String>, node: &mut FolderTreeNode) {
    // Render the current node's children in the UI.
    for (name, child) in node.children.iter_mut() {
        // Create a label for the current node, appending a slash if it's a directory.
        let mut label = name.clone();
        if !child.is_file {
            label.push('/');
        }

        path.push(name.clone());
        let current_path = path.join("/");

        if child.children.is_empty() {
            // If the node has no children, render a checkbox for it.
            ui.horizontal(|ui| {
                ui.checkbox(&mut child.checked, "");
                ui.label(label);
            });
        } else {
            // If the node has children, render a collapsible header with a checkbox.
            ui.horizontal(|ui| {
                if ui.checkbox(&mut child.checked, "").changed() {
                    // If the checkbox state changes, set all children to the same state.
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
            child.checked = child.children.values().any(|c| c.checked);
        }

        path.pop();
    }
}

// Builds a human-readable tree structure from a list of entries and a mapping of UUIDs to paths.
pub fn build_human_tree(
    entries: Vec<String>,
    path_map: HashMap<String, PathBuf>,
) -> FolderTreeNode {
    // Builds a tree structure from the provided entries and path map.
    println!("[DEBUG] build_human_tree: Start");
    let mut root = FolderTreeNode::default();

    for (uuid, original_path) in path_map {
        // For each UUID and its corresponding path, process the entry.
        println!("[DEBUG] Processing UUID: {uuid}, Path: {original_path:?}");

        let parent_label = original_path
            // Get the parent directory of the original path.
            .parent()
            .unwrap_or(&original_path)
            .display()
            .to_string();
        let item_name = original_path
            // Get the file name from the original path.
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        println!("[DEBUG] parent_label = \"{parent_label}\", item_name = \"{item_name}\"");

        let parent_node = root
            // Ensure the parent node exists in the tree.
            .children
            .entry(parent_label.clone())
            .or_insert_with(FolderTreeNode::default);

        parent_node
            // Ensure the item node exists in the parent node's children.
            .children
            .entry(item_name.clone())
            .or_insert_with(FolderTreeNode::default);

        let dir_prefix = format!("{uuid}/"); // Create a prefix for directory entries based on the UUID.
        let is_dir_backup = entries.iter().any(|e| e.starts_with(&dir_prefix)); // Check if there are any entries that start with the UUID prefix.

        if is_dir_backup {
            // If there are entries that start with the UUID prefix, treat it as a directory
            // backup.
            println!("[DEBUG] Detected directory backup for UUID: {uuid}");
            parent_node.children.get_mut(&item_name).unwrap().is_file = false;

            for tar_path in entries.iter().filter(|e| e.starts_with(&dir_prefix)) {
                // For each entry that starts with the UUID prefix, process it as a directory path.
                println!("[DEBUG]   tar_path = \"{tar_path}\"");

                let rest = tar_path[dir_prefix.len()..].trim_end_matches('/');
                // Remove the UUID prefix and any trailing slashes.
                if rest.is_empty() {
                    println!("[DEBUG]   Skipping empty rest after trim");
                    continue;
                }

                println!("[DEBUG]   Rest path: \"{rest}\"");

                let mut cursor = parent_node.children.get_mut(&item_name).unwrap(); // Get the item
                // node for the current UUID.
                for part in rest.split('/') {
                    // Split the rest path into parts and traverse the tree.
                    println!("[DEBUG]     Descending into part: \"{part}\"");
                    cursor = cursor
                        // Ensure the cursor is mutable to modify the tree.
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

// Recursively collects paths of checked files from the folder tree.
pub fn collect_recursive(node: &FolderTreeNode, path: &mut Vec<String>, output: &mut Vec<String>) {
    for (name, child) in &node.children {
        path.push(name.clone());
        if child.is_file && child.checked {
            // If the node is a file and checked, add its path to the output.
            let full_path = path.join("/");
            println!("[DEBUG] collect_recursive: Adding checked file {full_path}");
            output.push(full_path);
        }

        collect_recursive(child, path, output);
        path.pop();
    }
}

// Collects all paths of checked files from the folder tree into a vector.
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

// Parses a fingerprint file from a ZIP archive and returns a list of entries and a mapping of
// UUIDs to paths.
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

// Retrieves the embedded fingerprint from the environment variable or returns a default value.
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

// This function checks if the original path starts with "C:\Users\" and adjusts it to the current
// user's home directory.
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

            // Check if the original path starts with the expected prefix.
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

// This function checks if the given path exists, and if not, it attempts to adjust it based on the
// current user's home directory. If the adjusted path exists, it returns that; otherwise, it
// returns None.
pub fn fix_skip(p: &Path) -> Option<PathBuf> {
    println!("[DEBUG] fix_skip: Checking path {}", p.display());

    // If the original path exists, return it as-is.
    if p.exists() {
        println!("[DEBUG] -> Path exists, using as-is");
        return Some(p.to_path_buf());
    }

    let current_home = dirs::home_dir()?; // Get the current user's home directory.
    let adjusted = adjust_path(p, &current_home); // Adjust the path based on the current home directory.

    // If the adjusted path exists, return it; otherwise, return None.
    if adjusted.exists() {
        println!(
            "[DEBUG] -> Adjusted path exists: using {}",
            adjusted.display()
        );
        Some(adjusted)
    } else {
        // If neither the original nor the adjusted path exists, log a debug message and return
        // none.
        println!(
            "[DEBUG] -> Neither original nor adjusted path exists ({} -> {})",
            p.display(),
            adjusted.display()
        );
        None
    }
}
