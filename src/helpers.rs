use std::{
    env,
    fs::File,
    path::{Path, PathBuf},
};
use zip::ZipArchive;

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

pub fn parse_fingerprint(zip_path: &PathBuf) -> Result<Vec<String>, String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;

    let mut entries = Vec::new();

    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name();

        //skip metadata
        if name != "fingerprint.txt" {
            entries.push(name.to_string());
        }
    }

    Ok(entries)
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
