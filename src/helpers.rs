use std::{
    collections::HashMap,
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use tar::Archive;

use crate::FolderTreeNode;

pub fn collect_recursive(node: &FolderTreeNode, path: &mut Vec<String>, output: &mut Vec<String>) {
    for (name, child) in &node.children {
        path.push(name.clone());
        if child.is_file && child.checked {
            output.push(path.join("/"));
        }

        collect_recursive(child, path, output);
        path.pop();
    }
}

pub fn collect_paths(root: &FolderTreeNode) -> Vec<String> {
    let mut result = Vec::new();
    let mut path = Vec::new();
    collect_recursive(root, &mut path, &mut result);
    result
}

pub fn parse_fingerprint(
    zip_path: &PathBuf,
) -> Result<(Vec<String>, HashMap<String, PathBuf>), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = Archive::new(file);
    let mut path_map = HashMap::new();

    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let header_path = entry.path().map_err(|e| e.to_string())?;
        let name = header_path.to_string_lossy();
        if name == "fingerprint.txt" {
            let mut txt = String::new();
            entry.read_to_string(&mut txt).map_err(|e| e.to_string())?;
            for line in txt.lines().filter(|l| l.contains(": ")) {
                let (uuid, p) = line.split_once(": ").unwrap();
                path_map.insert(uuid.to_string(), PathBuf::from(p.trim()));
            }
            break;
        }
    }

    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = Archive::new(file);
    let mut entries = Vec::new();

    for entry in archive.entries().map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let entry_path = entry.path().map_err(|e| e.to_string())?;
        let entry_name = entry_path.to_string_lossy().into_owned();
        if entry_name != "fingerprint.txt" {
            entries.push(entry_name);
        }
    }

    Ok((entries, path_map))
}

pub fn get_fingered() -> String {
    env::var("FINGERPRINT").unwrap_or_else(|_| "DEFAULT_FINGERPRINT".into())
}

pub fn adjust_path(original: &Path, current_home: &Path) -> PathBuf {
    let og_str = original.to_string_lossy();
    let current_str = current_home.to_string_lossy();

    if og_str.to_lowercase().starts_with("c:\\users\\") {
        let parts: Vec<&str> = og_str.split('\\').collect();
        if parts.len() > 2 {
            let old_username = parts[2];
            let expected_prefix = format!("C:\\Users\\{}", old_username);

            if og_str.starts_with(&expected_prefix) {
                let rel_path = og_str.strip_prefix(&expected_prefix).unwrap_or("");
                return PathBuf::from(format!("{}{}", current_str, rel_path));
            }
        }
    }

    original.to_path_buf()
}

pub fn fix_skip(p: &Path) -> Option<PathBuf> {
    if p.exists() {
        return Some(p.to_path_buf());
    }

    let current_home = dirs::home_dir()?;
    let adjusted = adjust_path(p, &current_home);

    if adjusted.exists() {
        Some(adjusted)
    } else {
        None
    }
}
