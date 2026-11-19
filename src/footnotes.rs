//! Footnotes (CONTEXT.md): `#footnote[…]` written in a Work's own text. The History engine only
//! needs to see past them where it names things after the text: a Chapter's title is its
//! heading without any Footnote written in it.

const HEAD: &str = "#footnote[";

/// Some text with every `#footnote[…]` taken out, nested ones and all. One that doesn't close
/// is left as it is (it isn't a Footnote to Typst either).
pub fn without_footnotes(text: &str) -> String {
    if !text.contains(HEAD) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(HEAD) {
        // `\#footnote[` is text, not a Footnote.
        if rest[..at].ends_with('\\') {
            out.push_str(&rest[..at + HEAD.len()]);
            rest = &rest[at + HEAD.len()..];
            continue;
        }
        match closing(&rest[at + HEAD.len()..]) {
            Some(len) => {
                out.push_str(&rest[..at]);
                rest = &rest[at + HEAD.len() + len + 1..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Where the `]` matching an open `[` just before `text` is (a byte offset), skipping escaped
/// and nested brackets.
fn closing(text: &str) -> Option<usize> {
    let mut depth = 1;
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footnotes_are_taken_out_nested_and_escaped_ones_aside() {
        assert_eq!(without_footnotes("The End#footnote[Or is it?]"), "The End");
        assert_eq!(without_footnotes("A#footnote[b #footnote[c] d]B"), "AB");
        assert_eq!(without_footnotes(r"A\#footnote[b]"), r"A\#footnote[b]");
        assert_eq!(without_footnotes("A#footnote[open"), "A#footnote[open");
        assert_eq!(without_footnotes("A#footnote[x \\] y]B"), "AB");
        assert_eq!(without_footnotes("No notes"), "No notes");
    }
}
