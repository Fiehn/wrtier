//! Vaults' folders, for Manage Vaults (v3 plan, Vault management): the folder a new Vault gets,
//! what's in a folder before it's made a Vault, and moving one. Syncthing's part of a move is in
//! `syncthing`; the Vault name is in `vault/name.rs` (ADR 0022).

use std::path::{Path, PathBuf};

/// A folder name for a new Vault called `name`: the characters Windows refuses in a folder's name
/// replaced, so the Vault name can stay as it was typed.
pub fn folder_name_for(name: &str) -> String {
    let mut folder: String = name.trim().chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '-' } else { c })
        .collect();
    // Windows drops dots and spaces at the end of a folder's name.
    while folder.ends_with(['.', ' ']) {
        folder.pop();
    }
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    let stem = folder.split('.').next().unwrap_or("").to_ascii_uppercase();
    if reserved.contains(&stem.as_str()) {
        folder.push('_');
    }
    if folder.is_empty() { "Vault".into() } else { folder }
}

/// Where a new Vault called `name` goes in `location`. Refused if something's there already, so
/// a new Vault never mixes into another folder.
pub fn new_vault_folder(location: &Path, name: &str) -> Result<PathBuf, String> {
    if !location.is_dir() {
        return Err(format!("{} isn't a folder.", location.display()));
    }
    let folder = location.join(folder_name_for(name));
    if folder.exists() {
        return Err(format!("There's already a folder called “{}” there. Choose another name or place.", folder_name_for(name)));
    }
    Ok(folder)
}

/// How many files a folder holds (all the way down), to say before making it a Vault. Counted up
/// to `limit`: past that, "more than".
pub fn count_files(folder: &Path, limit: usize) -> usize {
    let mut count = 0;
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(entry.path()),
                Ok(_) => count += 1,
                Err(_) => {}
            }
            if count >= limit {
                return count;
            }
        }
    }
    count
}

/// Where a Vault's folder goes when moved into `parent`: under its own name. Refused into itself,
/// or where something's there already.
pub fn moved_to(folder: &Path, parent: &Path) -> Result<PathBuf, String> {
    let name = folder.file_name().ok_or("This Vault's folder has no name to move it by.")?;
    if !parent.is_dir() {
        return Err(format!("{} isn't a folder.", parent.display()));
    }
    let same = |a: &Path| std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    if same(parent).starts_with(same(folder)) {
        return Err("A Vault can't be moved into itself.".into());
    }
    let to = parent.join(name);
    if to.exists() {
        return Err(format!("There's already a folder called “{}” in {}.", name.to_string_lossy(), parent.display()));
    }
    Ok(to)
}

/// Moves a folder. On the same drive it's a rename; to another drive it's copied, the copy is
/// checked file by file, and only then is the original deleted, so a move cut short loses nothing.
pub fn move_folder(from: &Path, to: &Path) -> Result<(), String> {
    match std::fs::rename(from, to) {
        Ok(()) => return Ok(()),
        Err(e) if !other_drive(&e) => return Err(format!("The folder couldn't be moved: {e}")),
        Err(_) => {}
    }
    if let Err(e) = copy_checked(from, to) {
        let _ = std::fs::remove_dir_all(to); // the part copied: the original is still whole
        return Err(format!("The folder couldn't be copied to the other drive, so it wasn't moved: {e}"));
    }
    std::fs::remove_dir_all(from).map_err(|e| format!(
        "The Vault was copied to {}, but its old folder couldn't be deleted ({e}). Delete {} yourself once you've checked the copy.",
        to.display(), from.display(),
    ))
}

/// A rename refused because the two places are on different drives.
fn other_drive(e: &std::io::Error) -> bool {
    // EXDEV on Linux and Android, ERROR_NOT_SAME_DEVICE on Windows.
    e.kind() == std::io::ErrorKind::CrossesDevices || matches!(e.raw_os_error(), Some(18) if cfg!(unix)) || matches!(e.raw_os_error(), Some(17) if cfg!(windows))
}

/// Copies a folder, then checks every file arrived whole (the same size).
fn copy_checked(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    let mut stack = vec![PathBuf::new()];
    std::fs::create_dir(to)?;
    while let Some(relative) = stack.pop() {
        for entry in std::fs::read_dir(from.join(&relative))? {
            let entry = entry?;
            let path = relative.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                std::fs::create_dir(to.join(&path))?;
                stack.push(path);
            } else {
                let size = std::fs::copy(entry.path(), to.join(&path))?;
                files.push((path, size));
            }
        }
    }
    for (path, size) in files {
        let original = std::fs::metadata(from.join(&path))?.len();
        if size != original || std::fs::metadata(to.join(&path))?.len() != original {
            return Err(std::io::Error::other(format!("{} didn't copy whole", path.display())));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_vaults_folder_is_named_after_it_with_what_windows_refuses_replaced() {
        assert_eq!(folder_name_for("Short stories"), "Short stories");
        assert_eq!(folder_name_for("Draft: part 1/2?"), "Draft- part 1-2-");
        assert_eq!(folder_name_for("The end..."), "The end");
        assert_eq!(folder_name_for("con"), "con_");
        assert_eq!(folder_name_for("???"), "---");
        assert_eq!(folder_name_for("   "), "Vault");
    }

    #[test]
    fn a_new_vault_never_goes_into_a_folder_that_is_there_already() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Writing")).unwrap();
        assert!(new_vault_folder(dir.path(), "Writing").is_err());
        assert_eq!(new_vault_folder(dir.path(), "Stories").unwrap(), dir.path().join("Stories"));
    }

    #[test]
    fn counts_the_files_in_a_folder_all_the_way_down() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("b.txt"), "b").unwrap();
        assert_eq!(count_files(dir.path(), 100), 2);
        assert_eq!(count_files(dir.path(), 1), 1);
    }

    #[test]
    fn a_vault_moves_under_its_own_name_but_never_into_itself() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("Writing");
        std::fs::create_dir_all(vault.join("inside")).unwrap();
        let other = dir.path().join("Other");
        std::fs::create_dir(&other).unwrap();
        assert!(moved_to(&vault, &vault.join("inside")).is_err());
        let to = moved_to(&vault, &other).unwrap();
        assert_eq!(to, other.join("Writing"));
        std::fs::write(vault.join("inside").join("Rain.typ"), "Rain").unwrap();
        move_folder(&vault, &to).unwrap();
        assert!(!vault.exists());
        assert_eq!(std::fs::read_to_string(to.join("inside").join("Rain.typ")).unwrap(), "Rain");
    }

    #[test]
    fn a_copy_is_checked_before_the_original_goes() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        std::fs::create_dir_all(from.join("deep")).unwrap();
        std::fs::write(from.join("deep").join("a.db"), vec![7u8; 3000]).unwrap();
        let to = dir.path().join("to");
        copy_checked(&from, &to).unwrap();
        assert_eq!(std::fs::read(to.join("deep").join("a.db")).unwrap().len(), 3000);
    }
}
