use crate::helpers::get_fingered;
use std::{ fs::File, io::{ self, Write }, path::{ Path, PathBuf } };

use chrono::Local;
use serde::{ Deserialize, Serialize };
use walkdir::WalkDir;
use zip::{ CompressionMethod, ZipWriter, write::FileOptions };

#[derive(Serialize, Deserialize)]
struct BackupTemplate {
    paths: Vec<PathBuf>,
}

pub fn backup_gui(folders: &[PathBuf], output_dir: &Path) -> Result<PathBuf, String> {
    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let zip_name = format!("backup_{}.zip", timestamp);
    let zip_path = output_dir.join(&zip_name);

    let file = File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    let options: FileOptions<'_, ()> = FileOptions::default().compression_method(
        CompressionMethod::Deflated
    );

    zip.start_file("fingerprint.txt", options).unwrap();
    let mut fingerprint = format!("{}\n[Backup Info]\n", get_fingered());
    for folder in folders {
        if let Some(name) = folder.file_name() {
            fingerprint.push_str(&format!("{}: {}\n", name.to_string_lossy(), folder.display()));
        }
    }

    zip.write_all(fingerprint.as_bytes()).unwrap();

    for path in folders {
        if path.is_file() {
            let filename = path.file_name().unwrap().to_string_lossy();
            zip.start_file(filename, options).unwrap();
            let mut f = File::open(path).unwrap();
            io::copy(&mut f, &mut zip).unwrap();
        } else if path.is_dir() {
            for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
                let entry_path = entry.path();
                let relative = match entry_path.strip_prefix(path) {
                    Ok(r) => r,
                    Err(_) => {
                        continue;
                    }
                };

                let zip_folder = path.file_name().unwrap();
                let final_path = Path::new(zip_folder).join(relative);

                if entry_path.is_file() {
                    zip.start_file(final_path.to_string_lossy(), options).unwrap();
                    let mut f = File::open(entry_path).unwrap();
                    io::copy(&mut f, &mut zip).unwrap();
                } else if !relative.as_os_str().is_empty() {
                    zip.add_directory(final_path.to_string_lossy(), options).unwrap();
                }
            }
        }
    }

    zip.finish().unwrap();
    Ok(zip_path)
}
