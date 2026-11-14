//! Vault settings: preferences kept in `vault.db` and shared by every Device, such as the app
//! theme and the editor font size. Any Device can override one for itself; the override is
//! kept in the same file, under that Device's record, so it syncs and survives a reinstall.

use super::*;
use std::collections::BTreeMap;
use wrtier::change::device_record;

/// The record holding the settings every Device shares.
const SHARED: &str = "settings";

/// The settings this Device uses, and which of them are its own overrides.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// "system", "light", "dark", or "vault:<file>" for a theme in the Vault's `themes/`.
    pub theme: String,
    /// The editor's font size in pixels.
    pub font_size: u32,
    pub theme_here: bool,
    pub font_size_here: bool,
    /// Line numbers in the editor's margin.
    pub line_numbers: bool,
    pub line_numbers_here: bool,
    /// The line with the cursor shaded.
    pub highlight_line: bool,
    pub highlight_line_here: bool,
    /// The Page preview keeps the cursor's page in view (on by default).
    pub preview_follows: bool,
    pub preview_follows_here: bool,
    /// Comments' text highlighted in the editor (on by default).
    pub comment_highlights: bool,
    pub comment_highlights_here: bool,
    /// Keyboard shortcuts changed from the app's defaults: command → key (in CodeMirror's
    /// notation, such as "Mod-Shift-m"), or "none" for no key. The interface knows the commands
    /// and their defaults.
    pub shortcuts: BTreeMap<String, String>,
    /// This Device has its own shortcuts (for a different keyboard), instead of the Vault's.
    pub shortcuts_here: bool,
    /// This Device keeps its own list of Hidden folders (hidden.rs), instead of the Vault's.
    pub hidden_folders_here: bool,
}

/// A shortcut is the field "shortcut.<command>" of the shared record, or of this Device's own
/// if it has its own shortcuts. An empty value means the app's default.
const SHORTCUT: &str = "shortcut.";
/// The field of this Device's record that says it has its own shortcuts ("on").
const SHORTCUTS_HERE: &str = "shortcuts_here";

fn check_shortcut(command: &str, key: &str) -> Result<(), String> {
    if command.is_empty() || command.len() > 40 || !command.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("Not a command: {command:?}"));
    }
    if key.chars().count() > 60 || key.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("Not a key: {key:?}"));
    }
    Ok(())
}

/// The settings there are, as stored, with their default and what's allowed.
fn check(name: &str, value: &str) -> Result<(&'static str, String), String> {
    match name {
        "theme" if ["system", "light", "dark"].contains(&value) => Ok(("theme", value.to_string())),
        // A Vault theme, by its file. It may be missing on some Device (not synced yet, or
        // deleted): the interface then follows the system and says so.
        "theme" if value.strip_prefix("vault:").is_some_and(crate::themes::is_theme_file) => Ok(("theme", value.to_string())),
        "fontSize" => match value.parse::<u32>() {
            Ok(size) if (10..=40).contains(&size) => Ok(("font_size", size.to_string())),
            _ => Err("The font size must be a whole number from 10 to 40.".into()),
        },
        "lineNumbers" if ["on", "off"].contains(&value) => Ok(("line_numbers", value.to_string())),
        "highlightLine" if ["on", "off"].contains(&value) => Ok(("highlight_line", value.to_string())),
        "previewFollows" if ["on", "off"].contains(&value) => Ok(("preview_follows", value.to_string())),
        "commentHighlights" if ["on", "off"].contains(&value) => Ok(("comment_highlights", value.to_string())),
        _ => Err(format!("Not a setting: {name} = {value:?}")),
    }
}

impl Vault {
    /// A setting's value and whether it's this Device's own: the override if there is one, else
    /// the shared value, else `default`. (An empty value means "no override".)
    fn setting(&self, field: &str, default: &str) -> (String, bool) {
        let here = self.vault_doc.field(&device_record(self.device), field).filter(|v| !v.is_empty());
        match here {
            Some(value) => (value.to_string(), true),
            None => (self.vault_doc.field(SHARED, field).filter(|v| !v.is_empty()).unwrap_or(default).to_string(), false),
        }
    }

    pub fn settings(&self) -> Settings {
        let (theme, theme_here) = self.setting("theme", "system");
        let (font_size, font_size_here) = self.setting("font_size", "17");
        let (line_numbers, line_numbers_here) = self.setting("line_numbers", "off");
        let (highlight_line, highlight_line_here) = self.setting("highlight_line", "off");
        let (preview_follows, preview_follows_here) = self.setting("preview_follows", "on");
        let (comment_highlights, comment_highlights_here) = self.setting("comment_highlights", "on");
        Settings {
            theme,
            font_size: font_size.parse().unwrap_or(17),
            theme_here,
            font_size_here,
            line_numbers: line_numbers == "on",
            line_numbers_here,
            highlight_line: highlight_line == "on",
            highlight_line_here,
            preview_follows: preview_follows == "on",
            preview_follows_here,
            comment_highlights: comment_highlights == "on",
            comment_highlights_here,
            shortcuts: self.shortcuts(),
            shortcuts_here: self.shortcuts_here(),
            hidden_folders_here: self.hidden_folders_here(),
        }
    }

    fn shortcuts_here(&self) -> bool {
        self.vault_doc.field(&device_record(self.device), SHORTCUTS_HERE) == Some("on")
    }

    /// Where this Device's shortcuts are kept: its own record, or the shared one.
    fn shortcut_record(&self) -> String {
        if self.shortcuts_here() { device_record(self.device) } else { SHARED.to_string() }
    }

    /// The shortcuts kept in `record` that differ from the defaults.
    fn shortcuts_in(&self, record: &str) -> BTreeMap<String, String> {
        self.vault_doc.fields_of(record).into_iter()
            .filter_map(|(field, value)| Some((field.strip_prefix(SHORTCUT)?.to_string(), value)))
            .filter(|(_, value)| !value.is_empty())
            .collect()
    }

    fn shortcuts(&self) -> BTreeMap<String, String> {
        self.shortcuts_in(&self.shortcut_record())
    }

    /// Gives commands keys ("none": no key; "": the app's default), for every Device or, if this
    /// one has its own shortcuts, for this one. Several at once, so taking a key from another
    /// command is one save.
    pub fn set_shortcuts(&mut self, keys: &[(String, String)], now_ms: u64) -> Result<Settings, String> {
        for (command, key) in keys {
            check_shortcut(command, key)?;
        }
        let record = self.shortcut_record();
        let wanted: Vec<(String, String)> = keys.iter()
            .map(|(command, key)| (format!("{SHORTCUT}{command}"), key.clone()))
            .filter(|(field, key)| self.vault_doc.field(&record, field).unwrap_or("") != key)
            .collect();
        let changes: Vec<Change> = wanted.iter()
            .map(|(field, key)| self.vault_doc.set_field_local(&record, field, key, now_ms))
            .collect();
        if !changes.is_empty() {
            self.save_vault(&changes)?;
        }
        Ok(self.settings())
    }

    /// Every shortcut back to the app's default (where this Device's shortcuts are kept).
    pub fn reset_shortcuts(&mut self, now_ms: u64) -> Result<Settings, String> {
        let keys: Vec<(String, String)> = self.shortcuts().into_keys().map(|command| (command, String::new())).collect();
        self.set_shortcuts(&keys, now_ms)
    }

    /// This Device gets its own shortcuts (starting as a copy of the Vault's), or goes back to
    /// the Vault's.
    pub fn set_shortcuts_here(&mut self, here: bool, now_ms: u64) -> Result<Settings, String> {
        if here == self.shortcuts_here() {
            return Ok(self.settings());
        }
        let mine = device_record(self.device);
        let mut changes = Vec::new();
        if here {
            let shared = self.shortcuts_in(SHARED);
            let own = self.shortcuts_in(&mine);
            // Its own start the same as the Vault's: copy those, and clear any left from before.
            for (command, key) in &shared {
                if own.get(command) != Some(key) {
                    changes.push(self.vault_doc.set_field_local(&mine, &format!("{SHORTCUT}{command}"), key, now_ms));
                }
            }
            for command in own.keys().filter(|c| !shared.contains_key(*c)) {
                changes.push(self.vault_doc.set_field_local(&mine, &format!("{SHORTCUT}{command}"), "", now_ms));
            }
        }
        changes.push(self.vault_doc.set_field_local(&mine, SHORTCUTS_HERE, if here { "on" } else { "off" }, now_ms));
        self.save_vault(&changes)?;
        Ok(self.settings())
    }

    /// Sets a setting for every Device, or (`here`) only for this one. Setting it for every
    /// Device also drops this Device's override, so it follows the shared value again.
    pub fn set_setting(&mut self, name: &str, value: &str, here: bool, now_ms: u64) -> Result<Settings, String> {
        let (field, value) = check(name, value)?;
        let mine = device_record(self.device);
        let mut changes = Vec::new();
        if here {
            changes.push(self.vault_doc.set_field_local(&mine, field, &value, now_ms));
        } else {
            changes.push(self.vault_doc.set_field_local(SHARED, field, &value, now_ms));
            if self.vault_doc.field(&mine, field).is_some_and(|v| !v.is_empty()) {
                changes.push(self.vault_doc.set_field_local(&mine, field, "", now_ms));
            }
        }
        self.save_vault(&changes)?;
        Ok(self.settings())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;

    #[test]
    fn a_setting_is_shared_unless_this_device_overrides_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        assert_eq!(vault.settings(), Settings {
            theme: "system".into(), font_size: 17, theme_here: false, font_size_here: false,
            line_numbers: false, line_numbers_here: false, highlight_line: false, highlight_line_here: false,
            preview_follows: true, preview_follows_here: false,
            comment_highlights: true, comment_highlights_here: false,
            shortcuts: BTreeMap::new(), shortcuts_here: false, hidden_folders_here: false,
        });
        vault.set_setting("commentHighlights", "off", false, 400).unwrap();
        assert!(!vault.settings().comment_highlights);
        vault.set_setting("lineNumbers", "on", true, 500).unwrap();
        assert!(vault.settings().line_numbers && vault.settings().line_numbers_here);
        assert!(vault.set_setting("highlightLine", "maybe", false, 500).is_err());
        vault.set_setting("previewFollows", "off", true, 500).unwrap();
        assert!(!vault.settings().preview_follows && vault.settings().preview_follows_here);
        vault.set_setting("theme", "dark", false, 1000).unwrap();
        vault.set_setting("fontSize", "20", true, 2000).unwrap();
        let settings = vault.settings();
        assert_eq!((settings.theme.as_str(), settings.theme_here), ("dark", false));
        assert_eq!((settings.font_size, settings.font_size_here), (20, true));
        // Another Device sees the shared theme but not this one's font size.
        let Opened::Vault(other) = Vault::open(dir.path(), DeviceId(2), false, &dir.path().join(".recovery")).unwrap() else { panic!() };
        assert_eq!((other.settings().theme.as_str(), other.settings().font_size), ("dark", 17));
        // Setting it for every Device drops the override here.
        vault.set_setting("fontSize", "18", false, 3000).unwrap();
        assert_eq!((vault.settings().font_size, vault.settings().font_size_here), (18, false));
    }

    #[test]
    fn only_known_settings_and_sensible_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        assert!(vault.set_setting("theme", "purple", false, 1000).is_err());
        assert!(vault.set_setting("theme", "vault:../vault.db", false, 1000).is_err());
        vault.set_setting("theme", "vault:sepia.css", true, 1000).unwrap();
        assert_eq!(vault.settings().theme, "vault:sepia.css");
        assert!(vault.set_setting("fontSize", "3", false, 1000).is_err());
        assert!(vault.set_setting("colour", "red", false, 1000).is_err());
    }

    #[test]
    fn shortcuts_are_shared_unless_this_device_has_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        let keys = |pairs: &[(&str, &str)]| pairs.iter().map(|(c, k)| (c.to_string(), k.to_string())).collect::<Vec<_>>();
        let settings = vault.set_shortcuts(&keys(&[("addComment", "Mod-Shift-c"), ("strong", "none")]), 1000).unwrap();
        assert_eq!(settings.shortcuts, BTreeMap::from([("addComment".into(), "Mod-Shift-c".into()), ("strong".into(), "none".into())]));
        // Another Device has the same.
        let Opened::Vault(mut other) = Vault::open(dir.path(), DeviceId(2), false, &dir.path().join(".recovery2")).unwrap() else { panic!() };
        assert_eq!(other.settings().shortcuts.len(), 2);
        // It takes its own: they start as a copy, and change only there.
        let settings = other.set_shortcuts_here(true, 2000).unwrap();
        assert!(settings.shortcuts_here);
        assert_eq!(settings.shortcuts.len(), 2);
        other.set_shortcuts(&keys(&[("addComment", "Mod-Shift-n")]), 3000).unwrap();
        vault.absorb_vault_copies(&mut Vec::new());
        assert_eq!(vault.settings().shortcuts["addComment"], "Mod-Shift-c", "the Vault's are unchanged");
        // Back to the Vault's; then the Vault's are reset to the defaults.
        assert_eq!(other.set_shortcuts_here(false, 4000).unwrap().shortcuts["addComment"], "Mod-Shift-c");
        assert!(vault.reset_shortcuts(5000).unwrap().shortcuts.is_empty());
        assert!(vault.set_shortcuts(&keys(&[("add comment", "Mod-m")]), 6000).is_err());
        assert!(vault.set_shortcuts(&keys(&[("strong", "Mod b")]), 6000).is_err());
    }
}
