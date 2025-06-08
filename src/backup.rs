use crate::helpers::{Progress, get_fingered};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

use chrono::Local;
use serde::{Deserialize, Serialize};
use tar::{Builder, Header};
use uuid::Uuid;
use walkdir::WalkDir;

#[derive(Serialize, Deserialize)]
struct BackupTemplate {
    paths: Vec<PathBuf>,
}

pub fn backup_gui(
    folders: &[PathBuf],
    output_dir: &Path,
    progress: &Progress,
) -> Result<PathBuf, String> {
    println!("[DEBUG] backup_gui: Started");
    println!("[DEBUG] Output directory: {}", output_dir.display());

    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let zip_name = format!("backup_{}.tar", timestamp);
    let zip_path = output_dir.join(&zip_name);
    println!("[DEBUG] Creating backup archive: {}", zip_path.display());

    let tar_file = File::create(&zip_path).map_err(|e| e.to_string())?;
    let mut tar_builder = Builder::new(tar_file);

    let mut fingerprint_content = format!("{}\n[Backup Info]\n", get_fingered());

    // folders to uuid
    let folder_uuid: Vec<(Uuid, &PathBuf)> = folders
        .iter()
        .map(|folder| {
            let uuid = Uuid::new_v4();
            println!("[DEBUG] Assigned UUID {} to {}", uuid, folder.display());
            (uuid, folder)
        })
        .collect();

    let total_files: u32 = folders
        .iter()
        .flat_map(|p| WalkDir::new(p).into_iter().filter_map(Result::ok))
        .filter(|e| e.file_type().is_file())
        .count()
        .max(1) as u32;

    let mut done = 0u32;

    // generate fingerprint content
    for (uuid, original_path) in &folder_uuid {
        fingerprint_content.push_str(&format!("{}: {}\n", uuid, original_path.display()));
    }

    // write fingerprint.txt
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
    println!("[DEBUG] fingerprint.txt added to archive");

    for (uuid, original_path) in folder_uuid {
        if original_path.is_file() {
            println!("[DEBUG] Adding single file: {}", original_path.display());

            let metadata = original_path.metadata().map_err(|e| e.to_string())?;
            let mut header = Header::new_gnu();
            header.set_metadata(&metadata);
            header.set_cksum();

            let mut f = File::open(original_path).map_err(|e| e.to_string())?;

            let entry_name = match original_path.extension().and_then(|e| e.to_str()) {
                Some(ext) => format!("{}.{}", uuid, ext),
                None => uuid.to_string(),
            };
            println!("[DEBUG] -> Entry name in tar: {}", entry_name);

            tar_builder
                .append_data(&mut header, entry_name, &mut f)
                .map_err(|e| e.to_string())?;

            done += 1;
            progress.set(done * 100 / total_files);

            continue;
        }

        println!("[DEBUG] Walking folder: {}", original_path.display());

        for entry in WalkDir::new(original_path)
            .into_iter()
            .filter_map(Result::ok)
        {
            let entry_path = entry.path();
            let metadata = entry.metadata().map_err(|e| e.to_string())?;
            let relative_path = entry_path.strip_prefix(original_path).unwrap();
            let tar_entry_path = Path::new(&uuid.to_string()).join(relative_path);

            let mut header = Header::new_gnu();
            header.set_metadata(&metadata);
            header.set_cksum();

            if metadata.is_file() {
                println!("[DEBUG] Adding file: {}", entry_path.display());
                let mut file = File::open(entry_path).map_err(|e| e.to_string())?;
                tar_builder
                    .append_data(&mut header, tar_entry_path, &mut file)
                    .map_err(|e| e.to_string())?;

                done += 1;
                progress.set(done * 100 / total_files);
            } else if metadata.is_dir() {
                println!("[DEBUG] Adding directory: {}", entry_path.display());
                tar_builder
                    .append_data(&mut header, tar_entry_path, io::empty())
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    tar_builder.finish().map_err(|e| e.to_string())?;
    println!("[DEBUG] Archive finished: {}", zip_path.display());

    progress.done();

    Ok(zip_path)
}

// let file = File::create(&zip_path).map_err(|e| e.to_string())?;
// let mut zip = ZipWriter::new(file);
// let options: FileOptions<'_, ()> = FileOptions::default().compression_method(
//     CompressionMethod::Deflated
// );

// zip.start_file("fingerprint.txt", options).unwrap();
// let mut fingerprint = format!("{}\n[Backup Info]\n", get_fingered());
// for folder in folders {
//     if let Some(name) = folder.file_name() {
//         fingerprint.push_str(&format!(
//             "{}: {}\n",
//             name.to_string_lossy(),
//             folder.display()
//         ));
//     }
// }

// zip.write_all(fingerprint.as_bytes()).unwrap();

// for path in folders {
//     if path.is_file() {
//         let filename = path.file_name().unwrap().to_string_lossy();
//         zip.start_file(filename, options).unwrap();
//         let mut f = File::open(path).unwrap();
//         io::copy(&mut f, &mut zip).unwrap();
//     } else if path.is_dir() {
//         for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
//             let entry_path = entry.path();
//             let relative = match entry_path.strip_prefix(path) {
//                 Ok(r) => r,
//                 Err(_) => {
//                     continue;
//                 }
//             };

//             let zip_folder = path.file_name().unwrap();
//             let final_path = Path::new(zip_folder).join(relative);

//             if entry_path.is_file() {
//                 zip.start_file(final_path.to_string_lossy(), options)
//                     .unwrap();
//                 let mut f = File::open(entry_path).unwrap();
//                 io::copy(&mut f, &mut zip).unwrap();
//             } else if !relative.as_os_str().is_empty() {
//                 zip.add_directory(final_path.to_string_lossy(), options)
//                     .unwrap();
//             }
//         }
//     }
//     }

//     zip.finish().unwrap();
//     Ok(zip_path)
// }
