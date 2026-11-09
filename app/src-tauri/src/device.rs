//! Device state: what this Device remembers only for itself and never syncs. It lives in the
//! app's local data folder (not the roaming one on Windows, which could carry the Device ID to
//! another computer), never in the Vault.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wrtier::ids::DeviceId;

const FILE: &str = "device.json";

pub struct DeviceState {
    dir: PathBuf,
    pub device: DeviceId,
    pub last_vault: Option<PathBuf>,
    /// The tabs open in each Vault (by its folder), to open again next time.
    pub layouts: HashMap<String, Layout>,
    /// Where the window was, and how big.
    pub window: Option<WindowPlace>,
    /// Syncthing's address and API key, if set by hand (on Android, or when its settings file
    /// isn't in a usual place). Otherwise the app reads them from Syncthing's settings file.
    pub syncthing: Option<crate::syncthing::Connection>,
}

/// The tabs open in a Vault, which one is in front, and where the cursor was in each Work.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub tabs: Vec<String>,
    pub active: Option<String>,
    /// Work ID → cursor position (UTF-16, as the editor counts).
    #[serde(default)]
    pub cursors: HashMap<String, usize>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WindowPlace {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// How it's stored: plain JSON, so it can be read and fixed by hand. (Fields added later have
/// defaults, so a file from an older version still reads.)
#[derive(Serialize, Deserialize)]
struct Stored {
    device_id: String,
    last_vault: Option<PathBuf>,
    #[serde(default)]
    layouts: HashMap<String, Layout>,
    #[serde(default)]
    window: Option<WindowPlace>,
    #[serde(default)]
    syncthing: Option<crate::syncthing::Connection>,
}

impl DeviceState {
    /// Reads this Device's state, or starts it (with a new random Device ID) on first launch.
    /// A damaged file is kept next to it and replaced: a new Device ID only makes this a new
    /// Device, which loses nothing.
    pub fn load_or_create(dir: &Path) -> Result<Self, String> {
        let path = dir.join(FILE);
        if let Ok(json) = std::fs::read_to_string(&path) {
            let parsed = serde_json::from_str::<Stored>(&json).map_err(|e| e.to_string())
                .and_then(|s| Ok((s.device_id.parse::<DeviceId>()?, s)));
            match parsed {
                Ok((device, s)) => {
                    return Ok(DeviceState { dir: dir.to_path_buf(), device, last_vault: s.last_vault, layouts: s.layouts, window: s.window, syncthing: s.syncthing });
                }
                Err(_) => {
                    let _ = std::fs::rename(&path, dir.join(format!("{FILE}.damaged")));
                }
            }
        }
        let state = DeviceState { dir: dir.to_path_buf(), device: new_device_id()?, last_vault: None, layouts: HashMap::new(), window: None, syncthing: None };
        state.save()?;
        Ok(state)
    }

    /// Written next to itself and renamed over the old file, so a crash never leaves half of it.
    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let stored = Stored {
            device_id: self.device.to_string(),
            last_vault: self.last_vault.clone(),
            layouts: self.layouts.clone(),
            window: self.window,
            syncthing: self.syncthing.clone(),
        };
        let json = serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?;
        let (path, partial) = (self.dir.join(FILE), self.dir.join(format!("{FILE}.writing")));
        std::fs::write(&partial, json).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &path).map_err(|e| e.to_string())
    }
}

fn new_device_id() -> Result<DeviceId, String> {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).map_err(|e| format!("couldn't make a Device ID: {e}"))?;
    Ok(DeviceId(u32::from_le_bytes(bytes)))
}

/// A first suggestion for this Device's name: the computer's name.
pub fn suggested_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "This device".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_id_is_made_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let first = DeviceState::load_or_create(dir.path()).unwrap();
        let again = DeviceState::load_or_create(dir.path()).unwrap();
        assert_eq!(first.device, again.device);
    }

    #[test]
    fn the_last_vault_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = DeviceState::load_or_create(dir.path()).unwrap();
        state.last_vault = Some(PathBuf::from("C:/Writing"));
        state.save().unwrap();
        assert_eq!(DeviceState::load_or_create(dir.path()).unwrap().last_vault, Some(PathBuf::from("C:/Writing")));
    }

    #[test]
    fn tabs_and_the_window_are_remembered_and_older_files_still_read() {
        let dir = tempfile::tempdir().unwrap();
        // A file from before tabs were remembered.
        std::fs::write(dir.path().join(FILE), r#"{ "device_id": "0000002a", "last_vault": null }"#).unwrap();
        let mut state = DeviceState::load_or_create(dir.path()).unwrap();
        assert_eq!(state.device, DeviceId(42));
        let layout = Layout { tabs: vec!["Rain".into()], active: Some("Rain".into()), cursors: [("Rain".to_string(), 12)].into() };
        state.layouts.insert("C:/Writing".into(), layout.clone());
        state.window = Some(WindowPlace { x: 10, y: 20, width: 1200, height: 800, maximized: false });
        state.save().unwrap();
        let again = DeviceState::load_or_create(dir.path()).unwrap();
        assert_eq!(again.layouts["C:/Writing"], layout);
        assert_eq!(again.window, state.window);
    }

    #[test]
    fn a_damaged_file_is_kept_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "{ not json").unwrap();
        let state = DeviceState::load_or_create(dir.path()).unwrap();
        assert!(dir.path().join("device.json.damaged").exists());
        assert_eq!(DeviceState::load_or_create(dir.path()).unwrap().device, state.device);
    }
}
