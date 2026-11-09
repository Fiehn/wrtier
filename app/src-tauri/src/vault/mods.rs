//! Mods (ADR 0013): optional features that ship with the app, added to each Work separately.
//! Which Mods a Work has is kept here in `vault.db`, under the Work's ID, never in its History,
//! so returning to a Milestone or a Revert never adds or removes one.
//!
//! Each Mod is its own field, "on" or "off", like shortcuts: two Devices switching different
//! Mods both keep their switch, and for the same Mod the later one wins. This side doesn't know
//! which Mods there are (the interface does), so a Mod from a newer app is kept as it is.

use super::*;
use wrtier::change::device_record;

/// The record holding one Work's Mods, by the Work's ID (`storage::identity`).
fn work_record(identity: &str) -> String {
    format!("mods/{identity}")
}

/// A Mod for new Works: the field "new_work_mod.<mod>" of the shared settings record.
const NEW_WORK: &str = "new_work_mod.";
/// A Mod this Device hides: the field "hide_mod.<mod>" of its own record.
const HIDDEN: &str = "hide_mod.";
/// The record of the settings every Device shares (as in settings.rs).
const SHARED: &str = "settings";

/// Which Mods a Work has, which new Works get, and which this Device hides. Each holds only the
/// Mods switched on; a Mod this app doesn't know is listed too.
#[derive(Serialize, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Mods {
    /// The Work's Mods (empty when no Work was asked about).
    pub work: Vec<String>,
    pub new_works: Vec<String>,
    pub hidden: Vec<String>,
}

fn check_mod(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 40 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("Not a Mod: {name:?}"));
    }
    Ok(())
}

impl Vault {
    /// What tells a Work from every other (`storage::identity`), open or not.
    fn work_identity(&self, id: &str) -> Result<String, String> {
        let id = self.current_id(id);
        let identity = match self.works.get(&id) {
            Some(work) => work.identity().map(str::to_string),
            None => {
                let (db, _) = self.paths(&id)?;
                if !db.exists() {
                    return Err(format!("There's no Work called {id}."));
                }
                storage::identity(&db).map_err(err)?
            }
        };
        identity.ok_or_else(|| format!("{id} has nothing in it yet. Write something first, then add Mods."))
    }

    /// The Mods switched on in the fields of `record` starting with `prefix`.
    fn mods_in(&self, record: &str, prefix: &str) -> Vec<String> {
        self.vault_doc.fields_of(record).into_iter()
            .filter(|(_, value)| value == "on")
            .filter_map(|(field, _)| Some(field.strip_prefix(prefix)?.to_string()))
            .collect()
    }

    /// The Mods of a Work (if `id` is given), of new Works, and hidden on this Device.
    pub fn mods(&self, id: Option<&str>) -> Result<Mods, String> {
        let work = match id {
            Some(id) => self.mods_in(&work_record(&self.work_identity(id)?), ""),
            None => Vec::new(),
        };
        Ok(Mods {
            work,
            new_works: self.mods_in(SHARED, NEW_WORK),
            hidden: self.mods_in(&device_record(self.device), HIDDEN),
        })
    }

    /// Sets one field to "on" or "off", saving only if it changes.
    fn switch(&mut self, record: &str, field: &str, on: bool, now_ms: u64) -> Result<(), String> {
        let value = if on { "on" } else { "off" };
        let is_on = self.vault_doc.field(record, field) == Some("on");
        if is_on == on {
            return Ok(());
        }
        let change = self.vault_doc.set_field_local(record, field, value, now_ms);
        self.save_vault(&[change])
    }

    /// Adds a Mod to a Work, or removes it.
    pub fn set_mod(&mut self, id: &str, name: &str, on: bool, now_ms: u64) -> Result<Mods, String> {
        check_mod(name)?;
        let record = work_record(&self.work_identity(id)?);
        self.switch(&record, name, on, now_ms)?;
        self.mods(Some(id))
    }

    /// Whether new Works start with a Mod.
    pub fn set_new_work_mod(&mut self, name: &str, on: bool, now_ms: u64) -> Result<Mods, String> {
        check_mod(name)?;
        self.switch(SHARED, &format!("{NEW_WORK}{name}"), on, now_ms)?;
        self.mods(None)
    }

    /// Hides a Mod on this Device, or shows it again. The Work keeps it either way.
    pub fn set_mod_hidden(&mut self, name: &str, hidden: bool, now_ms: u64) -> Result<Mods, String> {
        check_mod(name)?;
        let record = device_record(self.device);
        self.switch(&record, &format!("{HIDDEN}{name}"), hidden, now_ms)?;
        self.mods(None)
    }

    /// A new Work gets the Mods for new Works, including any this app doesn't know.
    pub(super) fn give_new_work_mods(&mut self, identity: &str, now_ms: u64) -> Result<(), String> {
        let record = work_record(identity);
        let changes: Vec<Change> = self.mods_in(SHARED, NEW_WORK).iter()
            .map(|name| self.vault_doc.set_field_local(&record, name, "on", now_ms))
            .collect();
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

    #[test]
    fn a_work_has_its_own_mods_and_new_works_get_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        assert!(vault.mods(Some("Rain")).unwrap().work.is_empty());
        vault.set_mod("Rain", "spellcheck", true, 2000).unwrap();
        assert_eq!(vault.mods(Some("Rain")).unwrap().work, names(&["spellcheck"]));
        // The Mods for new Works, one of them from a newer app: a new Work gets both.
        vault.set_new_work_mod("chapters", true, 3000).unwrap();
        vault.set_new_work_mod("dialogue", true, 3000).unwrap();
        vault.create_work("The Long Night", 4000).unwrap();
        assert_eq!(vault.mods(Some("The Long Night")).unwrap().work, names(&["chapters", "dialogue"]));
        // Rain is as it was.
        assert_eq!(vault.mods(Some("Rain")).unwrap().work, names(&["spellcheck"]));
        vault.set_mod("Rain", "spellcheck", false, 5000).unwrap();
        assert!(vault.mods(Some("Rain")).unwrap().work.is_empty());
        assert!(vault.set_mod("Rain", "spell check", true, 6000).is_err());
    }

    #[test]
    fn a_works_mods_follow_it_when_renamed_and_work_while_it_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.set_mod("Rain", "chapters", true, 2000).unwrap();
        vault.close_work("Rain", 3000);
        let renamed = vault.rename_work("Rain", "Storm").unwrap();
        assert_eq!(vault.mods(Some(&renamed)).unwrap().work, names(&["chapters"]));
        vault.set_mod(&renamed, "spellcheck", true, 4000).unwrap();
        assert_eq!(vault.mods(Some(&renamed)).unwrap().work, names(&["chapters", "spellcheck"]));
    }

    #[test]
    fn devices_switching_different_mods_keep_both_and_a_device_can_hide_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.close_work("Rain", 1500);
        let Opened::Vault(mut other) = Vault::open(dir.path(), DeviceId(2), false, &dir.path().join(".recovery2")).unwrap() else { panic!() };
        vault.set_mod("Rain", "chapters", true, 2000).unwrap();
        other.absorb_vault_copies(&mut Vec::new());
        other.set_mod("Rain", "dialogue", true, 3000).unwrap(); // a Mod this app may not know
        vault.absorb_vault_copies(&mut Vec::new());
        assert_eq!(vault.mods(Some("Rain")).unwrap().work, names(&["chapters", "dialogue"]));
        // Hidden here only.
        vault.set_mod_hidden("chapters", true, 4000).unwrap();
        other.absorb_vault_copies(&mut Vec::new());
        assert_eq!(vault.mods(None).unwrap().hidden, names(&["chapters"]));
        assert!(other.mods(None).unwrap().hidden.is_empty());
        assert_eq!(other.mods(Some("Rain")).unwrap().work, names(&["chapters", "dialogue"]), "hiding doesn't remove it");
    }
}
