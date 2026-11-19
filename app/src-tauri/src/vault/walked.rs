//! What's in the Vault and its trash, found by walking their folders, and kept while nothing in
//! them changed: Inclusions look their Works up every few seconds per tab, a lost tab looks for
//! its History every second, and the Works list every ten, so walking every time adds up.
//!
//! A folder's modification time changes whenever a file in it is added, removed or renamed, so
//! comparing the walked folders' times says whether the walk still holds. Some file systems
//! (FAT on a memory card) don't keep those times well, so a walk is also trusted only for a
//! few seconds.

use super::inclusions::{relative_id, Index, Located};
use super::{walk_works, Files, Vault};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// How long a walk is trusted even though no folder's time changed.
const TRUSTED_FOR: Duration = Duration::from_secs(5);

pub(super) fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// One walk of the Vault and its trash.
pub(super) struct Walked {
    pub vault: Files,
    pub trash: Files,
    at: Instant,
    /// The Work index made from it, once asked for, and the `.db`s that had no Work ID then,
    /// with their times: an empty Work gets its ID with its first Change, which changes the
    /// file but not its folder.
    index: Option<(Arc<Index>, Vec<(PathBuf, Option<SystemTime>)>)>,
}

impl Walked {
    fn new(root: &Path, trash: &Path) -> Walked {
        Walked { vault: walk_works(root), trash: walk_works(trash), at: Instant::now(), index: None }
    }

    /// Does it still hold: no walked folder changed, and it isn't old?
    fn current(&self) -> bool {
        self.at.elapsed() < TRUSTED_FOR
            && self.vault.stamps.iter().chain(&self.trash.stamps).all(|(dir, at)| modified(dir) == *at)
    }
}

impl Vault {
    /// What's in the Vault and its trash, walked again only when something there changed.
    pub(super) fn walked(&mut self) -> &Walked {
        if !self.walked.as_ref().is_some_and(Walked::current) {
            self.walked = Some(Walked::new(&self.root, &self.trash_root()));
        }
        self.walked.as_ref().expect("just made")
    }

    /// The Vault's files, walked just now (for the Works list, which must be exact).
    pub(super) fn walk(&mut self) -> Files {
        self.walked = None;
        self.walked().vault.clone()
    }

    /// Every Work in the Vault and its trash, by its Work ID. Made again only when the walk is,
    /// or a Work that had no ID got one.
    pub(super) fn work_index(&mut self) -> Arc<Index> {
        self.walked();
        let walked = self.walked.as_ref().expect("walked above");
        if let Some((index, unnamed)) = &walked.index {
            if unnamed.iter().all(|(db, at)| modified(db) == *at) {
                return index.clone();
            }
        }
        let trash = self.trash_root();
        let dbs: Vec<(PathBuf, bool)> = walked.vault.dbs.iter().map(|d| (d.clone(), false))
            .chain(walked.trash.dbs.iter().map(|d| (d.clone(), true)))
            .collect();
        let mut index: Index = HashMap::new();
        let mut seen = HashSet::new();
        let mut unnamed = Vec::new();
        for (db, trashed) in dbs {
            seen.insert(db.clone());
            let Some(identity) = self.identity_of(&db) else {
                unnamed.push((db.clone(), modified(&db)));
                continue;
            };
            let id = if trashed { relative_id(&trash, &db) } else { self.id_of(&db) };
            match index.get_mut(&identity) {
                // The Vault's own copy wins over one in the trash; two in the Vault are a problem.
                Some(found) if found.trashed && !trashed => *found = Located { db, id, trashed, duplicate: false },
                Some(found) if found.trashed == trashed => found.duplicate = true,
                Some(_) => {}
                None => {
                    index.insert(identity, Located { db, id, trashed, duplicate: false });
                }
            }
        }
        // Forget the IDs of files that aren't there any more.
        self.identities.retain(|db, _| seen.contains(db));
        let index = Arc::new(index);
        if let Some(walked) = self.walked.as_mut() {
            walked.index = Some((index.clone(), unnamed));
        }
        index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::tests_support::open;

    #[test]
    fn the_index_is_kept_until_a_folder_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![crate::work::Op::Type { from: 0, to: 0, text: "Wet.".into(), origin: crate::work::Origin::Input }], 2000).unwrap();
        vault.close_work("Rain", 3000);
        let first = vault.work_index();
        assert_eq!(first.len(), 1);
        assert!(Arc::ptr_eq(&first, &vault.work_index()), "nothing changed: not walked again");

        std::fs::create_dir(dir.path().join("Old")).unwrap();
        std::fs::rename(dir.path().join("Rain.db"), dir.path().join("Old/Rain.db")).unwrap();
        let moved = vault.work_index();
        assert!(!Arc::ptr_eq(&first, &moved), "a folder changed: walked again");
        assert_eq!(moved.values().next().unwrap().id, "Old/Rain");
    }
}
