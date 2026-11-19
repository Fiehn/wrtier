//! Annotations (ADR 0020): notes a Work prints on the text of a Work it includes (Truant's
//! footnote on Zampanò's words), or on a whole Annotation an included Work made (the Editors on
//! one of Truant's). They live in the including Work's History, as `annotation/<ID>` records
//! whose one field holds a `Note`, and are printed as a `#footnote[…]` where the text they cover
//! ends, wherever their Work shows that text (Works including it in turn too).
//!
//! Which text an Annotation covers is named by the included Work's characters, as a Comment's
//! are, so it follows them through Moves and edits there. None of them shown: it's a Detached
//! annotation, not printed, until any of them is back.

use serde::{Deserialize, Serialize};
use wrtier::change::{annotation_record, Span, ANNOTATION_FIELD, ANNOTATION_PREFIX};
use wrtier::doc::Doc;
use crate::positions::Positions;
use super::inclusions::{inclusion_at, Ctx, Shown};
use super::{err, Vault};

/// What an Annotation's record holds.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Note {
    /// The ID of the Work whose text it covers, or which made the Annotation it's on.
    pub work: String,
    /// The characters it covers in that Work (`Span::to_text`); empty for one on an Annotation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pieces: Vec<String>,
    /// The Annotation (its ID, in `work`) it's on, for one on an Annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// What it covered when it was made (the selected text, or the Annotation's text).
    #[serde(default)]
    pub quote: String,
    /// Its text: Typst markup, printed as a footnote's.
    pub body: String,
}

/// An Annotation as recorded, with the Work it belongs to.
#[derive(Clone, Debug)]
pub struct Ann {
    pub id: String,
    /// The ID of the Work it belongs to (that made it).
    pub owner: String,
    pub note: Note,
    pub spans: Vec<Span>,
}

impl Ann {
    /// Is it on the Annotation `id` that the Work `owner` made?
    pub fn is_on(&self, owner: &str, id: &str) -> bool {
        self.note.pieces.is_empty() && self.note.work == owner && self.note.on.as_deref() == Some(id)
    }
}

/// The Annotations a Work has (not deleted), in a fixed order. `owner`: the Work's ID.
pub fn annotations_of(doc: &Doc, owner: &str) -> Vec<Ann> {
    doc.records(ANNOTATION_PREFIX).into_iter().filter_map(|record| {
        let value = doc.field(&record, ANNOTATION_FIELD)?;
        if value.is_empty() {
            return None; // deleted
        }
        let note: Note = serde_json::from_str(value).ok()?;
        let spans = note.pieces.iter().filter_map(|p| Span::parse(p)).collect();
        Some(Ann { id: record[ANNOTATION_PREFIX.len()..].to_string(), owner: owner.to_string(), note, spans })
    }).collect()
}

/// Its text as printed: its body, with the Annotations on it as footnotes at its end.
pub fn printed(ann: &Ann, all: &[Ann], depth: usize) -> String {
    let mut out = ann.body_text();
    if depth < 8 {
        for child in all.iter().filter(|c| c.is_on(&ann.owner, &ann.id)) {
            out.push_str(&format!("#footnote[{}]", printed(child, all, depth + 1)));
        }
    }
    out
}

impl Ann {
    fn body_text(&self) -> String {
        self.note.body.trim().to_string()
    }
}

/// An Annotation printed in an Inclusion, as the editor shows it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoteShown {
    pub id: String,
    /// The ID of the Work it belongs to.
    pub owner: String,
    /// That Work's name ("Truant"), filled in for the editor.
    pub owner_label: String,
    /// It belongs to the Work in front: it can be changed there.
    pub mine: bool,
    /// Where its footnote's number goes in the included text as shown (UTF-16).
    pub at: usize,
    /// What it covers there (UTF-16).
    pub ranges: Vec<(usize, usize)>,
    pub body: String,
    pub quote: String,
    /// The Annotations on it, in the order they print (at the end of its footnote).
    pub notes: Vec<NoteShown>,
}

/// An Annotation as its Notes on it: the Annotations on `ann`, as shown.
pub fn on_note(ann: &Ann, all: &[Ann], at: usize, depth: usize) -> Vec<NoteShown> {
    if depth >= 8 {
        return Vec::new();
    }
    all.iter().filter(|c| c.is_on(&ann.owner, &ann.id)).map(|c| NoteShown {
        id: c.id.clone(), owner: c.owner.clone(), owner_label: String::new(), mine: false, at,
        ranges: Vec::new(), body: c.note.body.clone(), quote: c.note.quote.clone(),
        notes: on_note(c, all, at, depth + 1),
    }).collect()
}

/// Every Annotation shown in some Inclusions (those on Annotations too), flattened.
pub fn every_note(shown: &[Shown]) -> Vec<&NoteShown> {
    fn walk<'a>(notes: &'a [NoteShown], out: &mut Vec<&'a NoteShown>) {
        for n in notes {
            out.push(n);
            walk(&n.notes, out);
        }
    }
    let mut out = Vec::new();
    for s in shown {
        walk(&s.notes, &mut out);
    }
    out
}

/// A Detached annotation of the Work in front, for the Footnotes tab.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetachedView {
    pub id: String,
    /// What it was on: the included Work's name ("Zampanò"), or why it can't be named.
    pub label: String,
    pub quote: String,
    pub body: String,
}

/// Brackets in an Annotation's text must pair up, or the footnote it's printed in would swallow
/// the rest of the Work.
fn check_body(body: &str) -> Result<(), String> {
    // A note on it is an Annotation on the Annotation, never a Footnote inside it.
    if body.contains("#footnote") {
        return Err("An Annotation can't have a Footnote inside it: annotate the Annotation instead.".into());
    }
    let mut depth: i64 = 0;
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("The Annotation's [ and ] don't pair up: write \\[ or \\] for a bracket on its own.".into());
    }
    Ok(())
}

/// An Annotation's note (as recorded) with new text; None if it can't be read, or the new
/// text's brackets don't pair up.
pub fn note_with_body(value: &str, body: &str) -> Option<String> {
    check_body(body).ok()?;
    let mut note: Note = serde_json::from_str(value).ok()?;
    note.body = body.trim().to_string();
    serde_json::to_string(&note).ok()
}

fn new_id() -> Result<String, String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| format!("couldn't make an ID for the Annotation: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl Vault {
    /// The ID of an open Work (its `storage::identity`), which its Annotations are known by.
    fn identity_of_open(&self, id: &str) -> Result<String, String> {
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        work.identity().map(str::to_string).ok_or_else(|| "Write something in the Work first.".into())
    }

    /// Records an Annotation's note (one Change, which undo takes back like typing).
    fn record_note(&mut self, id: &str, ann: &str, note: Option<&Note>, now_ms: u64) -> Result<(), String> {
        let value = match note {
            Some(note) => serde_json::to_string(note).map_err(err)?,
            None => String::new(),
        };
        self.record_inclusions(id, now_ms);
        self.with_work(id, now_ms, |w, _| {
            w.set_annotation(&annotation_record(ann), ANNOTATION_FIELD, &value, now_ms);
            w.save();
            Ok(())
        })?;
        self.refresh_text_file_inclusions(id);
        Ok(())
    }

    /// A new Annotation by the open Work `id` on text selected in one of its Inclusions: `at`
    /// is where the Inclusion's character is in `id`'s text, `from..to` what was selected in
    /// the included text as shown (UTF-16), at any depth of Inclusions, but within one Work's
    /// own text. Returns its ID.
    pub fn annotate(&mut self, id: &str, at: usize, from: usize, to: usize, body: &str, now_ms: u64) -> Result<String, String> {
        check_body(body)?;
        let shown = self.shown_in(id, false)?;
        let text = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?.text();
        let s = inclusion_at(&text, at, &shown).ok_or("That Inclusion isn't there any more.")?;
        if s.missing.is_some() {
            return Err("That Inclusion can't be shown, so nothing in it can be annotated.".into());
        }
        let positions = Positions::new(&s.plain);
        let (from, to) = (positions.to_char(from).map_err(err)?, positions.to_char(to).map_err(err)?);
        if from >= to {
            return Err("Select some of the included text to annotate.".into());
        }
        let (p, b, _, work) = s.all_runs.iter().find(|(p, _, len, _)| from >= *p && to <= p + len).cloned()
            .ok_or("An Annotation can't reach past the edge of one Work's text: select less.")?;
        let quote: String = s.plain.chars().skip(from).take(to - from).collect();
        // The characters, in that Work as it is now.
        let index = self.work_index();
        let located = index.get(&work).cloned().ok_or("The Work it's on can't be found.")?;
        let shelf = crate::preamble::Shelf::read(&self.root);
        let pieces = {
            let mut ctx = Ctx { works: &self.works, index: &index, cache: &mut self.included_docs, shelf: &shelf, device: self.device, recorded: false };
            let doc = ctx.doc(&located, None)?;
            doc.pieces_at(b + from - p, to - from)
        };
        if pieces.is_empty() {
            return Err("That text isn't there any more.".into());
        }
        let note = Note { work, pieces: pieces.iter().map(Span::to_text).collect(), on: None, quote, body: body.trim().to_string() };
        let ann = new_id()?;
        self.record_note(id, &ann, Some(&note), now_ms)?;
        Ok(ann)
    }

    /// A new Annotation by the open Work `id` on a whole Annotation another Work made (`owner`,
    /// its ID; `on`, the Annotation's), shown in one of `id`'s Inclusions. Returns its ID.
    pub fn annotate_annotation(&mut self, id: &str, owner: &str, on: &str, body: &str, now_ms: u64) -> Result<String, String> {
        check_body(body)?;
        let me = self.identity_of_open(id)?;
        if owner == me {
            return Err("That's this Work's own Annotation: change it instead.".into());
        }
        let shown = self.shown_in(id, false)?;
        let target = every_note(&shown).into_iter().find(|n| n.owner == owner && n.id == on)
            .ok_or("That Annotation isn't shown here any more.")?;
        let note = Note { work: owner.to_string(), pieces: Vec::new(), on: Some(on.to_string()), quote: target.body.clone(), body: body.trim().to_string() };
        let ann = new_id()?;
        self.record_note(id, &ann, Some(&note), now_ms)?;
        Ok(ann)
    }

    /// New text for one of the open Work `id`'s Annotations.
    pub fn set_annotation_body(&mut self, id: &str, ann: &str, body: &str, now_ms: u64) -> Result<(), String> {
        check_body(body)?;
        let mut note = self.note_of(id, ann)?;
        if note.body == body.trim() {
            return Ok(());
        }
        note.body = body.trim().to_string();
        self.record_note(id, ann, Some(&note), now_ms)
    }

    /// Deletes one of the open Work `id`'s Annotations (undo, or a Revert, brings it back).
    pub fn delete_annotation(&mut self, id: &str, ann: &str, now_ms: u64) -> Result<(), String> {
        self.note_of(id, ann)?;
        self.record_note(id, ann, None, now_ms)
    }

    fn note_of(&self, id: &str, ann: &str) -> Result<Note, String> {
        let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
        let value = work.doc.field(&annotation_record(ann), ANNOTATION_FIELD).filter(|v| !v.is_empty())
            .ok_or("That Annotation isn't there any more.")?;
        serde_json::from_str(value).map_err(|e| format!("That Annotation can't be read: {e}"))
    }

    /// The open Work `id`'s Detached annotations: none of whose text is shown in its Inclusions
    /// now, in a fixed order, with what they were on.
    pub fn detached_annotations(&mut self, id: &str) -> Result<Vec<DetachedView>, String> {
        let me = self.identity_of_open(id).unwrap_or_default();
        let own = {
            let work = self.works.get(&self.current_id(id)).ok_or_else(|| format!("{id} isn't open."))?;
            annotations_of(&work.doc, &me)
        };
        if own.is_empty() {
            return Ok(Vec::new());
        }
        let shown = self.shown_in(id, false)?;
        let placed: std::collections::HashSet<&str> = every_note(&shown).into_iter().filter(|n| n.owner == me).map(|n| n.id.as_str()).collect();
        let index = self.work_index();
        Ok(own.iter().filter(|a| !placed.contains(a.id.as_str())).map(|a| {
            let label = match index.get(&a.note.work) {
                Some(found) => found.id.rsplit('/').next().unwrap_or(&found.id).to_string(),
                None => "a Work not in this Vault".into(),
            };
            DetachedView { id: a.id.clone(), label, quote: a.note.quote.clone(), body: a.note.body.clone() }
        }).collect())
    }

    /// Names the Works Annotations belong to, for the editor.
    pub(super) fn name_owners(&mut self, notes: &mut [NoteShown], me: &str) {
        let index = self.work_index();
        fn fill(notes: &mut [NoteShown], index: &super::inclusions::Index, me: &str) {
            for n in notes {
                n.mine = n.owner == me;
                n.owner_label = index.get(&n.owner).map(|l| l.id.rsplit('/').next().unwrap_or(&l.id).to_string()).unwrap_or_default();
                fill(&mut n.notes, index, me);
            }
        }
        fill(notes, &index, me);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_reads_back_as_written_and_brackets_must_pair() {
        let note = Note { work: "w".into(), pieces: vec!["00000001:1+0..00000001:1+3".into()], on: None, quote: "word".into(), body: "In _his_ hand".into() };
        let text = serde_json::to_string(&note).unwrap();
        assert_eq!(serde_json::from_str::<Note>(&text).unwrap(), note);
        assert!(check_body("a [b] c").is_ok());
        assert!(check_body(r"a \] c").is_ok());
        assert!(check_body("a ] c").is_err());
        assert!(check_body("a [ c").is_err());
        assert!(check_body("a #footnote[b] c").is_err());
    }
}
