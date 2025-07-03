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

use crate::FolderTreeNode;

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

// if !icon then fuck you
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

pub fn render_tree(ui: &mut egui::Ui, path: &mut Vec<String>, node: &mut FolderTreeNode) {
    for (name, child) in node.children.iter_mut() {
        let mut label = name.clone();
        if !child.is_file {
            label.push('/');
        }

        path.push(name.clone());
        let current_path = path.join("/");

        if child.children.is_empty() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut child.checked, "");
                ui.label(label);
            });
        } else {
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
                        render_tree(ui, path, child);
                    });
            });
            child.checked = child.children.values().any(|c| c.checked);
        }

        path.pop();
    }
}

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

        let dir_prefix = format!("{uuid}/");
        let is_dir_backup = entries.iter().any(|e| e.starts_with(&dir_prefix));

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

                let mut cursor = parent_node.children.get_mut(&item_name).unwrap();

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

pub fn get_fingered() -> &'static str {
    const DEFAULT: &str = "DEFAULT_FINGERPRINT";

    match option_env!("FINGERPRINT") {
        Some(val) => {
            println!("get_fingered: using embedded fingerprint = \"{val}\"");
            val
        }
        None => {
            println!(
                "get_fingered: no embedded fingerprint found, fallback \"{DEFAULT}\""
            );
            DEFAULT
        }
    }
}

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

pub fn fix_skip(p: &Path) -> Option<PathBuf> {
    println!("[DEBUG] fix_skip: Checking path {}", p.display());

    if p.exists() {
        println!("[DEBUG] -> Path exists, using as-is");
        return Some(p.to_path_buf());
    }

    let current_home = dirs::home_dir()?;
    let adjusted = adjust_path(p, &current_home);

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
