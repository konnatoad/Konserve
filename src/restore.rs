use crate::helpers::{adjust_path, get_fingered};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tar::Archive;

pub fn restore_backup(
    zip_path: &PathBuf,
    selected: Option<Vec<String>>,
    status: Arc<Mutex<String>>,
) -> Result<(), String> {
    *status.lock().unwrap() = "Restoring backup…".into();
    let mut archive = Archive::new(File::open(zip_path).map_err(|e| e.to_string())?);
    let mut path_map: HashMap<String, PathBuf> = HashMap::new();
    let mut valid = false;

    for entry_res in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry_res.map_err(|e| e.to_string())?;
        let header_path = entry.path().map_err(|e| e.to_string())?;
        let name = header_path.to_string_lossy();
        if name == "fingerprint.txt" {
            let mut txt = String::new();
            entry.read_to_string(&mut txt).map_err(|e| e.to_string())?;
            if txt.contains(&get_fingered()) {
                valid = true;
                for line in txt.lines().filter(|l| l.contains(": ")) {
                    let (uuid, p) = line.split_once(": ").unwrap();
                    path_map.insert(uuid.to_string(), PathBuf::from(p.trim()));
                }
            }
            break;
        }
    }
    if !valid {
        return Err("Invalid backup fingerprint.".into());
    }

    let mut to_extract = HashSet::new();
    if let Some(human_sel) = &selected {
        for (uuid, orig) in &path_map {
            let parent = orig.parent().unwrap_or(orig);
            let folder = orig.file_name().unwrap().to_string_lossy();
            let base = format!("{}/{}", parent.display(), folder);

            if human_sel.contains(&base) {
                to_extract.insert(uuid.clone());
            }
            for h in human_sel {
                if let Some(rest) = h.strip_prefix(&(base.clone() + "/")) {
                    to_extract.insert(format!("{}/{}", uuid, rest));
                }
            }
        }
    }

    let mut archive = Archive::new(File::open(zip_path).map_err(|e| e.to_string())?);
    let current_home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("C:\\"));
    for entry_res in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry_res.map_err(|e| e.to_string())?;
        let path_in_tar = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();

        if path_in_tar == "fingerprint.txt" {
            continue;
        }

        if selected.is_some() && !to_extract.contains(&path_in_tar) {
            continue;
        }

        let tar_path = Path::new(&path_in_tar);
        let root_component = tar_path
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy();
        if let Some(orig_base) = path_map.get(&root_component.to_string()) {
            let adjusted = adjust_path(orig_base, &current_home);
            let rel = tar_path
                .strip_prefix(Path::new(&root_component as &str))
                .unwrap_or_else(|_| Path::new(""));
            let unpack_to = adjusted.join(rel);
            if let Some(dir) = unpack_to.parent() {
                fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            entry.unpack(&unpack_to).map_err(|e| e.to_string())?;
        }
    }

    *status.lock().unwrap() = "✅ Restore complete.".into();
    Ok(())
}
