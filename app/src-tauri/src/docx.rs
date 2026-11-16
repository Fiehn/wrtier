//! The `.docx` Export (ADR 0016): the Work compiled in a Look, or in none (keeping only its
//! prose and structure), turned into named Word styles by the `typst-docx` crate. This is the
//! Wrtier half of it: which kind of text each Story element is, the probe in the Preamble,
//! Comments as Word comments, and the report.

use std::ops::Range;

use typst_docx::{Comment, Note, Reply, Role, Selector, Target, What};

use crate::preamble::{ElementForm, Header, HEADER_END};
use crate::work::Mark;

/// What a Story element's `.docx:` line may say. With no line, its text is paragraphs in a
/// style of its own, named after it.
pub const ROLES: &[&str] = &["chapter heading", "heading above chapters", "left out"];

/// The kinds of text in a Work. Typst's headings come first, so a Chapter is Word's own
/// Heading 1, or Heading 2 throughout when any Story element is a `heading above chapters`
/// (a Part); then each Story element; then what an Inclusion of a whole Work starts with.
pub fn targets(elements: &[ElementForm]) -> Vec<Target> {
    let parts = elements.iter().any(|e| e.docx.as_deref() == Some("heading above chapters"));
    let chapter: u8 = if parts { 2 } else { 1 };
    let look = || Some("the Look".to_string());
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
        let origin = Some(format!("the Story element {name}"));
        out.push(Target { name, selector: Selector::Label(e.name.clone()), role, origin });
    }
    // `inclusions::wrap`'s title and author line.
    out.push(Target { name: "Inclusion title".into(), selector: Selector::Label("inclusion-title".into()), role: Role::Heading(chapter), origin: look() });
    out.push(Target { name: "Inclusion author".into(), selector: Selector::Label("inclusion-author".into()), role: Role::Paragraph, origin: look() });
    out
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

/// The report, a line per item.
pub fn report(notes: &[Note]) -> Vec<String> {
    notes.iter().map(|n| match &n.what {
        What::FormattedDirectly { paragraphs } => {
            // Word's own heading styles are named "heading 1", shown as "Heading 1".
            let style = n.style.as_deref().unwrap_or("A style");
            let mut chars = style.chars();
            let style: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
            let many = if *paragraphs == 1 { "1 paragraph doesn't".to_string() } else { format!("{paragraphs} paragraphs don't") };
            let from = n.origin.as_deref().map(|o| format!(", as {o} sets them") ).unwrap_or_default();
            format!("{style}: {many} look like the rest{from}, so they're formatted directly.")
        }
        What::CommentNotPlaced { text } => {
            let by = n.origin.as_deref().unwrap_or("someone");
            format!("A Comment by {by} (“{text}”) isn't in it: the text it's on isn't either.")
        }
    }).collect()
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
        let plain = targets(&[element("epigraph", None)]);
        assert_eq!(role(&plain, "Chapter"), Some(Role::Heading(1)));
        assert_eq!(role(&plain, "EPIGRAPH"), Some(Role::Paragraph));
        let parted = targets(&[element("part", Some("heading above chapters")), element("note", Some("left out")), element("chapter", Some("chapter heading"))]);
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
    fn names_dates_and_seeds() {
        assert_eq!(initials("Anna  berg"), "AB");
        assert_eq!(w3c(1_700_000_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(seed("a"), seed("a"));
        assert_ne!(seed("a"), seed("b"));
    }
}
