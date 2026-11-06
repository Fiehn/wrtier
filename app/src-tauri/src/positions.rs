//! Positions at the boundary between the interface and the engine.
//!
//! The interface counts positions in UTF-16 code units, as CodeMirror and JavaScript do. The
//! engine counts characters. The two differ by one for every character outside the Basic
//! Multilingual Plane, such as an emoji, so every position is converted here, and one the engine
//! must never see (past the end, or in the middle of such a character) is rejected.

use std::fmt;

/// Where the text's two-unit characters are, so converting a position is a binary search.
/// With none (most text), both directions are the identity.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Positions {
    /// The character index of every two-unit character, in order.
    wide: Vec<usize>,
    chars: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PositionError {
    PastTheEnd { position: usize, len: usize },
    InsideACharacter { position: usize },
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PositionError::PastTheEnd { position, len } => write!(f, "position {position} is past the end of the text ({len})"),
            PositionError::InsideACharacter { position } => write!(f, "position {position} is in the middle of a character"),
        }
    }
}

impl std::error::Error for PositionError {}

impl Positions {
    pub fn new(text: &str) -> Self {
        let mut positions = Positions::default();
        for (i, c) in text.chars().enumerate() {
            if c.len_utf16() == 2 {
                positions.wide.push(i);
            }
            positions.chars = i + 1;
        }
        positions
    }

    /// The text's length in UTF-16 code units.
    pub fn utf16_len(&self) -> usize {
        self.chars + self.wide.len()
    }

    /// The number of two-unit characters that end at or before UTF-16 position `utf16`.
    /// (The j-th one starts at `wide[j] + j`, which grows with j, so this is a binary search.)
    fn wide_ending_by(&self, utf16: usize) -> usize {
        let (mut lo, mut hi) = (0, self.wide.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.wide[mid] + mid + 2 <= utf16 { lo = mid + 1 } else { hi = mid }
        }
        lo
    }

    /// A position from the interface as a character index, if the engine may see it.
    pub fn to_char(&self, utf16: usize) -> Result<usize, PositionError> {
        if utf16 > self.utf16_len() {
            return Err(PositionError::PastTheEnd { position: utf16, len: self.utf16_len() });
        }
        let before = self.wide_ending_by(utf16);
        if self.wide.get(before).is_some_and(|&i| i + before + 1 == utf16) {
            return Err(PositionError::InsideACharacter { position: utf16 });
        }
        Ok(utf16 - before)
    }

    /// A character index from the engine as a position for the interface.
    pub fn to_utf16(&self, char_index: usize) -> usize {
        char_index + self.wide.partition_point(|&i| i < char_index)
    }

    /// Keeps up with an edit: `removed` characters at character index `at` replaced by
    /// `inserted`. Only the two-unit characters after the edit shift, and those are rare.
    pub fn edit(&mut self, at: usize, removed: usize, inserted: &str) {
        let start = self.wide.partition_point(|&i| i < at);
        let end = self.wide.partition_point(|&i| i < at + removed);
        let added = inserted.chars().count();
        for i in &mut self.wide[end..] {
            *i = *i + added - removed;
        }
        let new = inserted.chars().enumerate().filter(|(_, c)| c.len_utf16() == 2).map(|(i, _)| at + i);
        self.wide.splice(start..end, new);
        self.chars = self.chars + added - removed;
    }
}

/// The byte index in `text` of UTF-16 position `utf16` (or the end).
pub fn byte_at_utf16(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

/// `text` with UTF-16 positions `from..to` replaced by `insert`.
pub fn replace_utf16(text: &str, from: usize, to: usize, insert: &str) -> String {
    let (a, b) = (byte_at_utf16(text, from), byte_at_utf16(text, to));
    format!("{}{}{}", &text[..a], insert, &text[b..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_the_same_both_ways() {
        let p = Positions::new("abc");
        assert_eq!(p.utf16_len(), 3);
        for i in 0..=3 {
            assert_eq!(p.to_char(i), Ok(i));
            assert_eq!(p.to_utf16(i), i);
        }
        assert_eq!(p.to_char(4), Err(PositionError::PastTheEnd { position: 4, len: 3 }));
    }

    #[test]
    fn an_emoji_takes_two_positions_in_the_interface() {
        // a 😊 b 😊 😊 c: 6 characters, 9 UTF-16 code units
        let text = "a😊b😊😊c";
        let p = Positions::new(text);
        assert_eq!(p.utf16_len(), 9);
        assert_eq!(p.utf16_len(), text.encode_utf16().count());
        // For each UTF-16 position 0..=9: the character index, or Err if it splits an emoji.
        let expected: [Result<usize, ()>; 10] = [Ok(0), Ok(1), Err(()), Ok(2), Ok(3), Err(()), Ok(4), Err(()), Ok(5), Ok(6)];
        for (utf16, want) in expected.into_iter().enumerate() {
            match want {
                Ok(c) => {
                    assert_eq!(p.to_char(utf16), Ok(c), "UTF-16 position {utf16}");
                    assert_eq!(p.to_utf16(c), utf16, "character {c}");
                }
                Err(_) => assert_eq!(p.to_char(utf16), Err(PositionError::InsideACharacter { position: utf16 })),
            }
        }
        assert!(matches!(p.to_char(10), Err(PositionError::PastTheEnd { .. })));
    }

    #[test]
    fn matches_javascript_on_every_boundary() {
        let text = "Hé 😊 𝄞 漢字 \u{10FFFF}!";
        let p = Positions::new(text);
        let mut utf16 = 0;
        for (i, c) in text.chars().enumerate() {
            assert_eq!(p.to_char(utf16), Ok(i));
            assert_eq!(p.to_utf16(i), utf16);
            utf16 += c.len_utf16();
        }
        assert_eq!(p.to_char(utf16), Ok(text.chars().count()));
    }

    #[test]
    fn empty_text() {
        let p = Positions::new("");
        assert_eq!(p.to_char(0), Ok(0));
        assert!(p.to_char(1).is_err());
    }

    #[test]
    fn edits_keep_it_the_same_as_starting_over() {
        // A fixed-seed run of random edits over text with emoji, checked against a fresh scan.
        let pieces = ["a", "😊", "bc", "𝄞x", "", "漢字😊", "\n"];
        let mut text: Vec<char> = "start 😊 here".chars().collect();
        let mut p = Positions::new(&text.iter().collect::<String>());
        let mut seed: u64 = 42;
        let mut next = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n.max(1)
        };
        for _ in 0..500 {
            let at = next(text.len() + 1);
            let removed = next(text.len() - at + 1).min(3);
            let inserted = pieces[next(pieces.len())];
            text.splice(at..at + removed, inserted.chars());
            p.edit(at, removed, inserted);
            assert_eq!(p, Positions::new(&text.iter().collect::<String>()));
        }
    }

    #[test]
    fn replacing_by_utf16_position() {
        assert_eq!(replace_utf16("a😊b", 3, 4, "X"), "a😊X");
        assert_eq!(replace_utf16("a😊b", 1, 3, ""), "ab");
        assert_eq!(replace_utf16("ab", 2, 2, "!"), "ab!");
    }
}
