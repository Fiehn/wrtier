//! A Work's Preamble: the Looks it uses (the first is its default), its extra Styles, the
//! Author's own Story elements it uses, its settings and its raw Typst. All of it lives in the
//! Work's History as fields, so it merges and reverts like everything else.
//!
//! The Work also records the content of every Style, Look and Story element it uses (ADR 0004),
//! so a Milestone renders as it was and the `.db` compiles on its own. A changed one is recorded
//! just before this Device's first edit in a sitting; until then the Page preview uses the Style
//! library's version and the Work shows a Style update notice.

use std::collections::HashMap;
use std::path::Path;
use wrtier::change::{Change, PREAMBLE_RECORD};
use wrtier::doc::Doc;
use crate::library::{self, Kind};

/// The fields of the Preamble record. Lists are stored one name per line.
const LOOKS: &str = "looks";
const STYLES: &str = "styles";
const ELEMENTS: &str = "elements";
const TITLE: &str = "title";
const AUTHOR: &str = "author";
const LANGUAGE: &str = "language";
const RAW: &str = "raw";
/// The field of a recorded Style, Look or Story element that holds its file's content.
const CONTENT: &str = "content";

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preamble {
    /// In order: the first is the Work's default, and its Text file's.
    pub looks: Vec<String>,
    /// Applied on top of whichever Look is rendered.
    pub styles: Vec<String>,
    /// The Author's own Story elements this Work uses (the built-in ones it always has).
    pub elements: Vec<String>,
    pub title: String,
    pub author: String,
    /// E.g. "en" or "da": decides quotation marks and hyphenation.
    pub language: String,
    /// Typst for this Work only, after everything else.
    pub raw: String,
}

fn list(doc: &Doc, field: &str) -> Vec<String> {
    doc.field(PREAMBLE_RECORD, field).unwrap_or("").lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// The record holding a Work's recorded copy of a library file.
fn record(kind: Kind, name: &str) -> String {
    match kind {
        Kind::Style => format!("style/{name}"),
        Kind::Look => format!("look/{name}"),
        Kind::StoryElement => format!("element/{name}"),
    }
}

impl Preamble {
    pub fn of(doc: &Doc) -> Preamble {
        let text = |field| doc.field(PREAMBLE_RECORD, field).unwrap_or("").to_string();
        Preamble {
            looks: list(doc, LOOKS),
            styles: list(doc, STYLES),
            elements: list(doc, ELEMENTS),
            title: text(TITLE),
            author: text(AUTHOR),
            language: text(LANGUAGE),
            raw: text(RAW),
        }
    }

    /// Checks what the dialog sent: names without line breaks, a sensible language code.
    pub fn check(&self) -> Result<(), String> {
        for name in self.looks.iter().chain(&self.styles).chain(&self.elements) {
            if name.trim().is_empty() || name.contains('\n') {
                return Err(format!("Not a name in the Style library: {name:?}"));
            }
        }
        let lang = self.language.trim();
        if !lang.is_empty() && (!(2..=3).contains(&lang.len()) || !lang.chars().all(|c| c.is_ascii_lowercase())) {
            return Err("The language is a two- or three-letter code, such as en, da or de.".into());
        }
        Ok(())
    }

    /// The other Device's version of the raw Typst, when two Devices edited it without seeing
    /// each other's edit (the Merge notice).
    pub fn raw_lost(doc: &Doc) -> Option<String> {
        doc.merge_notices().into_iter()
            .filter_map(|n| doc.concurrent_field(n.key))
            .find(|(record, field, _)| record == PREAMBLE_RECORD && field == RAW)
            .map(|(_, _, lost)| lost)
    }

    /// The Changes that make `doc`'s Preamble this one: a field for each part that differs.
    /// The raw Typst is also set again when two Devices' edits of it were in conflict: having
    /// seen both, the Author's choice settles it.
    pub fn record(&self, doc: &mut Doc, now_ms: u64) -> Vec<Change> {
        let current = Preamble::of(doc);
        let settle_raw = Preamble::raw_lost(doc).is_some();
        let mut changes = Vec::new();
        let mut set = |doc: &mut Doc, field: &str, old: String, new: String| {
            if old != new {
                changes.push(doc.set_field_local(PREAMBLE_RECORD, field, &new, now_ms));
            }
        };
        set(doc, LOOKS, current.looks.join("\n"), self.looks.join("\n"));
        set(doc, STYLES, current.styles.join("\n"), self.styles.join("\n"));
        set(doc, ELEMENTS, current.elements.join("\n"), self.elements.join("\n"));
        set(doc, TITLE, current.title, self.title.trim().to_string());
        set(doc, AUTHOR, current.author, self.author.trim().to_string());
        set(doc, LANGUAGE, current.language, self.language.trim().to_string());
        if settle_raw && current.raw == self.raw {
            changes.push(doc.set_field_local(PREAMBLE_RECORD, RAW, &self.raw, now_ms));
        } else {
            set(doc, RAW, current.raw, self.raw.clone());
        }
        changes
    }
}

/// The Style library's files, read once: kind and name → (path in the Vault, content).
#[derive(Default)]
pub struct Shelf {
    files: HashMap<(u8, String), (String, String)>,
}

impl Shelf {
    pub fn read(root: &Path) -> Shelf {
        let mut files = HashMap::new();
        for item in library::list(root).items {
            if let Ok(text) = std::fs::read_to_string(root.join(&item.path)) {
                files.insert((item.kind as u8, item.name), (item.path, text));
            }
        }
        Shelf { files }
    }

    fn get(&self, kind: Kind, name: &str) -> Option<&(String, String)> {
        self.files.get(&(kind as u8, name.to_string()))
    }

    /// The Story elements in the library (built-in and the Author's), for "Add it?".
    pub fn element_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.files.keys().filter(|(k, _)| *k == Kind::StoryElement as u8).map(|(_, n)| n.clone()).collect();
        names.sort();
        names
    }
}

/// One library file a Work uses, as a compile gets it.
#[derive(Clone, Debug, PartialEq)]
pub struct Used {
    pub kind: Kind,
    pub name: String,
    pub content: String,
    /// What an error in it is reported against: its path in the Style library, or where the
    /// copy came from if the library doesn't have it.
    pub label: String,
}

/// Where a compile takes each file from.
#[derive(Clone, Copy, PartialEq)]
pub enum Version {
    /// The Style library's version if it has one (the Page preview), else the Work's own copy.
    Library,
    /// What the Work recorded (a Milestone, a Text file, an Export); the library's version only
    /// for something it never recorded (a new Work, before its first edit).
    Recorded,
}

/// Every library file the Work uses, in the order the header needs them, and what's missing.
pub struct Uses {
    pub elements: Vec<Used>,
    /// Each Look with its Styles, in the Preamble's order.
    pub looks: Vec<(Used, Vec<Used>)>,
    pub styles: Vec<Used>,
    /// Styles given to single Inclusions (ADR 0015): recorded, but not applied to the whole Work.
    pub inclusion_styles: Vec<Used>,
    /// Names the Work uses that can't be found anywhere.
    pub missing: Vec<String>,
}

pub(crate) fn find(doc: &Doc, shelf: &Shelf, version: Version, kind: Kind, name: &str) -> Option<Used> {
    let recorded = || doc.field(&record(kind, name), CONTENT).map(|c| Used {
        kind, name: name.to_string(), content: c.to_string(), label: format!("this Work's copy of {name}"),
    });
    let in_library = || shelf.get(kind, name).map(|(path, content)| Used {
        kind, name: name.to_string(), content: content.clone(), label: path.clone(),
    });
    let built_in = || (kind == Kind::StoryElement).then(|| library::built_in_elements().find(|(n, _)| *n == name)).flatten()
        .map(|(_, content)| Used { kind, name: name.to_string(), content: content.to_string(), label: format!("the app's {name}") });
    match version {
        Version::Library => in_library().or_else(recorded).or_else(built_in),
        Version::Recorded => recorded().or_else(in_library).or_else(built_in),
    }
}

/// What the Work uses: the built-in Story elements, its own, its Looks with their Styles, and
/// its extra Styles.
pub fn uses(doc: &Doc, shelf: &Shelf, version: Version) -> Uses {
    uses_plus(doc, shelf, version, &[])
}

/// The record holding the Author's own Story elements that the Work's Inclusions use, one name
/// per line in its `names` field: the app records it with the versions of the included Works
/// (ADR 0015), and they become part of the Work.
pub const INCLUDED_ELEMENTS: &str = "included/story-elements";

/// The Story elements the Work's Inclusions use, as it recorded them.
pub fn included_elements(doc: &Doc) -> Vec<String> {
    doc.field(INCLUDED_ELEMENTS, "names").unwrap_or("").lines().map(str::trim)
        .filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// `uses`, plus Story elements its Inclusions use now that it hasn't recorded yet (the Page
/// preview shows Inclusions as they are now).
pub fn uses_plus(doc: &Doc, shelf: &Shelf, version: Version, extra_elements: &[String]) -> Uses {
    let mut preamble = Preamble::of(doc);
    for name in included_elements(doc).into_iter().chain(extra_elements.iter().cloned()) {
        if !preamble.elements.contains(&name) {
            preamble.elements.push(name);
        }
    }
    let mut missing = Vec::new();
    let get = |kind: Kind, name: &str, missing: &mut Vec<String>| {
        let found = find(doc, shelf, version, kind, name);
        if found.is_none() {
            let what = match kind { Kind::Style => "Style", Kind::Look => "Look", Kind::StoryElement => "Story element" };
            missing.push(format!("the {what} {name} isn't in the Style library"));
        }
        found
    };
    let mut elements: Vec<Used> = Vec::new();
    for (name, _) in library::built_in_elements() {
        elements.extend(get(Kind::StoryElement, name, &mut missing));
    }
    for name in &preamble.elements {
        if !elements.iter().any(|e| &e.name == name) {
            elements.extend(get(Kind::StoryElement, name, &mut missing));
        }
    }
    let mut looks = Vec::new();
    for name in &preamble.looks {
        let Some(look) = get(Kind::Look, name, &mut missing) else { continue };
        let styles = library::look_styles(&look.content).iter()
            .filter_map(|s| get(Kind::Style, s, &mut missing))
            .collect();
        looks.push((look, styles));
    }
    let styles = preamble.styles.iter().filter_map(|s| get(Kind::Style, s, &mut missing)).collect();
    // Styles given to single Inclusions: recorded like the rest, but applied only inside them.
    let mut inclusion_styles: Vec<Used> = Vec::new();
    for record in doc.records("inclusion/") {
        for name in doc.field(&record, "styles").unwrap_or("").lines().map(str::trim).filter(|l| !l.is_empty()) {
            if !inclusion_styles.iter().any(|s| s.name == name) {
                inclusion_styles.extend(get(Kind::Style, name, &mut missing));
            }
        }
    }
    Uses { elements, looks, styles, inclusion_styles, missing }
}

/// Everything the Work uses whose library version differs from what it recorded (or that it
/// never recorded): (record, content). Recording these is what keeps Milestones as they were.
pub fn unrecorded(doc: &Doc, shelf: &Shelf) -> Vec<(String, String)> {
    let uses = uses(doc, shelf, Version::Library);
    let all = uses.elements.iter()
        .chain(uses.looks.iter().flat_map(|(look, styles)| std::iter::once(look).chain(styles)))
        .chain(&uses.styles)
        .chain(&uses.inclusion_styles);
    let mut found: Vec<(String, String)> = Vec::new();
    for used in all {
        let record = record(used.kind, &used.name);
        if doc.field(&record, CONTENT) != Some(used.content.as_str()) && !found.iter().any(|(r, _)| *r == record) {
            found.push((record, used.content.clone()));
        }
    }
    found
}

/// The Style update notice: what changed in the library since this Work last recorded it.
/// Only for files it had recorded before (a first recording isn't news).
pub fn update_notice(doc: &Doc, shelf: &Shelf) -> Option<String> {
    let changed: Vec<String> = unrecorded(doc, shelf).into_iter()
        .filter(|(record, _)| doc.field(record, CONTENT).is_some())
        .map(|(record, _)| record.split_once('/').map(|(_, n)| n.to_string()).unwrap_or(record))
        .collect();
    if changed.is_empty() {
        return None;
    }
    let names = changed.join(", ");
    let (has, it) = if changed.len() == 1 { ("has", "it") } else { ("have", "them") };
    Some(format!("{names} {has} changed in the Style library since this Work last used {it}. \
        The Page preview shows the new version; your next edit records it in this Work. Milestones keep the old one."))
}

/// The Changes recording every changed library file the Work uses.
pub fn record_unrecorded(doc: &mut Doc, shelf: &Shelf, now_ms: u64) -> Vec<Change> {
    unrecorded(doc, shelf).into_iter().map(|(record, content)| doc.set_field_local(&record, CONTENT, &content, now_ms)).collect()
}

/// A part of the header that came from one file: an error inside it is reported against it.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    /// Byte range in the header.
    pub start: usize,
    pub end: usize,
    pub label: String,
}

/// How a Story element is written, for completing it after a `#`.
#[derive(Clone, Debug, PartialEq)]
pub struct ElementForm {
    pub name: String,
    /// Takes a body (`#dream[…]`), or not (`#scene-break()`).
    pub body: bool,
    /// Its name and what it's for, from its header comment ("Epigraph: A quotation…").
    pub detail: Option<String>,
    /// What a `.docx` Export does with it: its header's ".docx:" line (see `library::DOCX`).
    pub docx: Option<String>,
}

impl ElementForm {
    /// Read from a Story element's file: its function's parameters, and its header comment.
    pub fn of(name: &str, content: &str) -> ElementForm {
        let body = !content.lines().any(|l| l.trim().starts_with(&format!("#let {name}()")));
        let mut title = None;
        let mut docx = None;
        let mut description = Vec::new();
        for line in content.lines().map(str::trim).take_while(|l| l.starts_with("//")) {
            let line = line.trim_start_matches('/').trim();
            if let Some(t) = line.strip_prefix("Story element:") {
                title = Some(t.trim().to_string());
            } else if let Some(d) = line.strip_prefix(".docx:") {
                docx = Some(d.trim().to_string());
            } else if !line.is_empty() && !["Written as:", "Shown as:", ".docx:"].iter().any(|k| line.starts_with(k)) {
                description.push(line);
            }
        }
        let description = description.join(" ");
        let detail = match (title, description.is_empty()) {
            (Some(t), false) => Some(format!("{t}: {description}")),
            (Some(t), true) => Some(t),
            (None, false) => Some(description),
            (None, true) => None,
        };
        ElementForm { name: name.to_string(), body, detail, docx }
    }
}

/// The Typst the app puts before a Work's text: its settings, its Story elements, each of its
/// Looks as a function, the chosen Look, its extra Styles and its raw Typst (ADR 0011).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    pub text: String,
    pub segments: Vec<Segment>,
    /// The Story elements it defines, for the completions' first group.
    pub elements: Vec<ElementForm>,
    /// What's missing (a Look naming a Style the library doesn't have, …): shown as warnings.
    pub missing: Vec<String>,
}

pub use wrtier::text_file::{HEADER_END, HEADER_START};

/// The header of a Work's Text file: its recorded copies, rendered in its first Look, so the
/// file compiles on its own exactly as the Work was recorded (ADR 0011).
pub fn text_file_header(doc: &Doc, shelf: &Shelf) -> String {
    header(doc, uses(doc, shelf, Version::Recorded), None).text
}

/// A Typst string literal.
pub(crate) fn string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace(['\n', '\r'], " "))
}

/// Builds the header. `look`: the Look to render (by name), or the Work's first.
pub fn header(doc: &Doc, uses: Uses, look: Option<&str>) -> Header {
    let preamble = Preamble::of(doc);
    let mut h = Header { missing: uses.missing, ..Header::default() };
    h.text.push_str(HEADER_START);
    let mut document = Vec::new();
    if !preamble.title.is_empty() {
        document.push(format!("title: {}", string(&preamble.title)));
    }
    if !preamble.author.is_empty() {
        document.push(format!("author: {}", string(&preamble.author)));
    }
    if !document.is_empty() {
        h.text.push_str(&format!("#set document({})\n", document.join(", ")));
    }
    if !preamble.language.is_empty() {
        h.text.push_str(&format!("#set text(lang: {})\n", string(&preamble.language)));
    }
    let add = |h: &mut Header, used: &Used| {
        let start = h.text.len();
        h.text.push_str(&used.content);
        if !used.content.ends_with('\n') {
            h.text.push('\n');
        }
        h.segments.push(Segment { start, end: h.text.len(), label: used.label.clone() });
    };
    for element in &uses.elements {
        add(&mut h, element);
        h.elements.push(ElementForm::of(&element.name, &element.content));
    }
    // Each Look wraps the rest of the document in its Styles, lowest wins.
    for (i, (look, styles)) in uses.looks.iter().enumerate() {
        h.text.push_str(&format!("// Look: {}\n#let wrtier-look-{}(doc) = [\n", look.name, i + 1));
        for style in styles {
            add(&mut h, style);
        }
        h.text.push_str("#doc\n]\n");
    }
    let chosen = look.and_then(|name| uses.looks.iter().position(|(l, _)| l.name == name)).unwrap_or(0);
    if !uses.looks.is_empty() {
        h.text.push_str(&format!("#show: wrtier-look-{}\n", chosen + 1));
    }
    // Extra Styles come after the Look, so they win over it, in every Look.
    for style in &uses.styles {
        add(&mut h, style);
    }
    if !preamble.raw.trim().is_empty() {
        add(&mut h, &Used { kind: Kind::Style, name: String::new(), content: preamble.raw.clone(), label: "the Preamble's raw Typst".into() });
    }
    h.text.push_str(HEADER_END);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrtier::ids::DeviceId;

    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        library::write_defaults(dir.path(), |_| false).unwrap();
        dir
    }

    fn with_preamble(p: &Preamble) -> Doc {
        let mut doc = Doc::new(DeviceId(1));
        p.record(&mut doc, 1000);
        doc
    }

    fn paperback() -> Preamble {
        Preamble { looks: vec!["Paperback".into(), "Manuscript".into()], title: "Rain".into(), language: "en".into(), ..Preamble::default() }
    }

    #[test]
    fn the_preamble_round_trips_through_fields() {
        let mut doc = Doc::new(DeviceId(1));
        let p = Preamble { styles: vec!["Book text".into()], elements: vec!["dream".into()], author: "R. L. Fiehn".into(), raw: "#set par(leading: 0.8em)".into(), ..paperback() };
        assert_eq!(p.record(&mut doc, 1000).len(), 7);
        assert_eq!(Preamble::of(&doc), p);
        assert!(p.record(&mut doc, 2000).is_empty(), "nothing changed, nothing recorded");
        let mut q = p.clone();
        q.looks.reverse();
        assert_eq!(q.record(&mut doc, 3000).len(), 1, "reordering the Looks is one Change");
        assert!(Preamble { language: "English".into(), ..p.clone() }.check().is_err());
    }

    #[test]
    fn saving_the_dialog_settles_raw_typst_edited_on_two_devices() {
        let mut laptop = Doc::new(DeviceId(1));
        let mut phone = Doc::new(DeviceId(2));
        let a = Preamble { raw: "#set par(leading: 0.8em)".into(), ..Preamble::default() }.record(&mut phone, 1000);
        let b = Preamble { raw: "#show link: underline".into(), ..Preamble::default() }.record(&mut laptop, 2000);
        laptop.apply_all(&a);
        assert_eq!(Preamble::of(&laptop).raw, "#show link: underline");
        assert_eq!(Preamble::raw_lost(&laptop).as_deref(), Some("#set par(leading: 0.8em)"));
        // Saved as it is (the Author looked at both and kept this one): settled.
        let kept = Preamble::of(&laptop).record(&mut laptop, 3000);
        assert_eq!(kept.len(), 1);
        assert_eq!(Preamble::raw_lost(&laptop), None);
        drop(b);
    }

    #[test]
    fn the_header_compiles_in_every_look_and_extra_styles_win() {
        let dir = vault();
        let shelf = Shelf::read(dir.path());
        let doc = with_preamble(&Preamble { styles: vec!["A4 page".into()], ..paperback() });
        let text = "= The Long Night\n\n#epigraph[Long.]\n\nThe rain had stopped.\n\n#scene-break()\n\nShe went.";
        for look in ["Paperback", "Manuscript"] {
            let h = header(&doc, uses(&doc, &shelf, Version::Library), Some(look));
            assert!(h.missing.is_empty(), "{:?}", h.missing);
            let scene = h.elements.iter().find(|e| e.name == "scene-break").unwrap();
            assert!(!scene.body, "#scene-break() takes no body");
            let epigraph = h.elements.iter().find(|e| e.name == "epigraph").unwrap();
            assert!(epigraph.body);
            assert_eq!(epigraph.detail.as_deref(), Some("Epigraph: A quotation set before a Chapter or Part."));
            let mut t = crate::typeset::Typesetter::new(false);
            let c = t.compile_with("Rain", Path::new("x"), &h, text.to_string());
            assert!(c.errors.is_empty() && c.warnings.is_empty(), "{look}: {:?} {:?}", c.errors, c.warnings);
            // The extra "A4 page" Style sets the page on top of the Look's.
            assert!((c.pages[0].width - 595.3).abs() < 1.0, "{look}: {} pt wide", c.pages[0].width);
        }
    }

    #[test]
    fn missing_files_are_named() {
        let dir = vault();
        let doc = with_preamble(&Preamble { looks: vec!["Hardback".into()], styles: vec!["Drop caps".into()], elements: vec!["dream".into()], ..Preamble::default() });
        let u = uses(&doc, &Shelf::read(dir.path()), Version::Library);
        assert_eq!(u.missing, [
            "the Story element dream isn't in the Style library",
            "the Look Hardback isn't in the Style library",
            "the Style Drop caps isn't in the Style library",
        ]);
    }

    #[test]
    fn a_changed_style_is_recorded_once_and_noticed_until_then() {
        let dir = vault();
        let mut doc = with_preamble(&paperback());
        let shelf = Shelf::read(dir.path());
        // First use: everything is recorded, and that's no news.
        assert_eq!(update_notice(&doc, &shelf), None);
        let first = record_unrecorded(&mut doc, &shelf, 2000);
        assert!(first.len() >= 5 + 2 + 4 + 1, "elements, Looks and Styles: {}", first.len());
        assert!(unrecorded(&doc, &shelf).is_empty());
        // The library's "Book text" changes on another Device.
        std::fs::write(dir.path().join("styles/Book text.typ"), "// Style: Book text\n// Bigger.\n#set text(size: 12pt)\n").unwrap();
        let shelf = Shelf::read(dir.path());
        let notice = update_notice(&doc, &shelf).expect("a notice");
        assert!(notice.starts_with("Book text has changed"), "{notice}");
        // The Page preview already uses the new one; a Milestone's render uses the recorded one.
        let live = uses(&doc, &shelf, Version::Library);
        let recorded = uses(&doc, &shelf, Version::Recorded);
        let book = |u: &Uses| u.looks[0].1.iter().find(|s| s.name == "Book text").unwrap().content.clone();
        assert!(book(&live).contains("12pt") && !book(&recorded).contains("12pt"));
        // The next edit records it, and the notice goes.
        assert_eq!(record_unrecorded(&mut doc, &shelf, 3000).len(), 1);
        assert_eq!(update_notice(&doc, &shelf), None);
    }

    #[test]
    fn a_work_compiles_from_its_own_copies_without_the_library() {
        let dir = vault();
        let mut doc = with_preamble(&paperback());
        record_unrecorded(&mut doc, &Shelf::read(dir.path()), 2000);
        let empty = tempfile::tempdir().unwrap(); // another Vault, no library at all
        let u = uses(&doc, &Shelf::read(empty.path()), Version::Library);
        assert!(u.missing.is_empty(), "{:?}", u.missing);
        assert!(u.looks[0].0.label.contains("this Work's copy"));
    }
}
