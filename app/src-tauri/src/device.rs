//! Device state: what this Device keeps only for itself and never syncs. It lives in the
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
    /// The Vaults this Device has opened, the most recently opened first (CONTEXT.md, Known
    /// Vaults). The first is opened at startup.
    pub known_vaults: Vec<KnownVault>,
    /// The tabs open in each Vault (by its folder), to open again next time.
    pub layouts: HashMap<String, Layout>,
    /// Where the window was, and how big.
    pub window: Option<WindowPlace>,
    /// Syncthing's address and API key, if set by hand (on Android, or when its settings file
    /// isn't in a usual place). Otherwise the app reads them from Syncthing's settings file.
    pub syncthing: Option<crate::syncthing::Connection>,
    /// The window has the system's own frame (title bar and borders) instead of the app's. Read
    /// when the window opens, so a change shows from the next start. Desktop only.
    pub native_frame: bool,
}

/// A Known Vault: its folder on this Device, and its Vault name as last seen (ADR 0022), so the
/// list shows names without reading every Vault's `vault.db`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KnownVault {
    pub path: PathBuf,
    pub name: String,
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
    /// The Work in the Reading panel, if one is being read.
    #[serde(default)]
    pub reading: Option<ReadingPlace>,
}

/// The Work being read in the Reading panel, and where it's scrolled to.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReadingPlace {
    /// Its ID in the Vault when last seen.
    pub work: String,
    /// Its Work ID, which follows it through renames and moves (none for a Text file only).
    pub identity: Option<String>,
    /// The first position shown (UTF-16).
    pub top: usize,
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
    #[serde(default)]
    known_vaults: Vec<KnownVault>,
    #[serde(default)]
    layouts: HashMap<String, Layout>,
    #[serde(default)]
    window: Option<WindowPlace>,
    #[serde(default)]
    syncthing: Option<crate::syncthing::Connection>,
    #[serde(default)]
    native_frame: bool,
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
                    return Ok(DeviceState { dir: dir.to_path_buf(), device, known_vaults: s.known_vaults, layouts: s.layouts, window: s.window, syncthing: s.syncthing, native_frame: s.native_frame });
                }
                Err(_) => {
                    let _ = std::fs::rename(&path, dir.join(format!("{FILE}.damaged")));
                }
            }
        }
        let state = DeviceState { dir: dir.to_path_buf(), device: new_device_id()?, known_vaults: Vec::new(), layouts: HashMap::new(), window: None, syncthing: None, native_frame: false };
        state.save()?;
        Ok(state)
    }

    /// Written next to itself and renamed over the old file, so a crash never leaves half of it.
    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let stored = Stored {
            device_id: self.device.to_string(),
            known_vaults: self.known_vaults.clone(),
            layouts: self.layouts.clone(),
            window: self.window,
            syncthing: self.syncthing.clone(),
            native_frame: self.native_frame,
        };
        let json = serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?;
        let (path, partial) = (self.dir.join(FILE), self.dir.join(format!("{FILE}.writing")));
        std::fs::write(&partial, json).map_err(|e| e.to_string())?;
        std::fs::rename(&partial, &path).map_err(|e| e.to_string())
    }
}

impl DeviceState {
    /// The Vault opened last: the one to open at startup.
    pub fn last_vault(&self) -> Option<&Path> {
        self.known_vaults.first().map(|k| k.path.as_path())
    }

    /// A Vault was opened: first on the list, with its name as it is now.
    pub fn remember_vault(&mut self, path: &Path, name: &str) {
        self.known_vaults.retain(|k| k.path != path);
        self.known_vaults.insert(0, KnownVault { path: path.to_path_buf(), name: name.to_string() });
    }

    /// Takes a Vault off this Device's list; nothing in it is touched.
    pub fn forget_vault(&mut self, path: &Path) {
        self.known_vaults.retain(|k| k.path != path);
        self.layouts.remove(&path.display().to_string());
    }

    /// A Vault was renamed: its new name, in its place on the list.
    pub fn rename_known(&mut self, path: &Path, name: &str) {
        if let Some(k) = self.known_vaults.iter_mut().find(|k| k.path == path) {
            k.name = name.to_string();
        }
    }

    /// A Vault's folder is somewhere else now (moved, or located): it keeps its place on the
    /// list and its tabs.
    pub fn relocate_vault(&mut self, from: &Path, to: &Path) {
        if let Some(k) = self.known_vaults.iter_mut().find(|k| k.path == from) {
            k.path = to.to_path_buf();
        }
        if let Some(layout) = self.layouts.remove(&from.display().to_string()) {
            self.layouts.insert(to.display().to_string(), layout);
        }
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
    fn known_vaults_are_remembered_the_last_opened_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = DeviceState::load_or_create(dir.path()).unwrap();
        state.remember_vault(Path::new("C:/Writing"), "Writing");
        state.remember_vault(Path::new("D:/Stories"), "Short stories");
        state.remember_vault(Path::new("C:/Writing"), "My novels");
        state.save().unwrap();
        let again = DeviceState::load_or_create(dir.path()).unwrap();
        let names: Vec<&str> = again.known_vaults.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["My novels", "Short stories"], "opened again: first, once, with its name as it is now");
        assert_eq!(again.last_vault(), Some(Path::new("C:/Writing")));
    }

    #[test]
    fn a_moved_vault_keeps_its_place_and_its_tabs_and_a_forgotten_one_goes() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = DeviceState::load_or_create(dir.path()).unwrap();
        state.remember_vault(Path::new("C:/Old"), "Old drafts");
        state.remember_vault(Path::new("C:/Writing"), "Writing");
        state.layouts.insert("C:/Writing".into(), Layout { tabs: vec!["Rain".into()], ..Layout::default() });
        state.relocate_vault(Path::new("C:/Writing"), Path::new("D:/Writing"));
        assert_eq!(state.last_vault(), Some(Path::new("D:/Writing")));
        assert_eq!(state.layouts["D:/Writing"].tabs, ["Rain"]);
        state.rename_known(Path::new("C:/Old"), "Older drafts");
        assert_eq!(state.known_vaults[1].name, "Older drafts");
        state.forget_vault(Path::new("C:/Old"));
        assert_eq!(state.known_vaults.len(), 1);
    }

    #[test]
    fn tabs_and_the_window_are_remembered_and_older_files_still_read() {
        let dir = tempfile::tempdir().unwrap();
        // A file from before tabs were remembered.
        std::fs::write(dir.path().join(FILE), r#"{ "device_id": "0000002a" }"#).unwrap();
        let mut state = DeviceState::load_or_create(dir.path()).unwrap();
        assert_eq!(state.device, DeviceId(42));
        assert!(!state.native_frame, "the app's own frame unless chosen otherwise");
        let layout = Layout { tabs: vec!["Rain".into()], active: Some("Rain".into()), cursors: [("Rain".to_string(), 12)].into(), reading: Some(ReadingPlace { work: "Ash".into(), identity: Some("ab12".into()), top: 40 }) };
        state.layouts.insert("C:/Writing".into(), layout.clone());
        state.window = Some(WindowPlace { x: 10, y: 20, width: 1200, height: 800, maximized: false });
        state.native_frame = true;
        state.save().unwrap();
        let again = DeviceState::load_or_create(dir.path()).unwrap();
        assert_eq!(again.layouts["C:/Writing"], layout);
        assert_eq!(again.window, state.window);
        assert!(again.native_frame);
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
