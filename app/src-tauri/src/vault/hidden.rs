//! Hidden folders: Folders the Author has chosen not to see in the Vault browser. Which ones is a
//! Vault setting, kept in `vault.db` like the shortcuts: one field per Folder ("on" or "off"),
//! in the shared record, or in this Device's own record once it keeps its own list. Two Devices
//! hiding different Folders both keep theirs; for the same Folder the later one wins.
//!
//! A Folder is known by its path. The app's own renames carry the mark along (in every record,
//! so a Device with its own list keeps it too); a rename made any other way can't be told from
//! a new Folder, so it shows again. A path that's gone from the Vault stays listed until the
//! Author removes it, as sync may bring it back.

use super::moves::folder_under;
use super::*;
use wrtier::change::device_record;

/// A hidden Folder: the field "hidden_folder.<path>" of the record the list is kept in.
const HIDDEN_FOLDER: &str = "hidden_folder.";
/// The field of this Device's record that says it keeps its own list ("on").
const HIDDEN_HERE: &str = "hidden_folders_here";
/// The record of the settings every Device shares (as in settings.rs).
const SHARED: &str = "settings";

/// The Folders hidden in the fields of `record`, sorted.
fn hidden_in(doc: &Doc, record: &str) -> Vec<String> {
    doc.fields_of(record).into_iter()
        .filter(|(_, value)| value == "on")
        .filter_map(|(field, _)| Some(field.strip_prefix(HIDDEN_FOLDER)?.to_string()))
        .collect()
}

/// `path` with `from` at its start replaced by `to`, if it's `from` or inside it.
fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        Some(to.to_string())
    } else {
        path.strip_prefix(from)?.strip_prefix('/').map(|rest| format!("{to}/{rest}"))
    }
}

impl Vault {
    pub(super) fn hidden_folders_here(&self) -> bool {
        self.vault_doc.field(&device_record(self.device), HIDDEN_HERE) == Some("on")
    }

    /// Where this Device's list is kept: its own record, or the shared one.
    fn hidden_record(&self) -> String {
        if self.hidden_folders_here() { device_record(self.device) } else { SHARED.to_string() }
    }

    /// The Folders hidden on this Device, sorted; ones gone from the Vault too.
    pub fn hidden_folders(&self) -> Vec<String> {
        hidden_in(&self.vault_doc, &self.hidden_record())
    }

    /// Hides a Folder (it must be in the Vault), or shows it again (any listed path, there or
    /// not). Returns the Folders hidden now.
    pub fn set_folder_hidden(&mut self, path: &str, hidden: bool, now_ms: u64) -> Result<Vec<String>, String> {
        let path = if hidden {
            let path = self.checked_folder(path)?;
            if !folder_under(&self.root, &path)?.is_dir() {
                return Err(format!("{path} isn't a folder in the Vault anymore."));
            }
            path
        } else {
            path.to_string()
        };
        let record = self.hidden_record();
        let field = format!("{HIDDEN_FOLDER}{path}");
        let is_hidden = self.vault_doc.field(&record, &field) == Some("on");
        if is_hidden != hidden {
            let change = self.vault_doc.set_field_local(&record, &field, if hidden { "on" } else { "off" }, now_ms);
            self.save_vault(&[change])?;
        }
        Ok(self.hidden_folders())
    }

    /// This Device keeps its own list of Hidden folders (starting as a copy of the Vault's), or
    /// goes back to the Vault's.
    pub fn set_hidden_folders_here(&mut self, here: bool, now_ms: u64) -> Result<Settings, String> {
        if here == self.hidden_folders_here() {
            return Ok(self.settings());
        }
        let mine = device_record(self.device);
        let mut changes = Vec::new();
        if here {
            // Its own list starts as the Vault's: copy that, and clear any left from before.
            let shared = hidden_in(&self.vault_doc, SHARED);
            let own = hidden_in(&self.vault_doc, &mine);
            for path in shared.iter().filter(|p| !own.contains(p)) {
                changes.push(self.vault_doc.set_field_local(&mine, &format!("{HIDDEN_FOLDER}{path}"), "on", now_ms));
            }
            for path in own.iter().filter(|p| !shared.contains(p)) {
                changes.push(self.vault_doc.set_field_local(&mine, &format!("{HIDDEN_FOLDER}{path}"), "off", now_ms));
            }
        }
        changes.push(self.vault_doc.set_field_local(&mine, HIDDEN_HERE, if here { "on" } else { "off" }, now_ms));
        self.save_vault(&changes)?;
        Ok(self.settings())
    }

    /// The app renamed or moved the Folder `from` to `to`: Hidden folders at or inside it follow,
    /// in the shared list and every Device's own.
    pub(super) fn hidden_folders_follow(&mut self, from: &str, to: &str, now_ms: u64) -> Result<(), String> {
        let mut records = vec![SHARED.to_string()];
        records.extend(self.vault_doc.records("device/"));
        let mut changes = Vec::new();
        for record in records {
            for path in hidden_in(&self.vault_doc, &record) {
                let Some(new_path) = moved_path(&path, from, to) else { continue };
                changes.push(self.vault_doc.set_field_local(&record, &format!("{HIDDEN_FOLDER}{path}"), "off", now_ms));
                changes.push(self.vault_doc.set_field_local(&record, &format!("{HIDDEN_FOLDER}{new_path}"), "on", now_ms));
            }
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.save_vault(&changes)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn other_device(dir: &Path, n: u32) -> Vault {
        let Opened::Vault(other) = Vault::open(dir, DeviceId(n), false, &dir.join(format!(".recovery{n}"))).unwrap() else { panic!() };
        other
    }

    #[test]
    fn a_hidden_folder_is_hidden_on_every_device() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Old drafts/Rain v1", 1000).unwrap();
        vault.create_folder("Old drafts/2023").unwrap();
        assert_eq!(vault.set_folder_hidden("Old drafts", true, 2000).unwrap(), names(&["Old drafts"]));
        assert_eq!(vault.hidden_folders(), names(&["Old drafts"]), "hiding it again changes nothing");
        assert_eq!(vault.set_folder_hidden("Old drafts", true, 2500).unwrap(), names(&["Old drafts"]));
        // Another Device sees it hidden too, and can show it again for everyone.
        let mut other = other_device(dir.path(), 2);
        assert_eq!(other.hidden_folders(), names(&["Old drafts"]));
        assert!(other.set_folder_hidden("Old drafts", false, 3000).unwrap().is_empty());
        vault.absorb_vault_copies(&mut Vec::new());
        assert!(vault.hidden_folders().is_empty());
    }

    #[test]
    fn only_a_folder_in_the_vault_can_be_hidden_but_a_gone_one_can_be_shown() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        assert!(vault.set_folder_hidden("Nowhere", true, 1000).is_err());
        assert!(vault.set_folder_hidden("../outside", true, 1000).is_err());
        vault.create_folder("Old drafts").unwrap();
        vault.set_folder_hidden("Old drafts", true, 2000).unwrap();
        // Gone from the Vault: still listed, until it's removed from the list.
        vault.trash_folder("Old drafts", 3000).unwrap();
        assert_eq!(vault.hidden_folders(), names(&["Old drafts"]));
        assert!(vault.set_folder_hidden("Old drafts", false, 4000).unwrap().is_empty());
    }

    #[test]
    fn a_device_can_keep_its_own_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_folder("Old drafts").unwrap();
        vault.create_folder("Notes").unwrap();
        vault.set_folder_hidden("Old drafts", true, 1000).unwrap();
        let mut phone = other_device(dir.path(), 2);
        // Its own list starts as a copy of the Vault's, and changes only there.
        let settings = phone.set_hidden_folders_here(true, 2000).unwrap();
        assert!(settings.hidden_folders_here);
        assert_eq!(phone.hidden_folders(), names(&["Old drafts"]));
        phone.set_folder_hidden("Notes", true, 3000).unwrap();
        vault.absorb_vault_copies(&mut Vec::new());
        assert_eq!(vault.hidden_folders(), names(&["Old drafts"]), "the Vault's list is unchanged");
        // Hiding on the laptop doesn't reach the phone's own list; going back to the Vault's does.
        vault.create_folder("Ideas").unwrap();
        vault.set_folder_hidden("Ideas", true, 4000).unwrap();
        phone.absorb_vault_copies(&mut Vec::new());
        assert_eq!(phone.hidden_folders(), names(&["Notes", "Old drafts"]));
        phone.set_hidden_folders_here(false, 5000).unwrap();
        assert_eq!(phone.hidden_folders(), names(&["Ideas", "Old drafts"]));
        // Taking its own again starts afresh from the Vault's, not from what it had before.
        phone.set_hidden_folders_here(true, 6000).unwrap();
        assert_eq!(phone.hidden_folders(), names(&["Ideas", "Old drafts"]));
    }

    #[test]
    fn the_mark_follows_the_apps_own_renames() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_folder("Writing/Old drafts/2023").unwrap();
        vault.create_folder("Writing Notes").unwrap();
        vault.set_folder_hidden("Writing/Old drafts", true, 1000).unwrap();
        vault.set_folder_hidden("Writing/Old drafts/2023", true, 1000).unwrap();
        vault.set_folder_hidden("Writing Notes", true, 1000).unwrap();
        let mut phone = other_device(dir.path(), 2);
        phone.set_hidden_folders_here(true, 2000).unwrap();
        vault.absorb_vault_copies(&mut Vec::new());
        // Renaming a Folder they're in: both lists follow; a Folder that only starts the same doesn't.
        vault.rename_folder("Writing", "Archive", 3000).unwrap();
        assert_eq!(vault.hidden_folders(), names(&["Archive/Old drafts", "Archive/Old drafts/2023", "Writing Notes"]));
        phone.absorb_vault_copies(&mut Vec::new());
        assert_eq!(phone.hidden_folders(), names(&["Archive/Old drafts", "Archive/Old drafts/2023", "Writing Notes"]));
        // Renamed outside the app: the app can't tell, so it shows again.
        std::fs::rename(dir.path().join("Writing Notes"), dir.path().join("Notes")).unwrap();
        assert!(vault.hidden_folders().contains(&"Writing Notes".to_string()), "the old path stays listed");
    }

    #[test]
    fn paths_move_only_at_or_inside_the_folder() {
        assert_eq!(moved_path("Writing", "Writing", "Archive").as_deref(), Some("Archive"));
        assert_eq!(moved_path("Writing/Old", "Writing", "Archive").as_deref(), Some("Archive/Old"));
        assert_eq!(moved_path("Writing Notes", "Writing", "Archive"), None);
        assert_eq!(moved_path("Notes", "Writing", "Archive"), None);
    }
}
