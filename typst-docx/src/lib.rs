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
mod ooxml;
mod styles;

pub use measure::Selector;
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

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub targets: Vec<Target>,
    /// Labels of two `metadata` markers bracketing a stretch set its own way (see
    /// `measure::Options::scope`); its text gets styles of its own, named after the start
    /// marker's value.
    pub scope: Option<(String, String)>,
    pub metadata: Metadata,
}

/// Something the report says about a style.
#[derive(Clone, Debug, PartialEq)]
pub enum What {
    /// This many paragraphs didn't fit their style and were formatted directly.
    FormattedDirectly { paragraphs: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// The Word style it's about.
    pub style: String,
    /// The target's origin, if the style comes from a target that has one.
    pub origin: Option<String>,
    pub what: What,
}

pub struct Docx {
    pub bytes: Vec<u8>,
    pub report: Vec<Note>,
}

/// The `.docx` of a compiled document.
pub fn convert(document: &PagedDocument, options: &Options) -> Result<Docx, String> {
    let setup = probe::read(document)
        .ok_or_else(|| "the document has no probe (typst_docx::probe::snippet())".to_string())??;
    let measuring = measure::Options {
        targets: options.targets.iter().map(|t| measure::Target { name: t.name.clone(), selector: t.selector.clone() }).collect(),
        scope: options.scope.clone(),
    };
    let units = measure::measure_in(document, &measuring, |page| setup.area(page));
    let laid = styles::lay_out(&units, &options.targets, &setup);
    let bytes = ooxml::zip(&ooxml::parts(&laid, &setup, &options.metadata))?;
    Ok(Docx { bytes, report: laid.notes })
}
