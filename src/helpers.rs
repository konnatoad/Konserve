use std::path::PathBuf;
use std::env;

pub fn get_fingered() -> String {
    env::var("FINGERPRINT").unwrap_or_else(|_| "DEFAULT_FINGERPRINT".into())
}

pub fn adjust_path(original: &PathBuf, current_home: &PathBuf) -> PathBuf {
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

    original.clone()
}

pub fn fix_skip(p: &PathBuf) -> Option<PathBuf> {
    if p.exists() {
        return Some(p.clone());
    }

    let current_home = dirs::home_dir()?;
    let adjusted = adjust_path(p, &current_home);

    if adjusted.exists() {
        Some(adjusted)
    } else {
        None
    }
}
