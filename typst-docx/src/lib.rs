//! Turns a compiled Typst document into an editable `.docx` whose formatting lives in named Word
//! styles, one per target (a label or a heading level), so whoever opens it can restyle every
//! epigraph, or every chapter heading, in one place.
//!
//! The caller compiles the document with its own `World`; this crate only reads the result.
//! It knows Typst and OOXML, nothing about the program that calls it. The document must hold
//! `probe::snippet()`, after the caller's own `set` rules, so the page and paragraph settings
//! can be read as Typst resolved them.
//!
//! Formatting comes from layers, each over the one before: what's measured on the laid-out
//! pages for each target (`measure`), then the page and paragraph settings from the probe.

pub mod measure;
pub mod probe;
pub mod fonts;
mod furniture;
mod ooxml;
mod styles;
pub mod translate;

pub use measure::Selector;
use typst::syntax::Source;
use typst_layout::PagedDocument;

/// What a target becomes in the `.docx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Paragraphs in a style of its own, named after the target.
    Paragraph,
    /// Word's built-in "Heading n" (1 to 9), which Word's navigation and contents use.
    Heading(u8),
    /// Not in the `.docx` at all (and not reported).
    LeftOut,
}

/// A kind of text that gets one named style.
#[derive(Clone, Debug)]
pub struct Target {
    /// The style's name in Word (a heading's is Word's own).
    pub name: String,
    pub selector: Selector,
    pub role: Role,
    /// Where its formatting can be changed, in the caller's terms; handed back in the report.
    pub origin: Option<String>,
}

/// About the document as a whole.
#[derive(Clone, Debug, Default)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    /// When it was written, as W3CDTF (`2026-11-16T10:00:00Z`): the document's date, so the
    /// same input gives the same file.
    pub date: String,
    /// For anything in the file that must look random but be the same every time.
    pub seed: u64,
}

/// A Word comment: on the text that came from `ranges` of the main source, or on the document
/// as a whole (no ranges), with its replies.
#[derive(Clone, Debug, Default)]
pub struct Comment {
    pub author: String,
    /// Shown in Word beside the comment, such as "RF".
    pub initials: String,
    /// W3CDTF, like `Metadata::date`.
    pub date: String,
    /// Its text; each line a paragraph.
    pub text: String,
    /// Byte ranges in the main source. The comment covers the text from the first character
    /// that came from any of them to the last (a Word comment covers one stretch). Empty: it's
    /// on the document as a whole, and sits at its start.
    pub ranges: Vec<std::ops::Range<usize>>,
    pub replies: Vec<Reply>,
    /// Handed back in the report if it can't be placed.
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub author: String,
    pub initials: String,
    pub date: String,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub targets: Vec<Target>,
    /// Labels of two `metadata` markers bracketing a stretch set its own way (see
    /// `measure::Options::scope`); its text gets styles of its own, named after the start
    /// marker's value.
    /// A stretch may set its own page: the document then also holds `probe::scope_snippet()`
    /// for the end marker, and the stretch's pages are sections of their own.
    pub scope: Option<(String, String)>,
    pub metadata: Metadata,
    /// Keep only the structure and emphasis: each target keeps its named style, but the styles
    /// are plain (a book draft in Times New Roman), the page is Word's own, and the only direct
    /// formatting is italic and bold where the text differs from what's around it.
    pub plain: bool,
    pub comments: Vec<Comment>,
    /// Embed the fonts the text is set in, as far as their licences allow (not when `plain`),
    /// so it looks right where they aren't installed; else they're only named.
    pub embed_fonts: bool,
    /// Sources with hand-written translations (`translate`), in stacking order: a later one
    /// wins. Not used when `plain`.
    pub translations: Vec<translate::Translation>,
}

/// Something on the pages the `.docx` can't hold as Typst set it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Uncarried {
    /// An image: left out.
    Image,
    /// A line or shape drawn among the text (a box's border, a highlight): left out.
    Drawing,
    /// An equation: its symbols are kept, in a row, as text.
    Equation,
    /// A table or grid: each cell's text is kept as a paragraph of its own.
    Table,
}

/// Something the report says.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// This many paragraphs didn't fit their style and were formatted directly.
    FormattedDirectly { paragraphs: usize },
    /// The headers and footers differ from page to page in a way Word can't repeat, so only the
    /// page number was kept, where it's most often shown.
    FurnitureSimplified,
    /// A font the text is set in that's only named, not embedded: the reader needs it
    /// installed. `why`: its licence doesn't allow it, or its outlines couldn't be converted.
    FontNotEmbedded { family: String, why: String },
    /// A comment none of whose text is in the `.docx` (it's in something left out, say); `text`
    /// is the start of the comment's own text, to recognise it by.
    CommentNotPlaced { text: String },
    /// Something the `.docx` doesn't carry as it was set (once for each place it comes from):
    /// `at`, where it starts in the main source (byte offset), if it came from there. `origin`
    /// is the target's it's in, if any.
    NotCarried { what: Uncarried, at: Option<usize> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// The Word style it's about, if any.
    pub style: Option<String>,
    /// The origin of what it's about (a target's, a comment's), if that has one.
    pub origin: Option<String>,
    pub what: What,
}

pub struct Docx {
    pub bytes: Vec<u8>,
    pub report: Vec<Note>,
}

/// The `.docx` of a compiled document. `main`: the source file it was compiled from, which
/// comments are anchored in.
pub fn convert(document: &PagedDocument, main: &Source, options: &Options) -> Result<Docx, String> {
    let setups = probe::read_all(document, if options.plain { None } else { options.scope.as_ref() })
        .ok_or_else(|| "the document has no probe (typst_docx::probe::snippet())".to_string())??;
    let setup = &setups.main;
    let measuring = measure::Options {
        targets: options.targets.iter().map(|t| measure::Target { name: t.name.clone(), selector: t.selector.clone() }).collect(),
        scope: if options.plain { None } else { options.scope.clone() },
    };
    let measured = measure::measure_in(document, &measuring, |page, scope| setups.of(scope).area(page), main, !options.comments.is_empty());
    let mut laid = styles::lay_out(&measured.units, &options.targets, setup);
    if options.plain {
        styles::make_plain(&mut laid);
    }
    let furniture = if options.plain {
        furniture::plain(furniture::Look { family: "Times New Roman".into(), size: 12.0, italic: false, bold: false, fill: "000000".into(), small_caps: false })
    } else {
        let pages = document.pages().len();
        // A page is set as the text it starts with is (a page with none, as the one before).
        let mut scopes: Vec<Option<&str>> = vec![None; pages];
        let mut seen = vec![false; pages];
        for u in measured.units.iter().filter(|u| u.note.is_none() && u.page < pages) {
            if !seen[u.page] {
                (seen[u.page], scopes[u.page]) = (true, u.scope.as_deref());
            }
        }
        for p in 1..pages {
            if !seen[p] {
                scopes[p] = scopes[p - 1];
            }
        }
        let areas: Vec<measure::Area> = (0..pages).map(|p| setups.of(scopes[p]).area(p)).collect();
        furniture::read(document, &areas, &ooxml::section_starts(&laid, &setups, pages), &styles::titles(&laid, pages))
    };
    let (families, not_embedded) = if options.embed_fonts && !options.plain {
        fonts::collect(document, options.metadata.seed)
    } else {
        (Vec::new(), Vec::new())
    };
    for (family, why) in not_embedded {
        let why = match why {
            fonts::NotEmbedded::Licence => "its licence doesn't allow embedding it".to_string(),
            fonts::NotEmbedded::Unconvertible(why) => format!("it couldn't be made TrueType: {why}"),
        };
        laid.notes.push(Note { style: None, origin: None, what: What::FontNotEmbedded { family, why } });
    }
    for s in measured.skipped {
        let target = s.target.map(|t| &options.targets[t]);
        if target.is_some_and(|t| t.role == Role::LeftOut) {
            continue; // left out with what it's in
        }
        laid.notes.push(Note { style: None, origin: target.and_then(|t| t.origin.clone()), what: What::NotCarried { what: s.what, at: s.at } });
    }
    let hands = if options.plain { Vec::new() } else { translate::merged(&options.translations) };
    let (parts, unplaced) = ooxml::parts(&laid, &setups, &options.metadata, &options.comments, &furniture, &families, &hands);
    laid.notes.extend(unplaced);
    let bytes = ooxml::zip(&parts)?;
    Ok(Docx { bytes, report: laid.notes })
}
