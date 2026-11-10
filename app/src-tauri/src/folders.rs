//! The folder browser that picks a Vault on Android, where the system's own picker gives no file
//! path: the folders inside one folder, marking Vaults and Syncthing folders. Plain file paths,
//! the same file code as desktop, thanks to "All files access" (ADR 0006).

use crate::vault::VAULT_FILE;
use serde::Serialize;
use std::path::Path;

/// The folder Syncthing puts in every folder it shares.
const SYNCTHING_MARK: &str = ".stfolder";

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub name: String,
    pub path: String,
    /// It holds a `vault.db`.
    pub vault: bool,
    /// Syncthing shares it.
    pub syncthing: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub path: String,
    pub parent: Option<String>,
    pub vault: bool,
    pub syncthing: bool,
    /// The folders inside, by name, ignoring case. Hidden ones (".trash", ".stfolder") are left out.
    pub folders: Vec<Folder>,
}

fn is_vault(dir: &Path) -> bool {
    dir.join(VAULT_FILE).is_file()
}

fn is_syncthing(dir: &Path) -> bool {
    dir.join(SYNCTHING_MARK).exists()
}

/// The folders inside `dir`.
pub fn list(dir: &Path) -> Result<FolderListing, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Couldn't open {}: {e}", dir.display()))?;
    let mut folders: Vec<Folder> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let path = entry.path();
            // is_dir() follows links, as a folder reached through one is still a folder.
            if name.starts_with('.') || !path.is_dir() {
                return None;
            }
            Some(Folder { vault: is_vault(&path), syncthing: is_syncthing(&path), path: path.display().to_string(), name })
        })
        .collect();
    folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
    Ok(FolderListing {
        path: dir.display().to_string(),
        parent: dir.parent().map(|p| p.display().to_string()),
        vault: is_vault(dir),
        syncthing: is_syncthing(dir),
        folders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_folders_marking_vaults_and_syncthing_folders() {
        let root = tempfile::tempdir().unwrap();
        let sync = root.path().join("Sync");
        fs::create_dir_all(sync.join(SYNCTHING_MARK)).unwrap();
        fs::create_dir_all(sync.join("Novels")).unwrap();
        fs::write(sync.join("Novels").join(VAULT_FILE), b"").unwrap();
        fs::create_dir_all(sync.join("photos")).unwrap();
        fs::create_dir_all(sync.join(".trash")).unwrap();
        fs::write(sync.join("notes.txt"), b"").unwrap();

        let listing = list(&sync).unwrap();
        assert!(listing.syncthing);
        assert!(!listing.vault);
        assert_eq!(listing.parent.as_deref(), Some(root.path().display().to_string().as_str()));
        let names: Vec<_> = listing.folders.iter().map(|f| (f.name.as_str(), f.vault, f.syncthing)).collect();
        // Sorted ignoring case; files and hidden folders left out.
        assert_eq!(names, [("Novels", true, false), ("photos", false, false)]);
        assert_eq!(listing.folders[0].path, sync.join("Novels").display().to_string());

        let vault = list(&sync.join("Novels")).unwrap();
        assert!(vault.vault);
        assert!(vault.folders.is_empty());
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        assert!(list(&root.path().join("gone")).is_err());
    }
}
