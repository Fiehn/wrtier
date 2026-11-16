//! The Colours Vault Mod's data: a colour for a Work or a Folder, from the same fixed slots as
//! the Reviewers' (`COLOURS`), shown in the Vault browser and Find in Vault. It's a Mod's own
//! data in `vault.db`, never in a History, so a Milestone or a Revert never changes it, and
//! switching the Mod off keeps every colour for later.
//!
//! One record, one field per Work or Folder: "work.<Work ID>" (`storage::identity`, so the
//! colour follows renames, moves and the trash) and "folder.<path>" (which follows the app's own
//! renames, as a Hidden folder does). An empty value is no colour. Two Devices colouring the
//! same one: the later wins.

use super::moves::folder_under;
use super::*;
use std::collections::BTreeMap;

/// The record holding every colour.
const RECORD: &str = "colours";
const WORK: &str = "work.";
const FOLDER: &str = "folder.";

/// The colours of the Works and Folders in the Vault, by their names now. Only those with one.
#[derive(Serialize, Debug, PartialEq, Default)]
pub struct Colours {
    pub works: BTreeMap<String, String>,
    pub folders: BTreeMap<String, String>,
}

/// A colour slot, or "" for none.
fn check_colour(colour: &str) -> Result<(), String> {
    if colour.is_empty() || COLOURS.contains(&colour) { Ok(()) } else { Err(format!("Not a colour: {colour}")) }
}

impl Vault {
    /// Gives a Work a colour, or none (""). It must have a History with something in it.
    pub fn set_work_colour(&mut self, id: &str, colour: &str, now_ms: u64) -> Result<(), String> {
        check_colour(colour)?;
        let identity = self.work_identity(id)
            .map_err(|e| e.replace("then add Mods", "then give it a colour"))?;
        self.set_colour_field(&format!("{WORK}{identity}"), colour, now_ms)
    }

    /// Gives a Folder (it must be in the Vault) a colour, or none ("").
    pub fn set_folder_colour(&mut self, path: &str, colour: &str, now_ms: u64) -> Result<(), String> {
        check_colour(colour)?;
        let path = self.checked_folder(path)?;
        if !folder_under(&self.root, &path)?.is_dir() {
            return Err(format!("{path} isn't a folder in the Vault anymore."));
        }
        self.set_colour_field(&format!("{FOLDER}{path}"), colour, now_ms)
    }

    fn set_colour_field(&mut self, field: &str, colour: &str, now_ms: u64) -> Result<(), String> {
        if self.vault_doc.field(RECORD, field).unwrap_or("") == colour {
            return Ok(());
        }
        let change = self.vault_doc.set_field_local(RECORD, field, colour, now_ms);
        self.save_vault(&[change])
    }

    /// The colours of the Works (`histories`, their `.db` files) and of the Folders, by name.
    pub(super) fn colours(&mut self, histories: &[PathBuf]) -> Colours {
        let set: Vec<(String, String)> = self.vault_doc.fields_of(RECORD).into_iter().filter(|(_, c)| !c.is_empty()).collect();
        let mut colours = Colours::default();
        if set.is_empty() {
            return colours; // nothing to look up: no need to read any Work's ID
        }
        let by_identity: HashMap<&str, &str> = set.iter()
            .filter_map(|(field, c)| Some((field.strip_prefix(WORK)?, c.as_str())))
            .collect();
        if !by_identity.is_empty() {
            for db in histories {
                let Some(identity) = self.identity_of(db) else { continue };
                if let Some(colour) = by_identity.get(identity.as_str()) {
                    colours.works.insert(self.id_of(db), colour.to_string());
                }
            }
        }
        for (field, colour) in &set {
            if let Some(path) = field.strip_prefix(FOLDER) {
                colours.folders.insert(path.to_string(), colour.clone());
            }
        }
        colours
    }

    /// The app renamed or moved the Folder `from` to `to`: the colours of it and the Folders in
    /// it follow. (A Work's follows by itself, being kept by the Work's ID.)
    pub(super) fn folder_colours_follow(&mut self, from: &str, to: &str, now_ms: u64) -> Result<(), String> {
        let mut changes = Vec::new();
        for (field, colour) in self.vault_doc.fields_of(RECORD) {
            let Some(path) = field.strip_prefix(FOLDER) else { continue };
            if colour.is_empty() {
                continue;
            }
            let moved = if path == from {
                to.to_string()
            } else if let Some(rest) = path.strip_prefix(from).and_then(|r| r.strip_prefix('/')) {
                format!("{to}/{rest}")
            } else {
                continue;
            };
            changes.push(self.vault_doc.set_field_local(RECORD, &field, "", now_ms));
            changes.push(self.vault_doc.set_field_local(RECORD, &format!("{FOLDER}{moved}"), &colour, now_ms));
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

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_works_colour_follows_it_through_renames_and_the_trash() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.create_work("Storm", 1000).unwrap();
        vault.set_work_colour("Rain", "red", 2000).unwrap();
        assert_eq!(vault.list(2100).colours.works, map(&[("Rain", "red")]));
        vault.rename_work("Rain", "Short stories/Rain").unwrap();
        assert_eq!(vault.list(3100).colours.works, map(&[("Short stories/Rain", "red")]));
        vault.trash_work("Short stories/Rain", 4000).unwrap();
        assert!(vault.list(4100).colours.works.is_empty());
        vault.restore_work("Short stories/Rain").unwrap();
        assert_eq!(vault.list(5100).colours.works, map(&[("Short stories/Rain", "red")]));
        // No colour again; and a colour that isn't one of the slots is refused.
        vault.set_work_colour("Short stories/Rain", "", 6000).unwrap();
        assert!(vault.list(6100).colours.works.is_empty());
        assert!(vault.set_work_colour("Storm", "navy", 7000).is_err());
        assert!(vault.set_work_colour("Nowhere", "red", 7000).is_err());
    }

    #[test]
    fn a_new_work_where_a_coloured_one_was_has_no_colour() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.set_work_colour("Rain", "red", 2000).unwrap();
        assert_eq!(vault.list(2100).colours.works, map(&[("Rain", "red")]));
        vault.rename_work("Rain", "Old/Rain").unwrap();
        vault.create_work("Rain", 3000).unwrap();
        assert_eq!(vault.list(3100).colours.works, map(&[("Old/Rain", "red")]));
    }

    #[test]
    fn a_folders_colour_follows_the_apps_own_renames() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_folder("Writing/Short stories").unwrap();
        vault.create_folder("Writing Notes").unwrap();
        vault.set_folder_colour("Writing", "teal", 1000).unwrap();
        vault.set_folder_colour("Writing/Short stories", "amber", 1000).unwrap();
        vault.set_folder_colour("Writing Notes", "green", 1000).unwrap();
        assert!(vault.set_folder_colour("Nowhere", "red", 1000).is_err());
        vault.rename_folder("Writing", "Archive", 2000).unwrap();
        assert_eq!(vault.list(2100).colours.folders, map(&[("Archive", "teal"), ("Archive/Short stories", "amber"), ("Writing Notes", "green")]));
    }

    #[test]
    fn colours_are_shared_and_the_later_wins() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.set_work_colour("Rain", "red", 2000).unwrap();
        let Opened::Vault(mut other) = Vault::open(dir.path(), DeviceId(2), false, &dir.path().join(".recovery2")).unwrap() else { panic!() };
        assert_eq!(other.list(2100).colours.works, map(&[("Rain", "red")]));
        other.set_work_colour("Rain", "violet", 3000).unwrap();
        vault.absorb_vault_copies(&mut Vec::new());
        assert_eq!(vault.list(3100).colours.works, map(&[("Rain", "violet")]));
    }

    #[test]
    fn switching_the_mod_off_keeps_the_colours() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        assert_eq!(vault.set_vault_mod("colours", true, 1500).unwrap().vault, vec!["colours".to_string()]);
        vault.set_work_colour("Rain", "red", 2000).unwrap();
        assert!(vault.set_vault_mod("colours", false, 3000).unwrap().vault.is_empty());
        assert_eq!(vault.list(3100).colours.works, map(&[("Rain", "red")]), "kept, for when it's back");
        // A Vault Mod isn't a Mod of any Work.
        assert!(vault.mods(Some("Rain")).unwrap().work.is_empty());
    }
}
