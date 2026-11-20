//! The Vault name (ADR 0022): what a Vault is called, kept in `vault.db` so it's the same on
//! every Device. It starts as the folder's name; renaming the Vault never renames or moves the
//! folder.

use super::*;

/// The record holding what's about the Vault itself.
const RECORD: &str = "vault";
const NAME: &str = "name";

/// What a Vault is called until it's given a name: its folder's.
pub fn folder_name(root: &Path) -> String {
    root.file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| root.display().to_string())
}

/// A Vault name as given: trimmed, 1 to 100 characters.
pub fn checked_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err("A Vault's name needs 1 to 100 characters.".into());
    }
    Ok(name.to_string())
}

impl Vault {
    /// The Vault name: the one it was given, or its folder's.
    pub fn name(&self) -> String {
        name_in(&self.vault_doc, &self.root)
    }

    pub fn set_name(&mut self, name: &str, now_ms: u64) -> Result<(), String> {
        let name = checked_name(name)?;
        let change = self.vault_doc.set_field_local(RECORD, NAME, &name, now_ms);
        self.save_vault(&[change])
    }
}

fn name_in(doc: &Doc, root: &Path) -> String {
    doc.field(RECORD, NAME).filter(|n| !n.trim().is_empty()).map(str::to_string).unwrap_or_else(|| folder_name(root))
}

/// The Vault name of a Vault that isn't open, read from its `vault.db`.
pub fn read_name(root: &Path, device: DeviceId) -> Result<String, String> {
    let (_, doc) = storage::load(&root.join(VAULT_FILE), device).map_err(err)?;
    Ok(name_in(&doc, root))
}

/// Renames a Vault that isn't open: only its `vault.db` is written, as any Device writes it.
pub fn rename_closed(root: &Path, device: DeviceId, name: &str, now_ms: u64) -> Result<String, String> {
    let name = checked_name(name)?;
    let file = root.join(VAULT_FILE);
    if !matches!(storage::probe(&file), Probe::History) {
        return Err(format!("{} can't be read as a Vault right now.", root.display()));
    }
    let (_, mut doc) = storage::load(&file, device).map_err(err)?;
    let change = doc.set_field_local(RECORD, NAME, &name, now_ms);
    storage::save(&file, &[change], &doc).map_err(err)?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: DeviceId = DeviceId(1);

    fn new_vault(root: &Path) -> Vault {
        match Vault::open(root, ME, true, &root.join(".recovery")).unwrap() {
            Opened::Vault(v) => v,
            Opened::NotAVault => panic!("asked to create it"),
        }
    }

    #[test]
    fn a_vault_is_called_after_its_folder_until_it_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Writing");
        std::fs::create_dir(&root).unwrap();
        let mut vault = new_vault(&root);
        assert_eq!(vault.name(), "Writing");
        vault.set_name("  My novels ", 1).unwrap();
        assert_eq!(vault.name(), "My novels");
        assert_eq!(read_name(&root, ME).unwrap(), "My novels", "kept in vault.db");
        assert!(root.is_dir(), "the folder keeps its own name");
    }

    #[test]
    fn a_vault_that_isnt_open_can_be_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Old");
        std::fs::create_dir(&root).unwrap();
        drop(new_vault(&root));
        assert_eq!(rename_closed(&root, DeviceId(2), "Old drafts", 5).unwrap(), "Old drafts");
        assert_eq!(new_vault(&root).name(), "Old drafts");
    }

    #[test]
    fn a_name_needs_some_characters() {
        assert!(checked_name("   ").is_err());
        assert!(checked_name(&"x".repeat(101)).is_err());
        assert_eq!(checked_name(" Stories ").unwrap(), "Stories");
    }
}
