//! Renaming and trashing Works, and following an open Work whose History moved underneath it
//! (renamed or trashed on another Device).
//!
//! A deleted Work goes to `.trash/` inside the Vault: hidden from the Vault browser, but synced,
//! so it can be restored from any Device. A Work is known by its first Change, which never
//! changes, so an open tab finds its History again wherever it went.

use super::*;

/// The Vault's trash folder.
pub const TRASH: &str = ".trash";

/// A Work in the trash.
#[derive(Serialize, Debug, PartialEq)]
pub struct TrashedWork {
    /// Its path inside the trash, without the extension (where it goes back to when restored).
    pub id: String,
    pub name: String,
}

/// A Work's files, from its `.db`: the History, the Text file, and the images folder.
fn work_files(db: &Path) -> [PathBuf; 3] {
    let stem = db.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    [db.to_path_buf(), db.with_file_name(format!("{stem}.typ")), db.with_file_name(format!("{stem}.images"))]
}

/// Moves a Work's files from `from` to `to` (both `.db` paths), making folders as needed. The
/// History goes first: if anything stops halfway, the Text file left behind is recognised as
/// a leftover of the Work at its new place.
fn move_work(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    for (a, b) in work_files(from).iter().zip(work_files(to).iter()) {
        if a.exists() {
            std::fs::rename(a, b).map_err(err)?;
        }
    }
    Ok(())
}

/// `id`, or `id (2)`, `id (3)`… : the first that no Work under `base` has.
fn free_id(base: &Path, id: &str) -> Result<String, String> {
    for n in 1.. {
        let candidate = if n == 1 { id.to_string() } else { format!("{id} ({n})") };
        let (db, typ) = paths_under(base, &candidate)?;
        if !db.exists() && !typ.exists() {
            return Ok(candidate);
        }
    }
    unreachable!("some number is free")
}

/// A folder's path under `base` from its ID ("Short stories/Old"), staying inside `base`.
pub(super) fn folder_under(base: &Path, path: &str) -> Result<PathBuf, String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.contains(['\\', ':'])) {
        return Err(format!("not a folder in this Vault: {path:?}"));
    }
    Ok(parts.iter().fold(base.to_path_buf(), |dir, part| dir.join(part)))
}

/// `path`, or `path (2)`, `path (3)`… : the first that isn't a folder under `base` yet.
fn free_folder(base: &Path, path: &str) -> Result<String, String> {
    for n in 1.. {
        let candidate = if n == 1 { path.to_string() } else { format!("{path} ({n})") };
        if !folder_under(base, &candidate)?.exists() {
            return Ok(candidate);
        }
    }
    unreachable!("some number is free")
}

impl Vault {
    pub(super) fn trash_root(&self) -> PathBuf {
        self.root.join(TRASH)
    }

    /// Checks a folder name the Author typed (each part must be a valid folder name everywhere,
    /// and not one the Vault browser skips). Returns it tidied.
    pub(super) fn checked_folder(&self, path: &str) -> Result<String, String> {
        let parts: Vec<&str> = path.split('/').map(str::trim).collect();
        for (i, part) in parts.iter().enumerate() {
            check_name(part, i == 0, false).map_err(|e| e.replace("A Work needs", "A folder needs"))?;
        }
        Ok(parts.join("/"))
    }

    /// Makes a new, empty folder for Works. Returns its path.
    pub fn create_folder(&mut self, path: &str) -> Result<String, String> {
        let path = self.checked_folder(path)?;
        let dir = folder_under(&self.root, &path)?;
        if dir.exists() {
            return Err(format!("There's already a folder called {path}."));
        }
        std::fs::create_dir_all(&dir).map_err(err)?;
        Ok(path)
    }

    /// The open Works inside a folder.
    fn open_inside(&self, folder: &str) -> Vec<String> {
        let prefix = format!("{folder}/");
        self.works.keys().filter(|id| id.starts_with(&prefix)).cloned().collect()
    }

    /// Renames (or moves) a folder with everything in it. Works open in it keep their tabs, and
    /// Hidden folders at or inside it stay hidden. Returns, for each open Work, its old and new ID.
    pub fn rename_folder(&mut self, path: &str, new_path: &str, now_ms: u64) -> Result<Vec<(String, String)>, String> {
        let from = folder_under(&self.root, path)?;
        if !from.is_dir() {
            return Err(format!("{path} isn't a folder in the Vault anymore."));
        }
        let new_path = self.checked_folder(new_path)?;
        if new_path == path {
            return Ok(Vec::new());
        }
        let to = folder_under(&self.root, &new_path)?;
        if to.exists() {
            return Err(format!("There's already a folder called {new_path}."));
        }
        if to.starts_with(&from) {
            return Err("A folder can't go inside itself.".into());
        }
        let open = self.open_inside(path);
        for id in &open {
            if let Some(work) = self.works.get_mut(id) {
                work.save();
            }
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        std::fs::rename(&from, &to).map_err(err)?;
        let mut moved = Vec::new();
        for id in open {
            let new_id = format!("{new_path}/{}", &id[path.len() + 1..]);
            let (db, typ) = self.paths(&new_id)?;
            let mut work = self.works.remove(&id).expect("listed above");
            work.relocate(db, typ, Recovery::for_work(&self.recovery_dir, &self.root, &new_id));
            self.works.insert(new_id.clone(), work);
            self.aliases.insert(id.clone(), new_id.clone());
            moved.push((id, new_id));
        }
        // The folder has moved already: if the mark can't be saved, it shows again, which is
        // the safe way round (nothing is hidden by mistake).
        let _ = self.hidden_folders_follow(path, &new_path, now_ms);
        Ok(moved)
    }

    /// Moves a folder with everything in it to the trash (closing any Works open in it first).
    /// Its Works can then be restored one by one.
    pub fn trash_folder(&mut self, path: &str, now_ms: u64) -> Result<(), String> {
        let from = folder_under(&self.root, path)?;
        if !from.is_dir() {
            return Err(format!("{path} isn't a folder in the Vault anymore."));
        }
        for id in self.open_inside(path) {
            self.close_work(&id, now_ms);
        }
        let trash_path = free_folder(&self.trash_root(), path)?;
        let to = folder_under(&self.trash_root(), &trash_path)?;
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        std::fs::rename(&from, &to).map_err(err)
    }

    /// Renames a Work (possibly into another folder). An open Work keeps its tab, its undo and
    /// any open burst. Returns the new ID.
    pub fn rename_work(&mut self, id: &str, new_name: &str) -> Result<String, String> {
        let id = &self.current_id(id);
        let (db, _) = self.paths(id)?;
        if !db.exists() {
            return Err(format!("{id} isn't in the Vault anymore."));
        }
        let new_id = self.free_name(new_name)?;
        let (new_db, new_typ) = self.paths(&new_id)?;
        if let Some(work) = self.works.get_mut(id) {
            work.save();
        }
        move_work(&db, &new_db)?;
        if let Some(mut work) = self.works.remove(id) {
            work.relocate(new_db, new_typ, Recovery::for_work(&self.recovery_dir, &self.root, &new_id));
            self.works.insert(new_id.clone(), work);
            self.aliases.insert(id.clone(), new_id.clone());
        }
        Ok(new_id)
    }

    /// Moves a Work to the trash (closing it first if it's open).
    pub fn trash_work(&mut self, id: &str, now_ms: u64) -> Result<(), String> {
        let id = &self.current_id(id);
        if self.works.contains_key(id) {
            self.close_work(id, now_ms);
        }
        let (db, _) = self.paths(id)?;
        if !db.exists() {
            return Err(format!("{id} isn't in the Vault anymore."));
        }
        let trash_id = free_id(&self.trash_root(), id)?;
        let (trash_db, _) = paths_under(&self.trash_root(), &trash_id)?;
        move_work(&db, &trash_db)
    }

    /// The Works in the trash.
    pub fn list_trash(&self) -> Vec<TrashedWork> {
        let trash = self.trash_root();
        let mut works: Vec<TrashedWork> = walk_works(&trash).dbs.iter().map(|db| {
            let relative = db.strip_prefix(&trash).unwrap_or(db);
            let path = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            let id = path.strip_suffix(".db").unwrap_or(&path).to_string();
            TrashedWork { id, name: name_of(db) }
        }).collect();
        works.sort_by_key(|w| w.id.to_lowercase());
        works
    }

    /// Puts a Work back from the trash where it was (as "Rain (2)" if that name is taken now).
    /// A tab still open on it follows it back. Returns its ID.
    pub fn restore_work(&mut self, trash_id: &str) -> Result<String, String> {
        let (trash_db, _) = paths_under(&self.trash_root(), trash_id)?;
        if !trash_db.exists() {
            return Err(format!("{trash_id} isn't in the trash anymore."));
        }
        let id = free_id(&self.root, trash_id)?;
        let (db, _) = self.paths(&id)?;
        move_work(&trash_db, &db)?;
        self.follow_lost();
        Ok(id)
    }

    /// Restores the History of an open tab whose Work went to the trash on another Device.
    pub fn restore_open(&mut self, id: &str) -> Result<String, String> {
        let id = self.current_id(id);
        let Some(Whereabouts::Trashed { db }) = self.works.get(&id).and_then(|w| w.whereabouts.clone()) else {
            return Err(format!("{id} isn't in the trash."));
        };
        let trash = self.trash_root();
        let relative = db.strip_prefix(&trash).map_err(err)?;
        let path = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        self.restore_work(path.strip_suffix(".db").unwrap_or(&path))?;
        Ok(self.current_id(&id))
    }

    /// Deletes everything in the trash, for good.
    pub fn empty_trash(&mut self) -> Result<(), String> {
        let trash = self.trash_root();
        if trash.exists() {
            std::fs::remove_dir_all(&trash).map_err(err)?;
        }
        Ok(())
    }

    /// Open Works whose History is no longer where their tab has it: find where it went, by its
    /// first Change. Renamed: the tab follows. In the trash or gone: saving stays paused (the
    /// recovery file holds what's typed meanwhile).
    pub(super) fn follow_lost(&mut self) {
        // A History that's back where its tab has it (restored): saving goes on as before.
        for work in self.works.values_mut() {
            if !work.is_lost() && matches!(work.whereabouts, Some(Whereabouts::Trashed { .. } | Whereabouts::Gone)) {
                work.whereabouts = None;
            }
        }
        let lost: Vec<String> = self.works.iter().filter(|(_, w)| w.is_lost()).map(|(id, _)| id.clone()).collect();
        if lost.is_empty() {
            return;
        }
        // Every History in the Vault and in its trash, by what tells each Work apart (the
        // Vault's own win over the trash's).
        let trash = self.trash_root();
        let mut found: HashMap<String, PathBuf> = HashMap::new();
        for db in walk_works(&trash).dbs.into_iter().chain(self.walk().dbs) {
            if let Ok(Some(identity)) = storage::identity(&db) {
                found.insert(identity, db);
            }
        }
        for id in lost {
            if self.works[&id].whereabouts == Some(Whereabouts::Diverged) {
                continue; // stays paused until the Author chooses
            }
            let place = self.works[&id].identity().and_then(|identity| found.get(identity)).cloned();
            // Only onto a History that has everything this tab saved: one that went its own
            // way (a copy edited separately) would be broken by mixing.
            let place = match place {
                Some(db) if !self.works[&id].agrees_with(&db) => {
                    self.works.get_mut(&id).expect("listed above").whereabouts = Some(Whereabouts::Diverged);
                    continue;
                }
                place => place,
            };
            match place {
                Some(db) if db.starts_with(&trash) => {
                    self.works.get_mut(&id).expect("listed above").whereabouts = Some(Whereabouts::Trashed { db });
                }
                Some(db) => {
                    let new_id = self.id_of(&db);
                    if new_id != id && self.works.contains_key(&new_id) {
                        continue; // (can't be: a Work is open in one tab at most)
                    }
                    let typ = db.with_extension("typ");
                    let recovery = Recovery::for_work(&self.recovery_dir, &self.root, &new_id);
                    let mut work = self.works.remove(&id).expect("listed above");
                    work.relocate(db, typ, recovery);
                    work.whereabouts = (new_id != id).then(|| Whereabouts::Renamed { to: new_id.clone() });
                    self.works.insert(new_id.clone(), work);
                    if new_id != id {
                        self.aliases.insert(id, new_id);
                    }
                }
                None => self.works.get_mut(&id).expect("listed above").whereabouts = Some(Whereabouts::Gone),
            }
        }
    }

    /// Before closing a tab whose Work went to the trash: point it at the trashed History, so
    /// what was typed since is saved there and comes back if the Work is restored.
    pub(super) fn save_into_trash(&mut self, id: &str) {
        let Some(work) = self.works.get_mut(id) else { return };
        let Some(Whereabouts::Trashed { db }) = work.whereabouts.clone() else { return };
        let typ = db.with_extension("typ");
        let recovery = Recovery::for_work(&self.recovery_dir, &self.root, &format!("{TRASH}/{id}"));
        work.relocate(db, typ, recovery);
    }

    /// Closes a tab whose Work is gone from the Vault, dropping what was typed since.
    pub fn discard_work(&mut self, id: &str) {
        let id = self.current_id(id);
        if let Some(mut work) = self.works.remove(&id) {
            work.discard();
        }
        self.aliases.retain(|_, to| *to != id);
    }

    /// Saves the text of a tab whose Work is gone as a new Work, and closes that tab.
    pub fn save_as_new(&mut self, id: &str, new_name: &str, now_ms: u64) -> Result<OpenedWork, String> {
        let id = self.current_id(id);
        let text = self.works.get(&id).map(|w| w.text()).ok_or_else(|| format!("{id} isn't open."))?;
        let opened = self.create_work(new_name, now_ms)?;
        self.edit(&opened.id, vec![Op::Paste { from: 0, to: 0, text }], now_ms)?;
        self.discard_work(&id);
        Ok(OpenedWork { text: self.works[&opened.id].text(), ..opened })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;
    use crate::work::{Op, Origin};

    fn typing(from: usize, text: &str) -> Op {
        Op::Type { from, to: from, text: text.into(), origin: Origin::Input }
    }

    #[test]
    fn folders_are_listed_even_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        assert_eq!(vault.create_folder(" Short stories / Old ").unwrap(), "Short stories/Old");
        assert!(vault.create_folder("Short stories").is_err(), "already there");
        assert!(vault.create_folder("styles").is_err(), "the app's own folder");
        assert!(vault.create_folder(".hidden").is_err());
        assert_eq!(vault.list(1000).folders, vec!["Short stories".to_string(), "Short stories/Old".to_string()]);
    }

    #[test]
    fn renaming_a_folder_keeps_the_tabs_of_works_open_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Short stories/Rain", 1000).unwrap();
        vault.edit("Short stories/Rain", vec![typing(0, "abc")], 2000).unwrap(); // a burst is open
        let moved = vault.rename_folder("Short stories", "Stories", 2500).unwrap();
        assert_eq!(moved, vec![("Short stories/Rain".to_string(), "Stories/Rain".to_string())]);
        vault.edit("Short stories/Rain", vec![typing(3, "d")], 2100).unwrap(); // still on its way under the old ID
        vault.close_work("Stories/Rain", 3000);
        assert_eq!(storage::load(&root.join("Stories/Rain.db"), ME).unwrap().1.text(), "abcd");
        assert!(!root.join("Short stories").exists(), "never recreated at the old place");
        assert!(vault.rename_folder("Stories", "Stories/Inside", 2600).is_err(), "not inside itself");
    }

    #[test]
    fn a_trashed_folder_takes_its_works_along() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Short stories/Rain", 1000).unwrap();
        vault.edit("Short stories/Rain", vec![typing(0, "abc")], 2000).unwrap();
        vault.trash_folder("Short stories", 3000).unwrap();
        assert!(!vault.works.contains_key("Short stories/Rain"), "closed (and saved) first");
        assert!(vault.list(4000).entries.is_empty());
        assert_eq!(vault.list_trash(), vec![TrashedWork { id: "Short stories/Rain".into(), name: "Rain".into() }]);
        assert_eq!(vault.restore_work("Short stories/Rain").unwrap(), "Short stories/Rain");
        assert_eq!(vault.open_work("Short stories/Rain", 5000).unwrap().text, "abc");
    }

    #[test]
    fn renaming_an_open_work_keeps_its_tab() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap(); // a burst is open
        let new_id = vault.rename_work("Rain", "Short stories/Rain").unwrap();
        assert_eq!(new_id, "Short stories/Rain");
        assert!(!root.join("Rain.db").exists() && root.join("Short stories/Rain.db").exists());
        // Edits still on their way under the old ID find it.
        vault.edit("Rain", vec![typing(3, "d")], 2100).unwrap();
        vault.close_work("Short stories/Rain", 3000);
        assert_eq!(storage::load(&root.join("Short stories/Rain.db"), ME).unwrap().1.text(), "abcd");
        assert!(!root.join("Rain.db").exists(), "never recreated at the old place");
    }

    #[test]
    fn a_trashed_work_can_be_restored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        vault.trash_work("Rain", 3000).unwrap();
        assert!(vault.list(4000).entries.is_empty(), "the browser doesn't show the trash");
        assert_eq!(vault.list_trash(), vec![TrashedWork { id: "Rain".into(), name: "Rain".into() }]);
        vault.create_work("Rain", 5000).unwrap(); // the name was taken meanwhile
        assert_eq!(vault.restore_work("Rain").unwrap(), "Rain (2)");
        assert_eq!(vault.open_work("Rain (2)", 6000).unwrap().text, "abc");
        vault.trash_work("Rain (2)", 7000).unwrap();
        vault.empty_trash().unwrap();
        assert!(vault.list_trash().is_empty());
    }

    #[test]
    fn a_tab_follows_its_work_renamed_on_another_device() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        vault.tick(9000); // saved
        // Syncthing carries out the other Device's rename here.
        std::fs::rename(root.join("Rain.db"), root.join("Storm.db")).unwrap();
        std::fs::rename(root.join("Rain.typ"), root.join("Storm.typ")).unwrap();
        let changed = vault.tick(10_000);
        let (_, state) = changed.iter().find(|(id, _)| id == "Rain").expect("the editor hears about it");
        assert_eq!(state.moved, Some(Whereabouts::Renamed { to: "Storm".into() }));
        vault.edit("Rain", vec![typing(3, "!")], 11_000).unwrap();
        vault.close_work("Storm", 12_000);
        assert_eq!(storage::load(&root.join("Storm.db"), ME).unwrap().1.text(), "abc!");
        assert!(!root.join("Rain.db").exists());
    }

    #[test]
    fn a_tab_whose_work_was_trashed_elsewhere_pauses_and_can_restore_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        vault.tick(9000);
        std::fs::create_dir_all(root.join(TRASH)).unwrap();
        std::fs::rename(root.join("Rain.db"), root.join(".trash/Rain.db")).unwrap();
        std::fs::rename(root.join("Rain.typ"), root.join(".trash/Rain.typ")).unwrap();
        vault.tick(10_000);
        assert!(matches!(vault.works["Rain"].whereabouts, Some(Whereabouts::Trashed { .. })));
        vault.edit("Rain", vec![typing(3, "d")], 11_000).unwrap();
        vault.tick(20_000); // the burst closes, but nothing is saved: the History isn't there
        assert!(!root.join("Rain.db").exists(), "never recreated at the old place");
        assert_eq!(vault.restore_open("Rain").unwrap(), "Rain");
        vault.close_work("Rain", 21_000);
        assert_eq!(storage::load(&root.join("Rain.db"), ME).unwrap().1.text(), "abcd");
    }

    #[test]
    fn closing_a_tab_whose_work_was_trashed_saves_into_the_trash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.tick(2000);
        std::fs::create_dir_all(root.join(TRASH)).unwrap();
        std::fs::rename(root.join("Rain.db"), root.join(".trash/Rain.db")).unwrap();
        vault.tick(3000);
        vault.edit("Rain", vec![typing(0, "late")], 4000).unwrap();
        vault.close_work("Rain", 5000);
        assert_eq!(storage::load(&root.join(".trash/Rain.db"), ME).unwrap().1.text(), "late");
    }

    #[test]
    fn a_tab_follows_its_own_work_not_another_one_from_this_device() {
        // What happened to test4: several Works made on this Device (each starting with Change
        // 1 of its own), and one moved into a folder in Explorer while its tab was open.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        for name in ["test", "test2", "test3"] {
            vault.create_work(name, 1000).unwrap();
            vault.edit(name, vec![typing(0, name)], 2000).unwrap();
        }
        vault.tick(9000); // all saved
        std::fs::create_dir_all(root.join("New folder")).unwrap();
        std::fs::rename(root.join("test2.db"), root.join("New folder/test2.db")).unwrap();
        std::fs::rename(root.join("test2.typ"), root.join("New folder/test2.typ")).unwrap();
        vault.tick(10_000);
        assert!(vault.works.contains_key("New folder/test2"), "followed into the folder");
        vault.edit("test2", vec![typing(5, "!")], 11_000).unwrap();
        vault.close_all(12_000);
        for (path, text) in [("test.db", "test"), ("New folder/test2.db", "test2!"), ("test3.db", "test3")] {
            assert_eq!(storage::load(&root.join(path), ME).unwrap().1.text(), text, "{path}");
        }
    }

    #[test]
    fn a_tab_never_mixes_its_history_with_a_copy_that_went_its_own_way() {
        // The situation test4 got into: a copy of the History, from before this tab's latest
        // Changes, edited on its own, then put where the tab's History was.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        vault.tick(9000); // saved
        let older = root.join("older.db");
        std::fs::copy(root.join("Rain.db"), &older).unwrap();
        vault.edit("Rain", vec![typing(3, "def")], 10_000).unwrap();
        vault.tick(20_000); // saved: Change N is "def"
        // The copy goes its own way from before "def": its Change N is something else.
        let (_, mut other) = storage::load(&older, ME).unwrap();
        storage::save(&older, &[other.insert_local(0, "XYZ", 15_000)], &other).unwrap();
        std::fs::copy(&older, root.join("Rain.db")).unwrap();

        vault.tick(30_000); // noticed
        vault.edit("Rain", vec![Op::Sync], 30_001).unwrap(); // and looked at
        assert_eq!(vault.works["Rain"].whereabouts, Some(Whereabouts::Diverged));
        vault.edit("Rain", vec![typing(6, "!")], 31_000).unwrap();
        vault.tick(40_000);
        // Nothing was written into the other History, so it still loads.
        assert_eq!(storage::load(&root.join("Rain.db"), ME).unwrap().1.text(), "XYZabc");
        // And what this tab has can go to a new Work.
        let saved = vault.save_as_new("Rain", "Rain (kept)", 41_000).unwrap();
        assert_eq!(saved.text, "abcdef!");
    }

    #[test]
    fn a_tab_whose_work_is_gone_can_save_it_as_a_new_work() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut vault = open(root);
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typing(0, "abc")], 2000).unwrap();
        vault.tick(9000);
        std::fs::remove_file(root.join("Rain.db")).unwrap();
        vault.tick(10_000);
        assert_eq!(vault.works["Rain"].whereabouts, Some(Whereabouts::Gone));
        let opened = vault.save_as_new("Rain", "Rain again", 11_000).unwrap();
        assert_eq!(opened.text, "abc");
        assert!(!vault.works.contains_key("Rain"));
        vault.close_work("Rain again", 12_000);
        assert_eq!(storage::load(&root.join("Rain again.db"), ME).unwrap().1.text(), "abc");
    }
}
