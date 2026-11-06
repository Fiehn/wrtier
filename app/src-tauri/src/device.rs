//! Device state: what this Device remembers only for itself and never syncs. It lives in the
//! app's local data folder (not the roaming one on Windows, which could carry the Device ID to
//! another computer), never in the Vault.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wrtier::ids::DeviceId;

const FILE: &str = "device.json";

pub struct DeviceState {
    dir: PathBuf,
    pub device: DeviceId,
    pub last_vault: Option<PathBuf>,
}

/// How it's stored: plain JSON, so it can be read and fixed by hand.
#[derive(Serialize, Deserialize)]
struct Stored {
    device_id: String,
    last_vault: Option<PathBuf>,
}

impl DeviceState {
    /// Reads this Device's state, or starts it (with a new random Device ID) on first launch.
    /// A damaged file is kept next to it and replaced: a new Device ID only makes this a new
    /// Device, which loses nothing.
    pub fn load_or_create(dir: &Path) -> Result<Self, String> {
        let path = dir.join(FILE);
        if let Ok(json) = std::fs::read_to_string(&path) {
            let parsed = serde_json::from_str::<Stored>(&json).map_err(|e| e.to_string())
                .and_then(|s| Ok((s.device_id.parse::<DeviceId>()?, s.last_vault)));
            match parsed {
                Ok((device, last_vault)) => return Ok(DeviceState { dir: dir.to_path_buf(), device, last_vault }),
                Err(_) => {
                    let _ = std::fs::rename(&path, dir.join(format!("{FILE}.damaged")));
                }
            }
        }
        let state = DeviceState { dir: dir.to_path_buf(), device: new_device_id()?, last_vault: None };
        state.save()?;
        Ok(state)
    }

    /// Written next to itself and renamed over the old file, so a crash never leaves half of it.
    pub fn save(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let stored = Stored { device_id: self.device.to_string(), last_vault: self.last_vault.clone() };
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
    fn a_damaged_file_is_kept_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "{ not json").unwrap();
        let state = DeviceState::load_or_create(dir.path()).unwrap();
        assert!(dir.path().join("device.json.damaged").exists());
        assert_eq!(DeviceState::load_or_create(dir.path()).unwrap().device, state.device);
    }
}
