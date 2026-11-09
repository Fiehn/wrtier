//! Typst colouring for the Raw view and Style library tabs.
//!
//! The colours come from Typst's own parser and highlighter, so they always agree with how
//! Typst reads the text; the editor has no second grammar. The editor sends a piece of text (a
//! whole library file, or the paragraphs on screen in a Work) and gets back coloured ranges in
//! its own positions (UTF-16 code units, counted from the start of that piece).

use serde::Serialize;
use typst::syntax::{highlight, LinkedNode, Tag};

/// One coloured stretch of text. Ranges can nest (a heading holding a string, say).
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Colour {
    pub from: usize,
    pub to: usize,
    /// The CSS class, such as `typ-func`; the theme's `--syntax-…` variables colour it.
    pub class: &'static str,
}

/// What to colour `text` with, read as Typst markup (as a Work or a Style file is).
pub fn colour(text: &str) -> Vec<Colour> {
    let root = typst::syntax::parse(text);
    // Every byte offset that starts a character, as a UTF-16 position.
    let mut utf16 = vec![0; text.len() + 1];
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        utf16[byte] = units;
        units += c.len_utf16();
    }
    utf16[text.len()] = units;

    let mut colours = Vec::new();
    walk(&LinkedNode::new(&root), &utf16, &mut colours);
    colours
}

fn walk(node: &LinkedNode, utf16: &[usize], colours: &mut Vec<Colour>) {
    // Errors aren't coloured: half-typed markup would flash red while writing, and the Page
    // preview already reports real errors.
    if let Some(tag) = highlight(node).filter(|&tag| tag != Tag::Error) {
        let range = node.range();
        if !range.is_empty() {
            colours.push(Colour { from: utf16[range.start], to: utf16[range.end], class: tag.css_class() });
        }
    }
    for child in node.children() {
        walk(&child, utf16, colours);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str) -> Vec<(&str, &'static str)> {
        let units: Vec<u16> = text.encode_utf16().collect();
        colour(text)
            .into_iter()
            .map(|c| {
                let slice = String::from_utf16(&units[c.from..c.to]).unwrap();
                (Box::leak(slice.into_boxed_str()) as &str, c.class)
            })
            .collect()
    }

    #[test]
    fn code_in_a_style_file() {
        let found = classes("// Page size\n#set page(width: 148mm, numbering: \"1\")\n");
        assert!(found.contains(&("// Page size", "typ-comment")), "{found:?}");
        assert!(found.contains(&("set", "typ-key")), "{found:?}");
        assert!(found.contains(&("page", "typ-func")), "{found:?}");
        assert!(found.contains(&("148mm", "typ-num")), "{found:?}");
        assert!(found.contains(&("\"1\"", "typ-str")), "{found:?}");
    }

    #[test]
    fn markup_in_a_work() {
        let found = classes("= One\nThe _rain_ had *stopped*. \\* <here>\n#scene-break()\n");
        assert!(found.contains(&("= One", "typ-heading")), "{found:?}");
        assert!(found.contains(&("_rain_", "typ-emph")), "{found:?}");
        assert!(found.contains(&("*stopped*", "typ-strong")), "{found:?}");
        assert!(found.contains(&("\\*", "typ-escape")), "{found:?}");
        assert!(found.contains(&("<here>", "typ-label")), "{found:?}");
        assert!(found.contains(&("scene-break", "typ-func")), "{found:?}");
    }

    #[test]
    fn positions_count_utf16_units() {
        // "😀" is two UTF-16 units and four bytes; "ø" is one unit and two bytes.
        let text = "😀 ø _x_";
        let emph = colour(text).into_iter().find(|c| c.class == "typ-emph").unwrap();
        assert_eq!((emph.from, emph.to), (5, 8));
    }

    #[test]
    fn half_typed_markup_isnt_an_error_colour() {
        assert!(colour("#set page(").iter().all(|c| c.class != "typ-error"));
    }
}
