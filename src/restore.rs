use crate::helpers::{ adjust_path, get_fingered };

use std::{ collections::HashMap, fs::{ self, File }, io::{ self, Read }, path::{ Path, PathBuf } };
use zip::ZipArchive;

pub fn restore_backup(zip_path: &PathBuf, selected: Option<Vec<String>>) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;

    let mut path_map = HashMap::new();
    let mut valid = false;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        if file.name() == "fingerprint.txt" {
            let mut contents = String::new();
            file.read_to_string(&mut contents).unwrap();

            if contents.contains(&get_fingered()) {
                valid = true;
                for line in contents.lines() {
                    if let Some((key, path)) = line.split_once(": ") {
                        let full_path = PathBuf::from(path);
                        path_map.insert(key.trim().to_string(), full_path);
                    }
                }
            }
            break;
        }
    }

    if !valid {
        return Err("Invalid backup fingerprint.".into());
    }

    let current_user_home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("C:\\"));

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let name_in_zip = file.name();

        if name_in_zip == "fingerprint.txt" {
            continue;
        }

        if let Some(ref selected) = selected {
            let normalized = name_in_zip.replace('\\', "/");
            if !selected.iter().any(|s| s == &normalized) {
                continue;
            }
        }

        let zip_path = Path::new(name_in_zip);

        if zip_path.components().count() == 1 {
            if let Some(original_path) = path_map.get(name_in_zip) {
                let adjusted_path = adjust_path(original_path, &current_user_home);
                if let Some(parent) = adjusted_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut out = File::create(adjusted_path).map_err(|e| e.to_string())?;
                io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
            }
            continue;
        }

        if let Some(root_component) = zip_path.components().next() {
            let root_name = root_component.as_os_str().to_string_lossy().to_string();
            if let Some(base_original_path) = path_map.get(&root_name) {
                let adjusted_base = adjust_path(base_original_path, &current_user_home);
                let relative_path = zip_path
                    .strip_prefix(&root_name)
                    .unwrap_or_else(|_| Path::new(""));
                let full_path = adjusted_base.join(relative_path);

                if file.name().ends_with('/') {
                    fs::create_dir_all(&full_path).map_err(|e| e.to_string())?;
                } else {
                    if let Some(parent) = full_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    let mut out = File::create(&full_path).map_err(|e| e.to_string())?;
                    io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    Ok(())
}
