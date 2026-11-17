//! What Marks and the Mods that make them keep in `vault.db`, never in a History.
//!
//! Dismissed Marks (ADR 0018): the record "dismissed/<Work ID>", one field per Mark,
//! "<mod>/<rule>/<its first character's ID>", holding the IDs of its characters separated by
//! spaces; "" once the Author asks to see them again. Kept by the characters' IDs, a dismissal
//! follows them through edits, Moves, Reverts and other Devices.
//!
//! Mods' own settings, as fields "<mod>.<name>" (the Dialogue checker's Speech verbs, its rules
//! switched off): for the whole Vault in the shared settings record ("mod_setting.<mod>.<name>"),
//! for one Work in "mod_settings/<Work ID>", and for new Works in the shared settings record
//! ("new_work_mod_setting.<mod>.<name>"), copied into a new Work's when it's made. "" is unset.
//! This side doesn't know what they mean (the interface does), as with the Mods themselves.

use super::*;
use std::collections::BTreeMap;

fn dismissed_record(identity: &str) -> String {
    format!("dismissed/{identity}")
}
fn settings_record(identity: &str) -> String {
    format!("mod_settings/{identity}")
}
const SHARED: &str = "settings";
const VAULT_SETTING: &str = "mod_setting.";
const NEW_WORK_SETTING: &str = "new_work_mod_setting.";

/// The Mods' settings, each "<mod>.<name>" with its value (unset ones left out), and how many
/// Marks each Mod has dismissed in the Work.
#[derive(Serialize, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModSettings {
    pub vault: BTreeMap<String, String>,
    /// Empty when no Work was asked about.
    pub work: BTreeMap<String, String>,
    pub new_works: BTreeMap<String, String>,
    pub dismissed: BTreeMap<String, usize>,
}

/// "<mod>.<name>": a Mod's name, a dot, and a name without spaces.
fn check_setting(key: &str) -> Result<(), String> {
    let ok = key.split_once('.').is_some_and(|(m, name)| {
        !m.is_empty() && m.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !name.is_empty() && !name.chars().any(|c| c.is_whitespace() || c.is_control())
    });
    if ok && key.len() <= 120 { Ok(()) } else { Err(format!("Not a Mod setting: {key:?}")) }
}

/// "<mod>/<rule>".
fn check_mark_key(key: &str) -> Result<(), String> {
    let ok = key.split_once('/').is_some_and(|(m, rule)| {
        !m.is_empty() && m.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !rule.is_empty() && rule.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    });
    if ok { Ok(()) } else { Err(format!("Not a Mark: {key:?}")) }
}

impl Vault {
    /// The fields of `record` that start with `prefix` and are set, without the prefix.
    fn set_fields(&self, record: &str, prefix: &str) -> BTreeMap<String, String> {
        self.vault_doc.fields_of(record).into_iter()
            .filter(|(_, v)| !v.is_empty())
            .filter_map(|(f, v)| Some((f.strip_prefix(prefix)?.to_string(), v)))
            .collect()
    }

    /// The Mods' settings: the Vault's, the Work's (if `id` is given) and new Works'.
    pub fn mod_settings(&self, id: Option<&str>) -> Result<ModSettings, String> {
        let (work, dismissed) = match id {
            Some(id) => {
                let identity = self.work_identity(id)?;
                let mut dismissed: BTreeMap<String, usize> = BTreeMap::new();
                for (field, _) in self.set_fields(&dismissed_record(&identity), "") {
                    if let Some((m, _)) = field.split_once('/') {
                        *dismissed.entry(m.to_string()).or_default() += 1;
                    }
                }
                (self.set_fields(&settings_record(&identity), ""), dismissed)
            }
            None => (BTreeMap::new(), BTreeMap::new()),
        };
        Ok(ModSettings {
            vault: self.set_fields(SHARED, VAULT_SETTING),
            work,
            new_works: self.set_fields(SHARED, NEW_WORK_SETTING),
            dismissed,
        })
    }

    /// Sets a Mod's setting ("" unsets it) for the whole Vault (`scope` "vault"), for one Work
    /// ("work", with `id`), or for new Works ("newWorks").
    pub fn set_mod_setting(&mut self, scope: &str, id: Option<&str>, key: &str, value: &str, now_ms: u64) -> Result<ModSettings, String> {
        check_setting(key)?;
        let (record, field) = match (scope, id) {
            ("vault", _) => (SHARED.to_string(), format!("{VAULT_SETTING}{key}")),
            ("newWorks", _) => (SHARED.to_string(), format!("{NEW_WORK_SETTING}{key}")),
            ("work", Some(id)) => (settings_record(&self.work_identity(id)?), key.to_string()),
            _ => return Err(format!("Not somewhere a Mod setting goes: {scope}")),
        };
        if self.vault_doc.field(&record, &field).unwrap_or("") != value {
            let change = self.vault_doc.set_field_local(&record, &field, value, now_ms);
            self.save_vault(&[change])?;
        }
        self.mod_settings(id)
    }

    /// A new Work gets the Mods' settings for new Works.
    pub(super) fn give_new_work_mod_settings(&mut self, identity: &str, now_ms: u64) -> Result<(), String> {
        let record = settings_record(identity);
        let changes: Vec<Change> = self.set_fields(SHARED, NEW_WORK_SETTING).iter()
            .map(|(key, value)| self.vault_doc.set_field_local(&record, key, value, now_ms))
            .collect();
        if changes.is_empty() {
            return Ok(());
        }
        self.save_vault(&changes)
    }

    /// The Marks dismissed in a Work: each "<mod>/<rule>" with its characters' IDs.
    pub(super) fn dismissed_marks(&self, id: &str) -> Result<Vec<(String, Vec<String>)>, String> {
        let identity = self.work_identity(id)?;
        Ok(self.set_fields(&dismissed_record(&identity), "").into_iter()
            .filter_map(|(field, ids)| {
                let (m, rest) = field.split_once('/')?;
                let (rule, _) = rest.split_once('/')?;
                Some((format!("{m}/{rule}"), ids.split(' ').map(str::to_string).collect()))
            })
            .collect())
    }

    /// Keeps Marks the Author dismissed in a Work: each "<mod>/<rule>" with its characters' IDs.
    pub(super) fn keep_dismissed(&mut self, id: &str, marks: &[(String, Vec<String>)], now_ms: u64) -> Result<(), String> {
        let record = dismissed_record(&self.work_identity(id)?);
        let mut changes = Vec::new();
        for (key, ids) in marks {
            check_mark_key(key)?;
            let Some(first) = ids.first() else { continue };
            changes.push(self.vault_doc.set_field_local(&record, &format!("{key}/{first}"), &ids.join(" "), now_ms));
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.save_vault(&changes)
    }

    /// Shows a Mod's Dismissed Marks in a Work again.
    pub fn show_dismissed(&mut self, id: &str, name: &str, now_ms: u64) -> Result<ModSettings, String> {
        let record = dismissed_record(&self.work_identity(id)?);
        let prefix = format!("{name}/");
        let changes: Vec<Change> = self.set_fields(&record, "").into_keys()
            .filter(|field| field.starts_with(&prefix))
            .collect::<Vec<_>>().iter()
            .map(|field| self.vault_doc.set_field_local(&record, field, "", now_ms))
            .collect();
        if !changes.is_empty() {
            self.save_vault(&changes)?;
        }
        self.mod_settings(Some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;
    use crate::work::{DismissedMark, Op};

    fn dismissed(vault: &mut Vault, id: &str, now: u64) -> Vec<DismissedMark> {
        vault.edit(id, vec![Op::Dismissed], now).unwrap().dismissed.unwrap()
    }

    #[test]
    fn a_dismissed_mark_follows_its_characters_and_is_kept_outside_the_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![Op::Type { from: 0, to: 0, text: "\"Hi.\" she said.".into(), origin: Default::default() }], 2000).unwrap();
        let changes = vault.works["Rain"].doc.change_count();
        // The period, typed in the burst still open: it closes, so the period has an ID.
        let state = vault.edit("Rain", vec![Op::Dismiss { from: 3, to: 4, key: "dialogue/period-before-tag".into() }, Op::Dismissed], 3000).unwrap();
        assert_eq!(state.dismissed.unwrap(), vec![DismissedMark { key: "dialogue/period-before-tag".into(), ranges: vec![(3, 4)] }]);
        assert!(vault.works["Rain"].doc.change_count() <= changes + 1, "only the burst closing is recorded");
        assert_eq!(vault.mod_settings(Some("Rain")).unwrap().dismissed, BTreeMap::from([("dialogue".to_string(), 1)]));
        // Text typed before it: it moves along.
        vault.edit("Rain", vec![Op::Type { from: 0, to: 0, text: "Then: ".into(), origin: Default::default() }], 4000).unwrap();
        assert_eq!(dismissed(&mut vault, "Rain", 5000)[0].ranges, vec![(9, 10)]);
        // Deleted: nothing to show (and typed again, it's new text).
        vault.edit("Rain", vec![Op::Delete { from: 9, to: 10, origin: Default::default() }], 6000).unwrap();
        assert!(dismissed(&mut vault, "Rain", 7000).is_empty());
        vault.edit("Rain", vec![Op::Type { from: 9, to: 9, text: ".".into(), origin: Default::default() }], 8000).unwrap();
        assert!(dismissed(&mut vault, "Rain", 9000).is_empty());
    }

    #[test]
    fn dismissed_marks_can_be_shown_again_one_mod_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![Op::Type { from: 0, to: 0, text: "Some text here.".into(), origin: Default::default() }], 2000).unwrap();
        vault.edit("Rain", vec![
            Op::Dismiss { from: 0, to: 4, key: "dialogue/speaker-change".into() },
            Op::Dismiss { from: 5, to: 9, key: "spellcheck/mistake".into() },
        ], 3000).unwrap();
        assert_eq!(dismissed(&mut vault, "Rain", 4000).len(), 2);
        let settings = vault.show_dismissed("Rain", "dialogue", 5000).unwrap();
        assert_eq!(settings.dismissed, BTreeMap::from([("spellcheck".to_string(), 1)]));
        assert_eq!(dismissed(&mut vault, "Rain", 6000).iter().map(|d| d.key.as_str()).collect::<Vec<_>>(), ["spellcheck/mistake"]);
        assert!(vault.edit("Rain", vec![Op::Dismiss { from: 0, to: 4, key: "dialogue/no such".into() }], 7000).is_err());
    }

    #[test]
    fn mod_settings_for_the_vault_a_work_and_new_works() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.set_mod_setting("vault", None, "dialogue.verb.grinned", "add", 2000).unwrap();
        vault.set_mod_setting("work", Some("Rain"), "dialogue.rule.speaker-change", "off", 2000).unwrap();
        vault.set_mod_setting("newWorks", None, "dialogue.rule.action-as-tag", "off", 2000).unwrap();
        let rain = vault.mod_settings(Some("Rain")).unwrap();
        assert_eq!(rain.vault, BTreeMap::from([("dialogue.verb.grinned".to_string(), "add".to_string())]));
        assert_eq!(rain.work, BTreeMap::from([("dialogue.rule.speaker-change".to_string(), "off".to_string())]));
        // A new Work starts with the settings for new Works.
        vault.create_work("Storm", 3000).unwrap();
        assert_eq!(vault.mod_settings(Some("Storm")).unwrap().work, BTreeMap::from([("dialogue.rule.action-as-tag".to_string(), "off".to_string())]));
        // Unset.
        vault.set_mod_setting("vault", None, "dialogue.verb.grinned", "", 4000).unwrap();
        assert!(vault.mod_settings(None).unwrap().vault.is_empty());
        assert!(vault.set_mod_setting("vault", None, "no dot", "x", 5000).is_err());
        assert!(vault.set_mod_setting("work", None, "dialogue.x", "x", 5000).is_err());
    }
}
