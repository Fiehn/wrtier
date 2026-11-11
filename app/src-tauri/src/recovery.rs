//! Recovery files: what an open Work holds in memory only (the open burst, a cut waiting for
//! its paste, Changes that failed to save), kept on this Device outside the Vault so that a
//! crash loses nothing. Updated after every edit, flushed to disk about once a second, and
//! removed once everything in it is safely in the `.db`.
//!
//! Each Work has up to two files: `<key>.json` (the burst and what the file is for) and
//! `<key>.db` (unsaved Changes, as a small History that merges into the Work's own).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use wrtier::change::Change;
use wrtier::doc::{Burst, BurstKind, Doc};
use wrtier::ids::{ChangeId, CharId};
use wrtier::storage;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// What the `.json` holds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Kept {
    /// The Vault's folder and the Work's ID in it: which Work this is for.
    pub vault: PathBuf,
    pub work: String,
    pub burst: Option<KeptBurst>,
    /// The burst is a cut waiting for its paste.
    pub cut: bool,
}

/// An open burst, as plain text (IDs in their readable form).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeptBurst {
    pub deleting: bool,
    pub id: String,
    pub left: Option<String>,
    /// The characters it hides, as runs of consecutive ones: (first, how many).
    pub replacing: Vec<(String, u32)>,
    pub draft: String,
    pub opened_ms: u64,
    pub last_ms: u64,
}

impl KeptBurst {
    pub fn of(burst: &Burst) -> Self {
        let mut replacing: Vec<(CharId, u32)> = Vec::new();
        for &c in &burst.replacing {
            match replacing.last_mut() {
                Some((first, n)) if first.change == c.change && first.offset + *n == c.offset => *n += 1,
                _ => replacing.push((c, 1)),
            }
        }
        KeptBurst {
            deleting: burst.kind == BurstKind::Deleting,
            id: burst.id.to_string(),
            left: burst.left.map(|c| c.to_string()),
            replacing: replacing.into_iter().map(|(c, n)| (c.to_string(), n)).collect(),
            draft: burst.draft.clone(),
            opened_ms: burst.opened_ms,
            last_ms: burst.last_ms,
        }
    }

    /// Opens it again in `doc`. False if it can't be (see `Doc::resume_burst`).
    pub fn resume(&self, doc: &mut Doc) -> Result<bool, String> {
        let id: ChangeId = self.id.parse()?;
        let left = self.left.as_deref().map(str::parse::<CharId>).transpose()?;
        let mut replacing = Vec::new();
        for (first, n) in &self.replacing {
            let first: CharId = first.parse()?;
            replacing.extend((0..*n).map(|i| CharId { change: first.change, offset: first.offset + i }));
        }
        let kind = if self.deleting { BurstKind::Deleting } else { BurstKind::Typing };
        Ok(doc.resume_burst(kind, id, left, replacing, self.draft.clone(), self.opened_ms, self.last_ms))
    }
}

/// One Work's recovery files.
pub struct Recovery {
    pub json: PathBuf,
    pub db: PathBuf,
    vault: PathBuf,
    work: String,
    /// What's on disk now, so an unchanged state isn't written again.
    written: Option<String>,
    written_changes: usize,
    unflushed: bool,
}

/// A file name that stays the same across versions of the app (FNV-1a of the Vault and Work).
fn key(vault: &Path, work: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in vault.to_string_lossy().as_bytes().iter().chain(b"\n").chain(work.as_bytes()) {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

impl Recovery {
    pub fn for_work(dir: &Path, vault: &Path, work: &str) -> Self {
        let key = key(vault, work);
        Recovery {
            json: dir.join(format!("{key}.json")),
            db: dir.join(format!("{key}.db")),
            vault: vault.to_path_buf(),
            work: work.to_string(),
            written: None,
            written_changes: 0,
            unflushed: false,
        }
    }

    /// What was kept for this Work, if anything.
    pub fn read(&self) -> Result<Option<Kept>, String> {
        match std::fs::read_to_string(&self.json) {
            Ok(json) => Ok(Some(serde_json::from_str(&json).map_err(err)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Keeps what the Work holds in memory only. Cheap when nothing changed.
    pub fn keep(&mut self, doc: &Doc, cut: bool, unsaved: &[Change]) -> Result<(), String> {
        let burst = doc.burst();
        if burst.is_none() && unsaved.is_empty() {
            return self.clear();
        }
        if unsaved.len() != self.written_changes {
            let _ = std::fs::remove_file(&self.db);
            if !unsaved.is_empty() {
                std::fs::create_dir_all(self.db.parent().expect("in a folder")).map_err(err)?;
                storage::save(&self.db, unsaved, doc).map_err(err)?;
            }
            self.written_changes = unsaved.len();
            self.unflushed = true;
        }
        let kept = Kept { vault: self.vault.clone(), work: self.work.clone(), burst: burst.map(KeptBurst::of), cut };
        let json = serde_json::to_string(&kept).map_err(err)?;
        if self.written.as_deref() != Some(json.as_str()) {
            std::fs::create_dir_all(self.json.parent().expect("in a folder")).map_err(err)?;
            let partial = self.json.with_extension("json.writing");
            std::fs::write(&partial, &json).map_err(err)?;
            std::fs::rename(&partial, &self.json).map_err(err)?;
            self.written = Some(json);
            self.unflushed = true;
        }
        Ok(())
    }

    /// Makes sure what was kept is on the disk itself, not only in the system's cache, so it
    /// survives the whole computer crashing. (About once a second: doing it on every keystroke
    /// would slow typing down.)
    pub fn flush(&mut self) -> Result<(), String> {
        if !self.unflushed {
            return Ok(());
        }
        for path in [&self.json, &self.db] {
            if path.exists() {
                std::fs::OpenOptions::new().write(true).open(path).and_then(|f| f.sync_all()).map_err(err)?;
            }
        }
        self.unflushed = false;
        Ok(())
    }

    /// Everything is in the `.db`: nothing needs keeping.
    pub fn clear(&mut self) -> Result<(), String> {
        if self.written.is_none() && self.written_changes == 0 && !self.json.exists() && !self.db.exists() {
            return Ok(());
        }
        for path in [&self.json, &self.db] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        self.written = None;
        self.written_changes = 0;
        self.unflushed = false;
        Ok(())
    }
}

/// The Works in `vault` with recovery files in `dir`.
pub fn kept_for(dir: &Path, vault: &Path) -> Vec<String> {
    kept_all(dir).into_iter().filter(|(v, _)| v == vault).map(|(_, work)| work).collect()
}

/// Every Work, in any Vault, with recovery files in `dir`: its Vault's folder and its ID,
/// sorted. (Uninstalling names them, since their writing would be lost.)
pub fn kept_all(dir: &Path) -> Vec<(PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut works: Vec<(PathBuf, String)> = entries.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str::<Kept>(&std::fs::read_to_string(e.path()).ok()?).ok())
        .map(|kept| (kept.vault, kept.work))
        .collect();
    works.sort();
    works
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrtier::ids::DeviceId;

    #[test]
    fn a_burst_is_kept_and_resumed() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Doc::new(DeviceId(1));
        let saved = vec![doc.insert_local(0, "The rain had stopped.", 1000)];
        doc.replace_text(13, 7, "eased", 2000);
        let mut recovery = Recovery::for_work(dir.path(), Path::new("C:/Vault"), "Rain");
        recovery.keep(&doc, false, &[]).unwrap();
        recovery.flush().unwrap();

        let kept = Recovery::for_work(dir.path(), Path::new("C:/Vault"), "Rain").read().unwrap().unwrap();
        assert_eq!(kept.work, "Rain");
        assert_eq!(kept.burst.as_ref().unwrap().replacing.len(), 1, "one run for the selection");
        let mut again = Doc::new(DeviceId(1));
        again.apply_all(&saved);
        assert!(kept.burst.unwrap().resume(&mut again).unwrap());
        assert_eq!(again.text(), "The rain had eased.");
        assert_eq!(kept_for(dir.path(), Path::new("C:/Vault")), vec!["Rain".to_string()]);
        assert!(kept_for(dir.path(), Path::new("C:/Other")).is_empty());
        assert_eq!(kept_all(dir.path()), vec![(PathBuf::from("C:/Vault"), "Rain".to_string())]);
    }

    #[test]
    fn nothing_is_kept_when_everything_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Doc::new(DeviceId(1));
        doc.insert_local(0, "abc", 1000);
        doc.type_text(3, "d", 2000);
        let mut recovery = Recovery::for_work(dir.path(), Path::new("C:/Vault"), "Rain");
        recovery.keep(&doc, false, &[]).unwrap();
        assert!(recovery.json.exists());
        doc.close_burst(3000);
        recovery.keep(&doc, false, &[]).unwrap();
        assert!(!recovery.json.exists() && !recovery.db.exists());
    }

    #[test]
    fn unsaved_changes_are_kept_as_a_small_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Doc::new(DeviceId(1));
        let change = doc.insert_local(0, "abc", 1000);
        let mut recovery = Recovery::for_work(dir.path(), Path::new("C:/Vault"), "Rain");
        recovery.keep(&doc, false, std::slice::from_ref(&change)).unwrap();
        assert_eq!(storage::read_all(&recovery.db).unwrap(), vec![change]);
    }
}
