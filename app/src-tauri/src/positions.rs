//! Positions at the boundary between the interface and the engine.
//!
//! The interface counts positions in UTF-16 code units, as CodeMirror and JavaScript do. The
//! engine counts characters. The two differ by one for every character outside the Basic
//! Multilingual Plane, such as an emoji, so every position is converted here, and one the engine
//! must never see (past the end, or in the middle of such a character) is rejected.

use std::fmt;

/// Where the text's two-unit characters are, so converting a position is a binary search.
/// With none (most text), both directions are the identity.
#[derive(Debug, Default)]
pub struct Positions {
    /// The character index of every two-unit character, in order.
    wide_chars: Vec<usize>,
    /// The UTF-16 position where each of those characters starts.
    wide_starts: Vec<usize>,
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
                positions.wide_starts.push(i + positions.wide_chars.len());
                positions.wide_chars.push(i);
            }
            positions.chars = i + 1;
        }
        positions
    }

    /// The text's length in UTF-16 code units.
    pub fn utf16_len(&self) -> usize {
        self.chars + self.wide_chars.len()
    }

    /// A position from the interface as a character index, if the engine may see it.
    pub fn to_char(&self, utf16: usize) -> Result<usize, PositionError> {
        if utf16 > self.utf16_len() {
            return Err(PositionError::PastTheEnd { position: utf16, len: self.utf16_len() });
        }
        // Two-unit characters that end at or before the position.
        let before = self.wide_starts.partition_point(|&start| start + 2 <= utf16);
        if self.wide_starts.get(before).is_some_and(|&start| start + 1 == utf16) {
            return Err(PositionError::InsideACharacter { position: utf16 });
        }
        Ok(utf16 - before)
    }

    /// A character index from the engine as a position for the interface.
    pub fn to_utf16(&self, char_index: usize) -> usize {
        char_index + self.wide_chars.partition_point(|&i| i < char_index)
    }
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
}
