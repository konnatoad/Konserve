# Konserve

A simple GUI backup utility for Windows, designed to help you **back up** important folders or files and **restore them** later—even on a different machine or user account.

> ✔ Portable backups with timestamped TARs  
> ✔ Embedded build-time fingerprint  
> ✔ Template support  
> ✔ Partial restore with visual tree selection & progress   
> ✔ Smart path correction across Windows users   

---

## Features

-  **Select multiple folders and files** to include in a backup
-  **Create timestamped `.tar` archives** with embedded path data
-  **Embedded fingerprint** (via build script) in every backup for traceability
-  **Restore entire backups** or selectively restore individual items
-  **Preview and toggle restore items** using a collapsible folder tree
-  **Progress bars & spinners** show pack/unpack progress in real time
-  **Save/Load/Edit templates** (`.json`) to re-use backup selections
-  **Auto-adjust file paths** when restoring to a new user account
-  **Friendly GUI** built with [egui](https://github.com/emilk/egui)

---

## UI Preview

| Backup | Template | Restore |
|--------|---------|---------|
| ![image](https://github.com/user-attachments/assets/778d4407-439c-43df-9857-df10717fcd6d) | ![template](https://github.com/user-attachments/assets/88896958-6b62-453f-973a-81744626b53d) | ![image](https://github.com/user-attachments/assets/6315f889-d01c-450d-a36c-fafbe47e1f6e) |


---

## Backup Format

Each backup:
- Is saved as a compressed `.tar` file:
  ```
  backup_YYYY-MM-DD_HH-MM-SS.tar
  ```
- Includes a `fingerprint.txt` file with:
  - A unique fingerprint ID (configured via `.env` or embedded at build time)
  - The original source paths for every backed-up file/folder

---

## Template System

Save your folder/file selection as a `.json` template to:
- Quickly re-select common paths for future backups
- Manually edit or review the list in-app
- Load templates even when some paths are missing (skips safely)

---

## Path Correction

Restoring to a different user account? No problem.
The tool auto-adjusts Windows-style paths like:

```
C:\Users\Alice\Documents → C:\Users\Bob\Documents
```

This allows seamless migration between machines or profiles.

---

## Build & Release

### Prerequisites

- Rust (2021+ toolchain): [Install Rust](https://rustup.rs)
- A Windows machine (or cross-compile target)
- (Optional) `.env` file with a custom fingerprint:
  ```env
  FINGERPRINT=MyMachine123
  ```

### Build

```bash
cargo build --release
```

Your executable will be:

```
target/release/Konserve.exe
```

---
