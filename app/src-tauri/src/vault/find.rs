//! Find in Vault (v2 plan): each Work's own text, read for finding in. The interface does the
//! matching (find.ts, so it reads text exactly as Find and replace does); this side only hands it
//! the text of Works that aren't open in a tab, from their History.
//!
//! A History can take a while to read, so it's read without the Vault's lock (the caller takes
//! the lock only for `text_source`), and each answer carries a stamp of the `.db` file: asked
//! again with that stamp, an unchanged Work isn't read again.

use super::*;

/// A Work's text for finding in, or why there's none.
#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TextForFinding {
    /// The same as the stamp asked with: what the interface has is still right.
    Unchanged,
    /// Its own text (an Inclusion is its one character), and the stamp to ask with next time.
    Text { stamp: String, text: String },
    /// It can't be read (from a newer version of the app, damaged, gone): why.
    Unreadable { reason: String },
}

/// Where a Work's text is: worked out under the Vault's lock, read after it's let go.
pub struct TextSource {
    db: PathBuf,
    typ: PathBuf,
    device: DeviceId,
    name: String,
    /// The Work is open here: its text as it is now.
    open: Option<String>,
}

impl Vault {
    /// Where to read a Work's text for Find in Vault (an ID in the Vault).
    pub fn text_source(&mut self, id: &str) -> Result<TextSource, String> {
        let (db, typ) = self.paths(id)?;
        // The open burst included, as the editor shows it.
        let open = self.works.get(&self.current_id(id)).map(|w| w.text());
        let name = id.rsplit('/').next().unwrap_or(id).to_string();
        Ok(TextSource { db, typ, device: self.device, name, open })
    }
}

impl TextSource {
    /// Reads the text, unless the files still have the stamp `known`.
    pub fn read(self, known: Option<&str>) -> TextForFinding {
        if let Some(text) = self.open {
            // Open here: what it is now (the interface reads open tabs itself, so this is rare).
            return TextForFinding::Text { stamp: String::new(), text };
        }
        let stamp = stamp_of(&self.db).or_else(|| stamp_of(&self.typ)).unwrap_or_default();
        if !stamp.is_empty() && known == Some(stamp.as_str()) {
            return TextForFinding::Unchanged;
        }
        let name = &self.name;
        let read = catch_unwind(AssertUnwindSafe(|| -> Result<String, String> {
            if self.db.exists() {
                match storage::probe(&self.db) {
                    Probe::History => {}
                    Probe::NewerHistory(_) => return Err("made by a newer version of the app".into()),
                    Probe::NotHistory => return Err(format!("{name}.db belongs to another program")),
                    Probe::Unreadable(e) => return Err(format!("can't be read right now ({e})")),
                }
                let (_, doc) = storage::load(&self.db, self.device).map_err(err)?;
                Ok(doc.committed_text())
            } else if self.typ.exists() {
                // A Text file without a History yet: its text is the file's, as opening it reads it.
                let contents = std::fs::read_to_string(&self.typ).map_err(err)?;
                let (text, _) = wrtier::text_file::split_marker(&contents);
                Ok(wrtier::text_file::split_header(&text).1)
            } else {
                Err("isn't in the Vault any more".into())
            }
        }));
        match read {
            Ok(Ok(text)) => TextForFinding::Text { stamp, text },
            Ok(Err(reason)) => TextForFinding::Unreadable { reason },
            Err(panic) => TextForFinding::Unreadable { reason: format!("its History looks damaged ({})", panic_message(panic.as_ref())) },
        }
    }
}

/// When a file (and its write-ahead log, if any) last changed, and how big it is: different
/// whenever its content is. None if there's no such file.
fn stamp_of(path: &Path) -> Option<String> {
    let part = |path: &Path| {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
        Some(format!("{modified}:{}", meta.len()))
    };
    let main = part(path)?;
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    Some(match part(Path::new(&wal)) {
        Some(log) => format!("{main}+{log}"),
        None => main,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::*;
    use super::*;
    use crate::work::Origin;

    fn typed(at: usize, text: &str) -> Op {
        Op::Type { from: at, to: at, text: text.into(), origin: Origin::Input }
    }

    fn read(vault: &mut Vault, id: &str, known: Option<&str>) -> TextForFinding {
        vault.text_source(id).unwrap().read(known)
    }

    #[test]
    fn a_closed_works_text_is_read_from_its_history_and_not_again_while_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typed(0, "= One\n\nThe lighthouse was dark.\n")], 2000).unwrap();
        vault.close_work("Rain", 3000);

        let TextForFinding::Text { stamp, text } = read(&mut vault, "Rain", None) else { panic!("not read") };
        assert_eq!(text, "= One\n\nThe lighthouse was dark.\n");
        assert!(!stamp.is_empty());
        assert_eq!(read(&mut vault, "Rain", Some(&stamp)), TextForFinding::Unchanged);

        // Written to again (opened, typed in, closed): read again.
        vault.open_work("Rain", 4000).unwrap();
        vault.edit("Rain", vec![typed(0, "Prologue.\n\n")], 5000).unwrap();
        vault.close_work("Rain", 6000);
        let TextForFinding::Text { text, .. } = read(&mut vault, "Rain", Some(&stamp)) else { panic!("not read again") };
        assert!(text.starts_with("Prologue.\n\n= One"));
    }

    #[test]
    fn an_open_work_is_read_as_it_is_now() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        vault.create_work("Rain", 1000).unwrap();
        vault.edit("Rain", vec![typed(0, "Still typing")], 2000).unwrap();
        let TextForFinding::Text { text, .. } = read(&mut vault, "Rain", None) else { panic!("not read") };
        assert_eq!(text, "Still typing");
    }

    #[test]
    fn a_text_file_without_a_history_is_read_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        std::fs::write(dir.path().join("Notes.typ"), "Written in vim, with a lighthouse.\n").unwrap();
        let TextForFinding::Text { text, .. } = read(&mut vault, "Notes", None) else { panic!("not read") };
        assert_eq!(text, "Written in vim, with a lighthouse.\n");
        assert!(!dir.path().join("Notes.db").exists(), "reading it makes no History");
    }

    #[test]
    fn a_work_that_cant_be_read_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut vault = open(dir.path());
        std::fs::write(dir.path().join("Other.db"), "not a History at all, just bytes").unwrap();
        assert!(matches!(read(&mut vault, "Other", None), TextForFinding::Unreadable { .. }));
        assert!(matches!(read(&mut vault, "Gone", None), TextForFinding::Unreadable { .. }));
    }
}
