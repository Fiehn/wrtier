//! The Reading panel (v2 plan): one Work's text, to read beside the Work being written. Reading
//! a Work never opens it and never records anything in its History: its text, Inclusions,
//! Comments and Story elements are read from its `.db` (through the cache Inclusions use, so an
//! unchanged file isn't read again), or from its Text file while it has no History yet.
//!
//! A History can take a while to read, so it's done in three steps: where the Work is (under
//! the Vault's lock), reading its file (without it), then working out its Inclusions and
//! Comments (under the lock again). The interface reads a Work open in a tab from that tab.
//!
//! The Work is followed by its Work ID, as an Inclusion follows it, so a rename or move on any
//! Device keeps it in the panel.

use super::inclusions::{comments_in, expand, views, Ctx, Located};
use super::*;
use crate::work::CommentHighlight;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// What the Reading panel shows of a Work, or why it can't.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Reading {
    /// The same as the stamp asked with: what the panel shows is still right.
    Unchanged,
    /// Its text (an Inclusion is its one character) and what goes with it. `id`: where it is in
    /// the Vault now; `identity`: its Work ID, to follow it with; `stamp`: to ask with next time.
    Text {
        id: String,
        name: String,
        identity: Option<String>,
        stamp: String,
        text: String,
        inclusions: Vec<InclusionView>,
        /// Its Comments on its own text (not Detached ones, nor Work comments), ranges in `text`.
        highlights: Vec<CommentHighlight>,
        /// The Author's own Story elements it has (its Preamble's).
        elements: Vec<String>,
    },
    /// In the trash, gone, from a newer version of the app, damaged: why.
    Unreadable { id: String, name: String, identity: Option<String>, reason: String },
}

/// Where a Work to read is: worked out under the Vault's lock, read after it's let go.
pub struct ReadingSource {
    id: String,
    identity: Option<String>,
    device: DeviceId,
    found: Found,
}

enum Found {
    /// A History: `fresh` when the cache (or an open tab) already has it as it is now.
    History { located: Located, fresh: bool },
    /// A Text file without a History yet.
    TextFile(PathBuf),
    Unreadable(String),
}

/// What reading the file gave.
pub enum Loaded {
    /// Nothing needed reading.
    Nothing,
    /// The History, and the file's stamp from just before it was read.
    Doc(Option<crate::stamps::Stamp>, Doc),
    Text(String),
    Unreadable(String),
}

fn name_of(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

impl Vault {
    /// Where the Work to read is: by its Work ID if it has one (following renames and moves),
    /// else by `work`, its ID in the Vault.
    pub fn reading_source(&mut self, work: &str, identity: Option<&str>) -> ReadingSource {
        let mut source = ReadingSource { id: work.to_string(), identity: identity.map(str::to_string), device: self.device, found: Found::Unreadable(String::new()) };
        let located = identity.and_then(|i| self.work_index().get(i).cloned());
        source.found = match located {
            Some(located) if located.trashed => {
                source.id = located.id.clone();
                Found::Unreadable(format!("{} is in the trash.", name_of(&located.id)))
            }
            Some(located) => {
                source.id = located.id.clone();
                self.history_found(located)
            }
            None => match self.paths(work) {
                Err(e) => Found::Unreadable(e),
                Ok((db, typ)) => {
                    // A `.db` here with another Work ID is another Work: the one read is gone.
                    let theirs = db.exists().then(|| self.identity_of(&db)).flatten();
                    if db.exists() && (identity.is_none() || theirs.as_deref() == identity) {
                        source.identity = source.identity.or(theirs);
                        self.history_found(Located { db, id: work.to_string(), trashed: false, duplicate: false })
                    } else if typ.exists() && identity.is_none() {
                        Found::TextFile(typ)
                    } else {
                        Found::Unreadable(format!("{} isn't in the Vault any more.", name_of(work)))
                    }
                }
            },
        };
        source
    }

    fn history_found(&self, located: Located) -> Found {
        let open = self.works.values().any(|w| w.db == located.db);
        let cached = self.included_docs.stamp_of(&(located.db.clone(), String::new()))
            .is_some_and(|seen| seen == file_stamp(&located.db));
        Found::History { located, fresh: open || cached }
    }

    /// What the Reading panel shows, once the file is read, unless it still has the stamp `known`.
    pub fn reading(&mut self, source: ReadingSource, loaded: Loaded, known: Option<&str>) -> Reading {
        let ReadingSource { id, identity, found, .. } = source;
        let name = name_of(&id);
        let unreadable = |reason: String| Reading::Unreadable { id: id.clone(), name: name.clone(), identity: identity.clone(), reason };
        let reading = match (found, loaded) {
            (Found::Unreadable(reason), _) | (_, Loaded::Unreadable(reason)) => return unreadable(reason),
            (Found::TextFile(_), Loaded::Text(text)) => Reading::Text {
                id: id.clone(), name: name.clone(), identity: None, stamp: String::new(), text,
                inclusions: Vec::new(), highlights: Vec::new(), elements: Vec::new(),
            },
            (Found::History { located, .. }, loaded) => {
                if let Loaded::Doc(stamp, doc) = loaded {
                    self.included_docs.insert((located.db.clone(), String::new()), stamp, Arc::new(doc));
                }
                match self.read_history(&located, identity.clone()) {
                    Ok(reading) => reading,
                    Err(reason) => return unreadable(reason),
                }
            }
            (Found::TextFile(_), _) => return unreadable(format!("{name} can't be read right now.")),
        };
        let Reading::Text { id, name, identity, text, inclusions, highlights, elements, .. } = reading else { return reading };
        // The stamp is what's shown, so anything that changes it (the included Works, a
        // Reviewer's name) is sent again, and nothing else is.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&(&id, &text, &inclusions, &highlights, &elements)).unwrap_or_default().hash(&mut hasher);
        let stamp = format!("{:016x}", hasher.finish());
        if known == Some(stamp.as_str()) {
            return Reading::Unchanged;
        }
        Reading::Text { id, name, identity, stamp, text, inclusions, highlights, elements }
    }

    /// A History's text, Inclusions, Comments and Story elements (it's in the cache, or open).
    fn read_history(&mut self, located: &Located, identity: Option<String>) -> Result<Reading, String> {
        let index = self.work_index();
        let shelf = self.shelf();
        let name = name_of(&located.id);
        // Open in a tab here: as Rust has it. Otherwise from the cache (read just now, or still
        // the same file), or read here if it went from the cache meanwhile.
        let open = self.works.values().any(|w| w.db == located.db);
        let read: Option<Arc<Doc>> = if open {
            None
        } else {
            let key = (located.db.clone(), String::new());
            Some(match self.included_docs.get(&key) {
                Some((_, doc)) => doc,
                None => {
                    let doc = Arc::new(storage::load(&located.db, self.device).map_err(|e| format!("{name} can't be read right now ({e})."))?.1);
                    self.included_docs.insert(key, file_stamp(&located.db), doc.clone());
                    doc
                }
            })
        };
        let doc: &Doc = match &read {
            Some(doc) => doc,
            None => &self.works.values().find(|w| w.db == located.db).expect("open").doc,
        };
        let mut ctx = Ctx { works: &self.works, index: &index, cache: &mut self.included_docs, shelf: &shelf, device: self.device, recorded: false };
        let mut stack: Vec<String> = identity.iter().cloned().collect();
        let mut shown = expand(&mut ctx, doc, &mut stack);
        drop(ctx);
        // As the Work in front shows them: its own text without the open burst of a tab.
        let text = doc.committed_text();
        let length = text.chars().count();
        let mut highlights = comments_in(doc, &text, &[(0, 0, length)]);
        for highlight in &mut highlights {
            (highlight.name, highlight.colour) = self.sender_label(doc, &highlight.sender);
            for reply in &mut highlight.replies {
                (reply.name, reply.colour) = self.sender_label(doc, &reply.sender);
            }
        }
        for s in &mut shown {
            let author = s.author.clone();
            for highlight in &mut s.comments {
                (highlight.name, highlight.colour) = self.included_sender(&author, &highlight.sender);
                for reply in &mut highlight.replies {
                    (reply.name, reply.colour) = self.included_sender(&author, &reply.sender);
                }
            }
        }
        let identity = identity.or_else(|| doc.field(storage::WORK_RECORD, storage::WORK_ID_FIELD).map(str::to_string));
        Ok(Reading::Text {
            id: located.id.clone(), name, identity, stamp: String::new(),
            inclusions: views(&text, &shown),
            highlights,
            elements: Preamble::of(doc).elements,
            text,
        })
    }
}

impl ReadingSource {
    /// Reads the file, if it needs reading (without the Vault's lock).
    pub fn load(&self) -> Loaded {
        let name = name_of(&self.id);
        let read = catch_unwind(AssertUnwindSafe(|| -> Result<Loaded, String> {
            match &self.found {
                Found::Unreadable(_) | Found::History { fresh: true, .. } => Ok(Loaded::Nothing),
                Found::History { located, fresh: false } => {
                    match storage::probe(&located.db) {
                        Probe::History => {}
                        Probe::NewerHistory(_) => return Err(format!("{name} can't be read: it was made by a newer version of the app.")),
                        Probe::NotHistory => return Err(format!("{name}.db belongs to another program.")),
                        Probe::Unreadable(e) => return Err(format!("{name} can't be read right now ({e}).")),
                    }
                    let stamp = file_stamp(&located.db);
                    let (_, doc) = storage::load(&located.db, self.device).map_err(err)?;
                    Ok(Loaded::Doc(stamp, doc))
                }
                Found::TextFile(typ) => {
                    let contents = std::fs::read_to_string(typ).map_err(err)?;
                    let (text, _) = wrtier::text_file::split_marker(&contents);
                    Ok(Loaded::Text(wrtier::text_file::split_header(&text).1))
                }
            }
        }));
        match read {
            Ok(Ok(loaded)) => loaded,
            Ok(Err(reason)) => Loaded::Unreadable(reason),
            Err(panic) => Loaded::Unreadable(format!("{name} can't be read: its History looks damaged ({}).", panic_message(panic.as_ref()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;
    use crate::work::Origin;
    use wrtier::change::INCLUSION;

    fn typed(at: usize, text: &str) -> Op {
        Op::Type { from: at, to: at, text: text.into(), origin: Origin::Input }
    }

    fn read(vault: &mut Vault, work: &str, identity: Option<&str>, known: Option<&str>) -> Reading {
        let source = vault.reading_source(work, identity);
        let loaded = source.load();
        vault.reading(source, loaded, known)
    }

    fn text_of(reading: &Reading) -> (&str, &str, Option<&str>) {
        match reading {
            Reading::Text { id, text, identity, .. } => (id, text, identity.as_deref()),
            other => panic!("not read: {other:?}"),
        }
    }

    /// A closed Work called `id`, with `text`.
    fn written(vault: &mut Vault, id: &str, text: &str, at: u64) {
        vault.create_work(id, at).unwrap();
        vault.edit(id, vec![typed(0, text)], at + 1).unwrap();
        vault.close_work(id, at + 2);
    }

    #[test]
    fn a_closed_work_is_read_without_opening_it_or_recording_anything() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        written(&mut vault, "Rain", "= One\n\nThe lighthouse was dark.\n", 1000);
        let db = dir.path().join("Rain.db");
        let before = storage::read_all(&db).unwrap().len();
        let modified = std::fs::metadata(&db).unwrap().modified().unwrap();

        let reading = read(&mut vault, "Rain", None, None);
        let (id, text, identity) = text_of(&reading);
        assert_eq!((id, text), ("Rain", "= One\n\nThe lighthouse was dark.\n"));
        assert!(identity.is_some(), "it names its Work ID, to be followed by");
        assert!(vault.works.is_empty(), "it isn't opened");
        assert_eq!(storage::read_all(&db).unwrap().len(), before, "nothing is recorded");
        assert_eq!(std::fs::metadata(&db).unwrap().modified().unwrap(), modified);
    }

    #[test]
    fn an_unchanged_work_is_not_sent_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        written(&mut vault, "Rain", "Dark.", 1000);
        let Reading::Text { stamp, identity, .. } = read(&mut vault, "Rain", None, None) else { panic!("not read") };
        assert_eq!(read(&mut vault, "Rain", identity.as_deref(), Some(&stamp)), Reading::Unchanged);

        vault.open_work("Rain", 4000).unwrap();
        vault.edit("Rain", vec![typed(5, " And wet.")], 5000).unwrap();
        vault.close_work("Rain", 6000);
        let reading = read(&mut vault, "Rain", identity.as_deref(), Some(&stamp));
        assert_eq!(text_of(&reading).1, "Dark. And wet.");
    }

    #[test]
    fn it_follows_its_work_through_a_rename_and_says_when_it_is_in_the_trash() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        written(&mut vault, "Rain", "Dark.", 1000);
        let Reading::Text { identity, .. } = read(&mut vault, "Rain", None, None) else { panic!("not read") };
        let identity = identity.unwrap();

        vault.rename_work("Rain", "Stories/Rain at sea").unwrap();
        let reading = read(&mut vault, "Rain", Some(&identity), None);
        assert_eq!(text_of(&reading).0, "Stories/Rain at sea");

        vault.trash_work("Stories/Rain at sea", 2000).unwrap();
        let Reading::Unreadable { reason, .. } = read(&mut vault, "Stories/Rain at sea", Some(&identity), None) else { panic!("read from the trash") };
        assert_eq!(reason, "Rain at sea is in the trash.");
    }

    #[test]
    fn another_work_made_where_it_was_is_not_taken_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        written(&mut vault, "Rain", "Dark.", 1000);
        let Reading::Text { identity, .. } = read(&mut vault, "Rain", None, None) else { panic!("not read") };
        vault.trash_work("Rain", 2000).unwrap();
        vault.empty_trash().unwrap();
        written(&mut vault, "Rain", "A different Rain.", 3000);
        let Reading::Unreadable { reason, .. } = read(&mut vault, "Rain", identity.as_deref(), None) else { panic!("read the other one") };
        assert_eq!(reason, "Rain isn't in the Vault any more.");
    }

    #[test]
    fn a_text_file_without_a_history_is_read_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        std::fs::write(dir.path().join("Notes.typ"), "Written in vim.\n").unwrap();
        let reading = read(&mut vault, "Notes", None, None);
        assert_eq!(text_of(&reading).1, "Written in vim.\n");
        assert!(!dir.path().join("Notes.db").exists(), "it doesn't get a History by being read");
    }

    #[test]
    fn its_comments_and_inclusions_come_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        written(&mut vault, "Zampano", "The Navidson Record.", 1000);
        vault.create_work("House", 2000).unwrap();
        vault.edit("House", vec![
            typed(0, "Truant writes.\n"),
            Op::Paste { from: 15, to: 15, text: INCLUSION.to_string() },
            Op::Include { at: 15, work: "Zampano".into(), chapter: None, target: None },
        ], 2001).unwrap();
        vault.edit("House", vec![Op::Comment { from: 0, to: 6, sender: "author".into(), body: "Who is he?".into(), sender_name: None }], 2002).unwrap();
        vault.close_work("House", 2003);

        let Reading::Text { text, inclusions, highlights, .. } = read(&mut vault, "House", None, None) else { panic!("not read") };
        assert_eq!(text, format!("Truant writes.\n{INCLUSION}"));
        assert_eq!(inclusions.len(), 1);
        assert_eq!((inclusions[0].at, inclusions[0].text.as_str()), (15, "The Navidson Record."));
        assert_eq!(highlights.len(), 1);
        assert_eq!((highlights[0].ranges.clone(), highlights[0].body.as_str(), highlights[0].name.as_str()), (vec![(0, 6)], "Who is he?", "Author"));
    }

    #[test]
    fn a_history_from_a_newer_app_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        std::fs::write(dir.path().join("Rain.db"), b"not a history at all").unwrap();
        let Reading::Unreadable { .. } = read(&mut vault, "Rain", None, None) else { panic!("read a file that isn't a History") };
    }
}
