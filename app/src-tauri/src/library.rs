//! The Style library: the Vault's `styles/` folder of plain files, so it can be edited in any
//! text editor.
//!
//! - A Style is `styles/<name>.typ`: only `set` and `show` rules.
//! - A Look is `styles/<name>.look`: the names of its Styles, one per line, in stack order.
//! - A Story element is `styles/elements/<name>.typ`: a Typst function named like the file,
//!   with a header comment saying how it's written, how the Formatted view shows it and what
//!   a `.docx` Export does with it.
//!
//! The files have no History. A Conflict copy of one is never merged: the Author picks which
//! version to keep. The defaults the app ships with are written into the Vault once and then
//! belong to it (ADR 0012).

use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use wrtier::conflicts;

/// The library's folder, at the top of the Vault.
pub const FOLDER: &str = "styles";
/// Where Story elements live, inside it.
const ELEMENTS: &str = "elements";

/// How a Story element looks in the Formatted view.
pub const SHOWN_AS: &[&str] = &["chip", "block"];
/// What a `.docx` Export can do with a Story element (v1's fixed menu).
pub const DOCX: &[&str] = &["chapter heading", "part heading", "centred #", "indented block", "centred paragraph", "plain text", "left out"];

/// The version of the defaults below. A later app with better versions of them offers those
/// (never applies them), and adds the ones a Vault has never had.
pub const DEFAULTS_VERSION: &str = "1";

/// The defaults, as paths inside `styles/` and their contents.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("Paperback page.typ", include_str!("../defaults/styles/Paperback page.typ")),
    ("A4 page.typ", include_str!("../defaults/styles/A4 page.typ")),
    ("Book text.typ", include_str!("../defaults/styles/Book text.typ")),
    ("Chapter openings.typ", include_str!("../defaults/styles/Chapter openings.typ")),
    ("Page numbers.typ", include_str!("../defaults/styles/Page numbers.typ")),
    ("Manuscript.typ", include_str!("../defaults/styles/Manuscript.typ")),
    ("Paperback.look", include_str!("../defaults/styles/Paperback.look")),
    ("Manuscript.look", include_str!("../defaults/styles/Manuscript.look")),
    ("A4 draft.look", include_str!("../defaults/styles/A4 draft.look")),
    ("elements/part.typ", include_str!("../defaults/styles/elements/part.typ")),
    ("elements/scene-break.typ", include_str!("../defaults/styles/elements/scene-break.typ")),
    ("elements/epigraph.typ", include_str!("../defaults/styles/elements/epigraph.typ")),
    ("elements/block-text.typ", include_str!("../defaults/styles/elements/block-text.typ")),
    ("elements/dedication.typ", include_str!("../defaults/styles/elements/dedication.typ")),
];

#[derive(Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Style,
    Look,
    StoryElement,
}

/// One file in the library, as read from it.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub kind: Kind,
    /// What the Work refers to it by: the file's name without its extension (for a Story
    /// element, also the name of its function).
    pub name: String,
    /// Its name as the header gives it ("Scene break"), or `name`.
    pub title: String,
    /// Inside the Vault: `styles/Book text.typ`.
    pub path: String,
    /// The header's other lines.
    pub description: String,
    /// What's wrong with it, if anything (it's still listed).
    pub problems: Vec<String>,
    /// A Story element's "Written as", "Shown as" and ".docx" lines.
    pub written_as: Option<String>,
    pub shown_as: Option<String>,
    pub docx: Option<String>,
    /// A Look's Styles, in stack order.
    pub styles: Vec<String>,
}

/// A Conflict copy of a library file: kept until the Author picks a version.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub path: String,
    pub copy: String,
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub items: Vec<Item>,
    pub conflicts: Vec<Conflict>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A path inside the Vault as the Author sees it: `styles/elements/part.typ`.
fn relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Every file under `styles/`.
fn files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.join(FOLDER)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => found.push(path),
                _ => {}
            }
        }
    }
    found.sort();
    found
}

/// What the library's files are now, cheaply: if it changes, something in it changed.
pub fn stamp(root: &Path) -> Vec<(PathBuf, u64, Option<std::time::SystemTime>)> {
    files(root).into_iter().map(|path| {
        let meta = std::fs::metadata(&path).ok();
        let (len, modified) = meta.map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
        (path, len, modified)
    }).collect()
}

/// The header: the comment lines at the top of a file, without their `//`.
fn header(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .take_while(|line| line.starts_with("//"))
        .map(|line| line.trim_start_matches('/').trim())
        .collect()
}

/// A Story element's name is its function's: letters, digits and `-`, starting with a letter.
pub fn is_element_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-')
}

/// Reads one library file. `None` if it isn't one (some other file in `styles/`).
fn read_item(root: &Path, path: &Path, text: &str) -> Option<Item> {
    let ext = path.extension()?.to_str()?;
    let name = path.file_stem()?.to_str()?.to_string();
    let in_elements = path.parent().is_some_and(|p| p == root.join(FOLDER).join(ELEMENTS));
    let in_top = path.parent().is_some_and(|p| p == root.join(FOLDER));
    let kind = match (ext, in_top, in_elements) {
        ("typ", true, _) => Kind::Style,
        ("look", true, _) => Kind::Look,
        ("typ", _, true) => Kind::StoryElement,
        _ => return None,
    };
    let lines = header(text);
    let label = match kind {
        Kind::Style => "Style:",
        Kind::Look => "Look:",
        Kind::StoryElement => "Story element:",
    };
    let mut item = Item {
        kind, title: name.clone(), name, path: relative(root, path),
        description: String::new(), problems: Vec::new(),
        written_as: None, shown_as: None, docx: None, styles: Vec::new(),
    };
    let mut description = Vec::new();
    for line in lines {
        let field = |key: &str| line.strip_prefix(key).map(|v| v.trim().to_string());
        if let Some(title) = field(label) {
            item.title = title;
        } else if let (Kind::StoryElement, Some(v)) = (kind, field("Written as:")) {
            item.written_as = Some(v);
        } else if let (Kind::StoryElement, Some(v)) = (kind, field("Shown as:")) {
            item.shown_as = Some(v);
        } else if let (Kind::StoryElement, Some(v)) = (kind, field(".docx:")) {
            item.docx = Some(v);
        } else if !line.is_empty() && !line.starts_with("One Style per line") {
            description.push(line);
        }
    }
    item.description = description.join(" ");
    match kind {
        Kind::Look => {
            item.styles = text.lines().map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with("//"))
                .map(str::to_string)
                .collect();
            if item.styles.is_empty() {
                item.problems.push("lists no Styles".into());
            }
        }
        Kind::StoryElement => {
            if !is_element_name(&item.name) {
                item.problems.push("its file name must be the function's name: small letters, digits and -, starting with a letter".into());
            } else if !defines(text, &item.name) {
                item.problems.push(format!("doesn't define #{}", item.name));
            }
            match item.shown_as.as_deref() {
                Some(v) if SHOWN_AS.contains(&v) => {}
                Some(v) => item.problems.push(format!("\"Shown as: {v}\" should be chip or block")),
                None => item.problems.push("has no \"Shown as:\" line (chip or block)".into()),
            }
            match item.docx.as_deref() {
                Some(v) if DOCX.contains(&v) => {}
                Some(v) => item.problems.push(format!("\".docx: {v}\" should be one of: {}", DOCX.join(", "))),
                None => item.problems.push("has no \".docx:\" line".into()),
            }
        }
        Kind::Style => {}
    }
    Some(item)
}

/// Does the file define a function (or value) called `name`?
fn defines(text: &str, name: &str) -> bool {
    text.lines().map(str::trim).any(|line| {
        line.strip_prefix("#let ").map(str::trim_start)
            .and_then(|rest| rest.strip_prefix(name))
            .is_some_and(|rest| rest.starts_with('(') || rest.trim_start().starts_with('='))
    })
}

/// Everything in the library, and the Conflict copies waiting for the Author.
pub fn list(root: &Path) -> Library {
    let mut library = Library::default();
    for path in files(root) {
        if let Some(main) = conflicts::main_of(&path) {
            library.conflicts.push(Conflict { path: relative(root, &main), copy: relative(root, &path) });
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        if let Some(item) = read_item(root, &path, &text) {
            library.items.push(item);
        }
    }
    // A Look naming a Style the library doesn't have.
    let styles: Vec<String> = library.items.iter().filter(|i| i.kind == Kind::Style).map(|i| i.name.clone()).collect();
    for look in library.items.iter_mut().filter(|i| i.kind == Kind::Look) {
        for missing in look.styles.iter().filter(|s| !styles.contains(s)) {
            look.problems.push(format!("names a Style the library doesn't have: {missing}"));
        }
    }
    library.items.sort_by_key(|i| (i.kind as u8, i.title.to_lowercase()));
    library.conflicts.sort_by(|a, b| a.copy.cmp(&b.copy));
    library
}

/// A library file's place on disk, from its path inside the Vault. The path comes from the
/// interface, so it must stay inside `styles/` and name a library file.
pub fn file(root: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    let inside = relative.components().all(|c| matches!(c, Component::Normal(_)))
        && relative.components().next().is_some_and(|c| c.as_os_str() == FOLDER);
    let ext_ok = relative.extension().is_some_and(|e| e == "typ" || e == "look");
    if !inside || !ext_ok {
        return Err(format!("{path} isn't a file in the Style library."));
    }
    Ok(root.join(relative))
}

pub fn read(root: &Path, path: &str) -> Result<String, String> {
    std::fs::read_to_string(file(root, path)?).map_err(err)
}

pub fn write(root: &Path, path: &str, text: &str) -> Result<(), String> {
    let file = file(root, path)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    std::fs::write(file, text).map_err(err)
}

/// A new Story element's details, from the form.
#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NewElement {
    pub title: String,
    pub shown_as: String,
    pub docx: String,
    pub description: String,
}

/// Makes a new Style, Look or Story element with a correct header and a starting point, and
/// returns its path. Never overwrites a file.
pub fn create(root: &Path, kind: Kind, name: &str, element: Option<NewElement>) -> Result<String, String> {
    let name = name.trim();
    let (path, text) = match kind {
        Kind::Style | Kind::Look => {
            if name.is_empty() || name.chars().count() > 100 || name.starts_with('.')
                || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
                return Err("A name needs 1 to 100 characters, without / \\ : * ? \" < > |.".into());
            }
            if kind == Kind::Style {
                (format!("{FOLDER}/{name}.typ"), format!("// Style: {name}\n// What this Style does, in a line.\n\n"))
            } else {
                (format!("{FOLDER}/{name}.look"), format!(
                    "// Look: {name}\n// What this Look is for, in a line.\n// One Style per line. When two Styles set the same thing, the one further down wins.\n\n"))
            }
        }
        Kind::StoryElement => {
            let Some(e) = element else { return Err("A Story element needs its details.".into()) };
            if !is_element_name(name) {
                return Err("A Story element's name is how it's written after #: small letters, digits and -, starting with a letter (e.g. dream or radio-message).".into());
            }
            if !SHOWN_AS.contains(&e.shown_as.as_str()) || !DOCX.contains(&e.docx.as_str()) {
                return Err("Pick how it's shown and what a .docx does with it.".into());
            }
            let title = if e.title.trim().is_empty() { name.to_string() } else { e.title.trim().to_string() };
            let description = e.description.trim().replace('\n', " ");
            // A block stands on its own lines; a chip sits inside a line. Either way it labels
            // what it makes, so a Style can restyle it with `show <name>: …`.
            let body = if e.shown_as == "block" {
                format!("#let {name}(body) = [#block(width: 100%, above: 1.4em, below: 1.4em, body) <{name}>]\n")
            } else {
                format!("#let {name}(body) = [#box(body) <{name}>]\n")
            };
            (format!("{FOLDER}/{ELEMENTS}/{name}.typ"), format!(
                "// Story element: {title}\n// Written as: #{name}[…]\n// Shown as: {}\n// .docx: {}\n// {}\n\n{body}",
                e.shown_as, e.docx, if description.is_empty() { "What it's for, in a line." } else { &description }))
        }
    };
    let file = file(root, &path)?;
    if file.exists() {
        return Err(format!("{path} already exists."));
    }
    write(root, &path, &text)?;
    Ok(path)
}

/// Which version to keep when a library file has a Conflict copy.
#[derive(serde::Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Keep {
    /// The file as it is: the copy is deleted.
    Main,
    /// The copy: it replaces the file.
    Copy,
}

pub fn resolve(root: &Path, copy: &str, keep: Keep) -> Result<(), String> {
    let copy_file = file(root, copy)?;
    let main = conflicts::main_of(&copy_file).ok_or_else(|| format!("{copy} isn't a Conflict copy."))?;
    match keep {
        Keep::Main => std::fs::remove_file(&copy_file).map_err(err),
        Keep::Copy => std::fs::rename(&copy_file, &main).map_err(err),
    }
}

/// Writes the defaults the Vault has never had. `had(path)` says whether `vault.db` records
/// one as written already; the paths returned are the ones to record now (including any whose
/// file the Author had already made, which is left as it is).
pub fn write_defaults(root: &Path, had: impl Fn(&str) -> bool) -> Result<Vec<&'static str>, String> {
    let mut written = Vec::new();
    for &(path, text) in DEFAULTS {
        if had(path) {
            continue;
        }
        let file = root.join(FOLDER).join(path);
        if !file.exists() {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(err)?;
            }
            std::fs::write(&file, text).map_err(err)?;
        }
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typeset::Typesetter;

    fn library(dir: &Path) -> Library {
        write_defaults(dir, |_| false).unwrap();
        list(dir)
    }

    #[test]
    fn the_defaults_are_listed_without_problems() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        assert_eq!(lib.items.len(), DEFAULTS.len());
        for item in &lib.items {
            assert!(item.problems.is_empty(), "{}: {:?}", item.path, item.problems);
            assert!(!item.description.is_empty(), "{} has no description", item.path);
        }
        let paperback = lib.items.iter().find(|i| i.name == "Paperback").unwrap();
        assert_eq!(paperback.kind, Kind::Look);
        assert_eq!(paperback.styles, ["Paperback page", "Book text", "Chapter openings", "Page numbers"]);
        let scene = lib.items.iter().find(|i| i.name == "scene-break").unwrap();
        assert_eq!((scene.kind, scene.title.as_str()), (Kind::StoryElement, "Scene break"));
        assert_eq!((scene.shown_as.as_deref(), scene.docx.as_deref()), (Some("chip"), Some("centred #")));
        assert_eq!(scene.path, "styles/elements/scene-break.typ");
    }

    /// Every default Look, with every default Story element, compiles, and a Style can restyle
    /// a Story element by its label.
    #[test]
    fn the_defaults_compile() {
        let dir = tempfile::tempdir().unwrap();
        let lib = library(dir.path());
        let read = |item: &Item| std::fs::read_to_string(dir.path().join(&item.path)).unwrap();
        let elements: String = lib.items.iter().filter(|i| i.kind == Kind::StoryElement).map(read).collect();
        let text = "#dedication[For M.]\n\n#part[Book One]\n\n= The Long Night\n\n#epigraph[The night is long. \\ — Someone]\n\n\
            The rain had _stopped_.\n\n#scene-break()\n\n#block-text[Dear Anna,\n\nCome home.]\n\nShe went.";
        for look in lib.items.iter().filter(|i| i.kind == Kind::Look) {
            let styles: String = look.styles.iter()
                .map(|s| read(lib.items.iter().find(|i| i.kind == Kind::Style && &i.name == s).unwrap()))
                .collect();
            let source = format!("{elements}\n{styles}\n#show <epigraph>: set text(fill: red)\n{text}");
            let mut t = Typesetter::new(false);
            let compiled = t.compile("Rain", Path::new("no-fonts"), source);
            assert!(compiled.errors.is_empty(), "{}: {:?}", look.name, compiled.errors);
            assert!(compiled.warnings.is_empty(), "{}: {:?}", look.name, compiled.warnings);
            // The dedication's page, the Part's, and the Chapter's.
            assert_eq!(compiled.pages.len(), 3, "{}", look.name);
        }
    }

    #[test]
    fn defaults_are_written_once_and_never_over_the_authors_files() {
        let dir = tempfile::tempdir().unwrap();
        let mine = dir.path().join("styles/Book text.typ");
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        std::fs::write(&mine, "// Style: Book text\n// Mine.\n").unwrap();
        let written = write_defaults(dir.path(), |_| false).unwrap();
        assert_eq!(written.len(), DEFAULTS.len(), "every default is recorded, mine included");
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "// Style: Book text\n// Mine.\n");
        // Deleted after being written once: it doesn't come back.
        std::fs::remove_file(dir.path().join("styles/A4 page.typ")).unwrap();
        assert!(write_defaults(dir.path(), |p| written.contains(&p)).unwrap().is_empty());
        assert!(!dir.path().join("styles/A4 page.typ").exists());
    }

    #[test]
    fn problems_are_reported_not_hidden() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "styles/Odd.look", "// Look: Odd\nBook text\nNo such style\n").unwrap();
        write(dir.path(), "styles/elements/dream.typ", "// Story element: Dream\n// Shown as: bubble\n#let sleep(body) = body\n").unwrap();
        write(dir.path(), "styles/Book text.typ", "#set text(size: 11pt)\n").unwrap();
        let lib = list(dir.path());
        let odd = lib.items.iter().find(|i| i.name == "Odd").unwrap();
        assert_eq!(odd.problems, ["names a Style the library doesn't have: No such style"]);
        let dream = lib.items.iter().find(|i| i.name == "dream").unwrap();
        assert!(dream.problems.iter().any(|p| p.contains("doesn't define #dream")));
        assert!(dream.problems.iter().any(|p| p.contains("bubble")));
        assert!(dream.problems.iter().any(|p| p.contains(".docx")));
    }

    #[test]
    fn new_story_elements_are_correct_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = create(dir.path(), Kind::StoryElement, "dream", Some(NewElement {
            title: "Dream".into(), shown_as: "block".into(), docx: "indented block".into(), description: "A dream.".into(),
        })).unwrap();
        assert_eq!(path, "styles/elements/dream.typ");
        let lib = list(dir.path());
        let dream = &lib.items[0];
        assert!(dream.problems.is_empty(), "{:?}", dream.problems);
        assert_eq!(dream.written_as.as_deref(), Some("#dream[…]"));
        // It compiles, and a Style can restyle it by its label.
        let source = format!("{}\n#show <dream>: set text(style: \"italic\")\n#dream[I was falling.]", read(dir.path(), &path).unwrap());
        assert!(Typesetter::new(false).compile("Rain", Path::new("x"), source).errors.is_empty());
        // Never over an existing file, and only sensible names.
        assert!(create(dir.path(), Kind::StoryElement, "dream", Some(NewElement {
            title: String::new(), shown_as: "chip".into(), docx: "plain text".into(), description: String::new(),
        })).is_err());
        assert!(create(dir.path(), Kind::StoryElement, "Dream Sequence", None).is_err());
        assert!(create(dir.path(), Kind::Style, "../escape", None).is_err());
        assert_eq!(create(dir.path(), Kind::Look, "Large print", None).unwrap(), "styles/Large print.look");
    }

    #[test]
    fn conflict_copies_wait_for_the_author() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "styles/A4 page.typ", "// Style: A4 page\n// Mine.\n").unwrap();
        let copy = "styles/A4 page.sync-conflict-20261107-101500-ABCDEFG.typ";
        write(dir.path(), copy, "// Style: A4 page\n// Theirs.\n").unwrap();
        let lib = list(dir.path());
        assert_eq!(lib.items.len(), 1, "the copy isn't listed as a Style of its own");
        assert_eq!(lib.conflicts, [Conflict { path: "styles/A4 page.typ".into(), copy: copy.into() }]);
        resolve(dir.path(), copy, Keep::Copy).unwrap();
        assert!(read(dir.path(), "styles/A4 page.typ").unwrap().contains("Theirs"));
        assert!(list(dir.path()).conflicts.is_empty());
    }

    #[test]
    fn only_library_files_can_be_read_or_written() {
        let dir = tempfile::tempdir().unwrap();
        assert!(file(dir.path(), "styles/Book text.typ").is_ok());
        assert!(file(dir.path(), "styles/../vault.db").is_err());
        assert!(file(dir.path(), "Rain.typ").is_err());
        assert!(file(dir.path(), "styles/notes.txt").is_err());
    }
}
