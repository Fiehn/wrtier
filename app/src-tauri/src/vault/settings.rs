//! Vault settings: preferences kept in `vault.db` and shared by every Device, such as the app
//! theme and the editor font size. Any Device can override one for itself; the override is
//! kept in the same file, under that Device's record, so it syncs and survives a reinstall.

use super::*;
use wrtier::change::device_record;

/// The record holding the settings every Device shares.
const SHARED: &str = "settings";

/// The settings this Device uses, and which of them are its own overrides.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// "system", "light" or "dark".
    pub theme: String,
    /// The editor's font size in pixels.
    pub font_size: u32,
    pub theme_here: bool,
    pub font_size_here: bool,
}

/// The settings there are, as stored, with their default and what's allowed.
fn check(name: &str, value: &str) -> Result<(&'static str, String), String> {
    match name {
        "theme" if ["system", "light", "dark"].contains(&value) => Ok(("theme", value.to_string())),
        "fontSize" => match value.parse::<u32>() {
            Ok(size) if (10..=40).contains(&size) => Ok(("font_size", size.to_string())),
            _ => Err("The font size must be a whole number from 10 to 40.".into()),
        },
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
        Settings { theme, font_size: font_size.parse().unwrap_or(17), theme_here, font_size_here }
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
        assert_eq!(vault.settings(), Settings { theme: "system".into(), font_size: 17, theme_here: false, font_size_here: false });
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
        assert!(vault.set_setting("fontSize", "3", false, 1000).is_err());
        assert!(vault.set_setting("colour", "red", false, 1000).is_err());
    }
}
