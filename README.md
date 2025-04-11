#  VanManen Backup Tool

A simple GUI backup utility for Windows, designed to help you **back up** important folders or files and **restore them** later—even on a different machine or user account.

> ✔ Portable backups with timestamped ZIPs  
> ✔ Template support  
> ✔ Partial restore with visual tree selection  
> ✔ Smart path correction across Windows users  

---

##  Features

-  **Select multiple folders and files** to include in a backup
-  **Create timestamped `.zip` archives** with embedded path data
-  **Restore entire backups** or selectively restore individual items
-  **Preview and toggle restore items** using a collapsible folder tree
-  **Save/Load/Edit templates** (`.json`) to re-use backup selections
-  **Auto-adjust file paths** when restoring to a new user account
-  **Friendly GUI** built with [egui](https://github.com/emilk/egui)

---

##  UI Preview

| Backup | Template |
|--------|---------|
| ![backup](https://github.com/user-attachments/assets/6bf1736b-7e66-4b5b-9af5-b179da536860) | ![template](https://github.com/user-attachments/assets/88896958-6b62-453f-973a-81744626b53d) |

---

##  Backup Format

Each backup:
- Is saved as a compressed `.zip` file: `backup_YYYY-MM-DD_HH-MM-SS.zip`
- Includes a `fingerprint.txt` file with:
  - A unique fingerprint ID (optional, via `.env`)
  - The original source paths for every backed up file/folder

---

##  Template System

Save your folder/file selection as a `.json` file to:
- Quickly re-select common paths for future backups
- Manually edit or review the list in-app
- Load templates even when some paths are missing (skips safely)

---

##  Path Correction

Restoring to a different user account? No problem.  
The tool auto-adjusts Windows-style paths like:

C:\Users\Alice\Documents → C:\Users\Bob\Documents


This allows seamless migration between machines or profiles.

---

##  Build Instructions

###  Prerequisites
- Rust (2021+ toolchain): [Install Rust](https://rustup.rs)
- A Windows machine (or cross-compiling for Windows)
- (Optional) `.env` file with a custom fingerprint:
  ```env
  FINGERPRINT=MyMachine123
