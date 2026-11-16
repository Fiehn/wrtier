//! The `.docx` Export (ADR 0016): the Work compiled in a Look, or in none (keeping only its
//! prose and structure), turned into named Word styles by the `typst-docx` crate. This is the
//! Wrtier half of it: which kind of text each Story element is, the probe in the Preamble,
//! Comments as Word comments, and the report.

use std::ops::Range;

use typst_docx::{Comment, Note, Reply, Role, Selector, Target, What};

use serde::Serialize;

use crate::preamble::{ElementForm, Header, HEADER_END};
use crate::vault::Made;
use crate::work::Mark;

/// What a Story element's `.docx:` line may say. With no line, its text is paragraphs in a
/// style of its own, named after it.
pub const ROLES: &[&str] = &["chapter heading", "heading above chapters", "left out"];

/// The kinds of text in a Work. Typst's headings come first, so a Chapter is Word's own
/// Heading 1, or Heading 2 throughout when any Story element is a `heading above chapters`
/// (a Part); then each Story element; then what an Inclusion of a whole Work starts with. Each
/// target's origin is the library file its look can be changed in: a Story element's own, or
/// else the Look's (`made`).
pub fn targets(elements: &[ElementForm], made: &Made) -> Vec<Target> {
    let parts = elements.iter().any(|e| e.docx.as_deref() == Some("heading above chapters"));
    let chapter: u8 = if parts { 2 } else { 1 };
    let look = || made.look.as_ref().and_then(|l| l.path.clone());
    let mut out: Vec<Target> = (1..=6u8).map(|level| Target {
        name: if level == 1 { "Chapter".into() } else { format!("Heading {level}") },
        selector: Selector::Heading(usize::from(level)),
        role: Role::Heading((level + chapter - 1).min(9)),
        origin: look(),
    }).collect();
    for e in elements {
        let role = match e.docx.as_deref() {
            Some("chapter heading") => Role::Heading(chapter),
            Some("heading above chapters") => Role::Heading(1),
            Some("left out") => Role::LeftOut,
            _ => Role::Paragraph,
        };
        let name = e.title.clone().unwrap_or_else(|| e.name.clone());
        let origin = made.elements.iter().find(|f| f.name == e.name).and_then(|f| f.path.clone());
        out.push(Target { name, selector: Selector::Label(e.name.clone()), role, origin });
    }
    // `inclusions::wrap`'s title and author line.
    out.push(Target { name: "Inclusion title".into(), selector: Selector::Label("inclusion-title".into()), role: Role::Heading(chapter), origin: look() });
    out.push(Target { name: "Inclusion author".into(), selector: Selector::Label("inclusion-author".into()), role: Role::Paragraph, origin: look() });
    out
}

/// What a `.docx` Export of `input` compiles (its header, with the probe) and converts with:
/// its targets, the Inclusions' scope markers, metadata (titled `name` when the Work has no
/// title), Comments, and the files' hand-written translations in stacking order.
pub fn prepare(input: &crate::vault::ExportInput, name: &str, plain: bool, embed_fonts: bool) -> (Header, typst_docx::Options) {
    let title = if input.preamble.title.trim().is_empty() { name.to_string() } else { input.preamble.title.clone() };
    let header = with_probe(&input.header);
    let options = typst_docx::Options {
        targets: targets(&input.header.elements, &input.made),
        scope: Some((crate::vault::SCOPE_START.into(), crate::vault::SCOPE_END.into())),
        metadata: typst_docx::Metadata { title, author: input.preamble.author.clone(), date: w3c(input.date_ms), seed: seed(&input.ident) },
        plain,
        comments: comments(input.comments.clone(), header.text.len()),
        embed_fonts,
        translations: input.made.translations.iter()
            .map(|file| typst_docx::translate::Translation { origin: file.path.clone(), source: file.content.clone() })
            .collect(),
    };
    (header, options)
}

/// The header with `typst-docx`'s probe at its end, after the Look and the extra Styles, so it
/// reads the settings the Work's text is set in.
pub fn with_probe(header: &Header) -> Header {
    let mut h = header.clone();
    let at = h.text.rfind(HEADER_END).unwrap_or(h.text.len());
    h.text.insert_str(at, &typst_docx::probe::snippet());
    h
}

/// "Anna Berg" → "AB".
fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(3).flat_map(char::to_uppercase).collect()
}

/// A time (ms since 1970) as W3CDTF, as Word dates comments and documents.
pub fn w3c(ms: u64) -> String {
    let (y, mo, d, h, mi, s) = crate::typeset::utc_parts(ms);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// A seed for whatever in the file must look random: from the Work's ID, so the same version
/// gives the same file on every Device.
pub fn seed(ident: &str) -> u64 {
    // FNV-1a
    ident.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// The Comments as Word comments, under their senders' names; `shift`: how far the body is
/// into the compiled source (the header's length).
pub fn comments(found: Vec<(Mark, Vec<Range<usize>>)>, shift: usize) -> Vec<Comment> {
    found.into_iter().map(|(m, ranges)| Comment {
        author: m.name.clone(),
        initials: initials(&m.name),
        date: w3c(m.time_ms),
        text: m.body.clone(),
        ranges: ranges.into_iter().map(|r| r.start + shift..r.end + shift).collect(),
        replies: m.replies.iter().map(|r| Reply { author: r.name.clone(), initials: initials(&r.name), date: w3c(r.time_ms), text: r.body.clone() }).collect(),
        origin: Some(m.name),
    }).collect()
}

/// A line of the report, and the library file to open to do something about it, if there's one.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub open: Option<String>,
}

/// The report, a line per item: what the converter noted (`notes`), and Styles whose hand-written
/// translations were partly left out for mistakes (not for prose only, which uses none).
pub fn report(notes: &[Note], made: &Made, plain: bool) -> Vec<Line> {
    let look = made.look.as_ref().and_then(|l| l.path.clone());
    let mut lines: Vec<Line> = notes.iter().map(|n| match &n.what {
        What::FormattedDirectly { paragraphs } => {
            // Word's own heading styles are named "heading 1", shown as "Heading 1".
            let style = n.style.as_deref().unwrap_or("A style");
            let mut chars = style.chars();
            let style: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
            let many = if *paragraphs == 1 { "1 paragraph doesn't".to_string() } else { format!("{paragraphs} paragraphs don't") };
            Line { text: format!("{style}: {many} look like the rest, so they're formatted directly."), open: n.origin.clone().or(look.clone()) }
        }
        What::FontNotEmbedded { family, why } => Line {
            text: format!("{family} is only named, not embedded ({why}): whoever opens it needs that font installed."), open: None,
        },
        What::FurnitureSimplified => Line {
            text: "The headers and footers change from page to page in a way Word can't repeat, so only the page number is kept.".into(),
            open: look.clone(),
        },
        What::CommentNotPlaced { text } => {
            let by = n.origin.as_deref().unwrap_or("someone");
            Line { text: format!("A Comment by {by} (“{text}”) isn't in it: the text it's on isn't either."), open: None }
        }
    }).collect();
    if !plain {
        for file in &made.translations {
            let problems = typst_docx::translate::problems(&file.content);
            if let Some(first) = problems.first() {
                let more = match problems.len() {
                    1 => String::new(),
                    n => format!(", and {} more", n - 1),
                };
                lines.push(Line { text: format!("{}'s .docx translations were partly left out: {first}{more}.", file.name), open: file.path.clone() });
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(name: &str, docx: Option<&str>) -> ElementForm {
        ElementForm { name: name.into(), body: true, detail: None, title: Some(name.to_uppercase()), docx: docx.map(String::from) }
    }

    #[test]
    fn chapters_go_a_level_down_under_parts() {
        let role = |targets: &[Target], name: &str| targets.iter().find(|t| t.name == name).map(|t| t.role);
        let plain = targets(&[element("epigraph", None)], &Made::default());
        assert_eq!(role(&plain, "Chapter"), Some(Role::Heading(1)));
        assert_eq!(role(&plain, "EPIGRAPH"), Some(Role::Paragraph));
        let parted = targets(&[element("part", Some("heading above chapters")), element("note", Some("left out")), element("chapter", Some("chapter heading"))], &Made::default());
        assert_eq!(role(&parted, "PART"), Some(Role::Heading(1)));
        assert_eq!(role(&parted, "Chapter"), Some(Role::Heading(2)));
        assert_eq!(role(&parted, "Heading 2"), Some(Role::Heading(3)));
        assert_eq!(role(&parted, "CHAPTER"), Some(Role::Heading(2)));
        assert_eq!(role(&parted, "NOTE"), Some(Role::LeftOut));
        assert_eq!(role(&parted, "Inclusion title"), Some(Role::Heading(2)));
        // Typst's Chapter heading is listed before an element at its level, so it's Word's "Heading 2".
        assert!(parted.iter().position(|t| t.name == "Chapter") < parted.iter().position(|t| t.name == "CHAPTER"));
    }

    #[test]
    fn the_probe_goes_at_the_end_of_the_header() {
        let header = Header { text: format!("#show: wrtier-look-1\n{HEADER_END}"), ..Header::default() };
        let probed = with_probe(&header);
        assert!(probed.text.starts_with("#show: wrtier-look-1\n#context"));
        assert!(probed.text.ends_with(HEADER_END));
    }

    #[test]
    fn report_lines_open_the_file_to_change() {
        use crate::vault::File;
        let file = |name: &str, path: &str, content: &str| File { name: name.into(), path: Some(path.into()), content: content.into() };
        let made = Made {
            translations: vec![file("Book text", "styles/Book text.typ", "/* .docx\nNormal <w:i/>\n*/")],
            elements: vec![file("epigraph", "styles/elements/epigraph.typ", "")],
            look: Some(file("Paperback", "styles/Paperback.look", "")),
        };
        let targets = targets(&[element("epigraph", None)], &made);
        assert_eq!(targets.iter().find(|t| t.name == "EPIGRAPH").unwrap().origin.as_deref(), Some("styles/elements/epigraph.typ"));
        let notes = [
            Note { style: Some("heading 1".into()), origin: Some("styles/Paperback.look".into()), what: What::FormattedDirectly { paragraphs: 2 } },
            Note { style: Some("Normal".into()), origin: None, what: What::FormattedDirectly { paragraphs: 1 } },
            Note { style: None, origin: Some("Anna".into()), what: What::CommentNotPlaced { text: "Hm".into() } },
        ];
        let lines = report(&notes, &made, false);
        assert_eq!(lines[0], Line { text: "Heading 1: 2 paragraphs don't look like the rest, so they're formatted directly.".into(), open: Some("styles/Paperback.look".into()) });
        assert_eq!(lines[1].open.as_deref(), Some("styles/Paperback.look"), "prose is the Look's");
        assert_eq!(lines[2].open, None);
        assert_eq!(lines[3], Line {
            text: "Book text's .docx translations were partly left out: line 2: should be a style's name, a colon, then its properties.".into(),
            open: Some("styles/Book text.typ".into()),
        });
        assert_eq!(report(&[], &made, true), [], "prose only uses no translations");
    }

    #[test]
    fn names_dates_and_seeds() {
        assert_eq!(initials("Anna  berg"), "AB");
        assert_eq!(w3c(1_700_000_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(seed("a"), seed("a"));
        assert_ne!(seed("a"), seed("b"));
    }
}
