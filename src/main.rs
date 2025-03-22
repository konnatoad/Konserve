use std::fs::File;
use std::io::{ self, Write };
use zip::write::FileOptions;
use std::path::Path;
use chrono::Local;
use walkdir::WalkDir;
use std::collections::HashMap;

fn main() {
    loop {
        println!("\n=== vanmanen ===");
        println!("1. Create Temp Backup");
        println!("2. Restore Backup");
        println!("3. Exit");
        println!("Enter your choice: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            "1" => create_temp_backup(),
            "2" => restore_backup(),
            "3" => {
                println!("get cancer");
                break;
            }
            _ => println!("Invalid choice. Please try again."),
        }
    }
}

fn create_temp_backup() {
    println!("Enter full path of folders to backup");
    println!("Press Enter on an empty line when you're done");

    let mut folders = Vec::new();
    loop {
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let trimmed = input.trim();

        if trimmed.is_empty() {
            break;
        }

        if Path::new(trimmed).exists() {
            folders.push(trimmed.to_string());
        } else {
            println!("'{}' does not exist. Skipping", trimmed);
        }

        if folders.is_empty() {
            println!("No folders to backup. Exiting");
            return;
        }
    }

    println!("Enter output folder (leave empty for default)");
    let mut output_input = String::new();
    io::stdin().read_line(&mut output_input).unwrap();
    let output_dir = output_input.trim();

    let output_path = if output_dir.is_empty() {
        std::env::current_dir().unwrap()
    } else {
        let path = Path::new(output_dir);
        if !path.exists() || !path.is_dir() {
            println!("'{}' is not a valid directory. Using default", output_dir);
            std::env::current_dir().unwrap()
        } else {
            path.to_path_buf()
        }
    };

    let timestamp = Local::now().format("%Y-%m-%d_%H-%M-%S");
    let zip_name = format!("backup_{}.zip", timestamp);
    let zip_path = output_path.join(zip_name);
    let file = match File::create(&zip_path) {
        Ok(f) => f,
        Err(e) => {
            println!("Error creating zip file: {}", e);
            return;
        }
    };

    let mut zip = zip::ZipWriter::new(file);
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("fingerprint.txt", options).unwrap();

    let mut fingerprint = String::from("pillupaa\n[Backup Info]\n");

    for (i, folder) in folders.iter().enumerate() {
        fingerprint.push_str(&format!("Folder {}: {}\n", i + 1, folder));
    }
    zip.write_all(fingerprint.as_bytes()).unwrap();

    for folder in &folders {
        let base_path = Path::new(folder);
        let walkdir = WalkDir::new(base_path);

        for entry in walkdir.into_iter().filter_map(Result::ok) {
            let path = entry.path();
            let relative = match path.strip_prefix(base_path) {
                Ok(p) => p,
                Err(_) => {
                    continue;
                }
            };

            let zip_path = Path::new(folder).file_name().unwrap().to_string_lossy().to_string();
            let final_path = Path::new(&zip_path).join(relative);

            if path.is_file() {
                zip.start_file(final_path.to_string_lossy(), options).unwrap();
                let mut f = File::open(path).unwrap();
                io::copy(&mut f, &mut zip).unwrap();
            } else if !relative.as_os_str().is_empty() {
                zip.add_directory(final_path.to_string_lossy(), options).unwrap();
            }
        }
    }
    zip.finish().unwrap();
    println!("Backup complete");
}

fn restore_backup() {
    println!("Enter path to the folder containing your backups:");

    let mut folder_input = String::new();
    io::stdin().read_line(&mut folder_input).unwrap();
    let backup_dir = folder_input.trim();

    let backup_path = Path::new(backup_dir);
    if !backup_path.exists() || !backup_path.is_dir() {
        println!("Backup folder not found");
        return;
    }

    let entries = match std::fs::read_dir(backup_path) {
        Ok(e) => e,
        Err(_) => {
            println!("Error reading backup folder");
            return;
        }
    };

    let zip_files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.path().is_file() &&
                e
                    .path()
                    .extension()
                    .map_or(false, |ext| ext == "zip")
        })
        .collect();

    if zip_files.is_empty() {
        println!("No zip files found in backup folder");
        return;
    }

    println!("\nFound the following backup files:");
    for (i, file) in zip_files.iter().enumerate() {
        let name = file.file_name();
        let name_str = name.to_string_lossy();
        println!("[{}] {}", i + 1, name_str);
    }

    println!("Enter the number of the backup you  want to restore:");
    let mut choice = String::new();
    io::stdin().read_line(&mut choice).unwrap();

    let selected = match choice.trim().parse::<usize>() {
        Ok(num) if num > 0 && num <= zip_files.len() => num - 1,
        _ => {
            println!("Invalid choice");
            return;
        }
    };

    let zip_path = zip_files[selected].path();

    let file = match File::open(&zip_path) {
        Ok(f) => f,
        Err(e) => {
            println!("Error opening zip file: {}", e);
            return;
        }
    };

    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            println!("Error opening zip archive: {}", e);
            return;
        }
    };

    let mut folder_map: HashMap<String, String> = HashMap::new();
    let mut is_valid = false;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        if file.name() == "fingerprint.txt" {
            let mut contents = String::new();
            use std::io::Read;
            file.read_to_string(&mut contents).unwrap();

            if contents.contains("pillupaa") {
                is_valid = true;
                for line in contents.lines() {
                    if line.starts_with("Folder ") {
                        if let Some((_, path)) = line.split_once(": ") {
                            if let Some(folder_name) = Path::new(path).file_name() {
                                folder_map.insert(
                                    folder_name.to_string_lossy().to_string(),
                                    path.to_string()
                                );
                            }
                        }
                    }
                }
            }
            break;
        }
    }

    if !is_valid {
        println!("No valid fingerprint found in zip archive.");
        return;
    }

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let internal_path = Path::new(file.name());

        if file.name() == "fingerprint.txt" {
            continue;
        }

        let zip_root_folder = internal_path
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .to_string();

        let base_restore_path = match folder_map.get(&zip_root_folder) {
            Some(p) => Path::new(p),
            None => {
                println!("Unknown folder in zip: {}", file.name());
                continue;
            }
        };

        let relative_path = internal_path.strip_prefix(&zip_root_folder).unwrap();
        let full_path = base_restore_path.join(relative_path);

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&full_path).unwrap();
        } else {
            if let Some(parent) = full_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent).unwrap();
                }
            }

            let mut outfile = File::create(&full_path).unwrap();
            std::io::copy(&mut file, &mut outfile).unwrap();
        }
    }

    println!("cancer recieved");
}
