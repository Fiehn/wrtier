//! The History view's numbers tie together: on a text written and deleted a whole word at a
//! time (no Moves, no Reverts), the words every Session added minus those it removed add up
//! to the words in the text now, and so do the words a summary of all of it counts.
//!
//! The words include dialogue punctuation: a closing quote after a comma, a dash, an apostrophe
//! inside a word. Only the tokens with a letter or digit in them are words.

use proptest::prelude::*;
use wrtier::doc::{Doc, SESSION_GAP_MS};
use wrtier::ids::DeviceId;

/// What can be written: each starts with a space, so it never joins the words around it.
const TOKENS: &[&str] = &[" rain", " ‘Yes,’", " don’t", " —", " ’", " 1914", " Ånd", " '", " gulls."];

/// Is a token a word? It has a letter or digit.
fn is_word(token: &str) -> bool {
    token.chars().any(char::is_alphanumeric)
}

#[derive(Debug, Clone)]
enum Action {
    Write { at: u16, token: usize },
    Erase { at: u16 },
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => (any::<u16>(), 0..TOKENS.len()).prop_map(|(at, token)| Action::Write { at, token }),
        1 => any::<u16>().prop_map(|at| Action::Erase { at }),
    ]
}

fn run(actions: &[Action]) {
    let mut doc = Doc::new(DeviceId(1));
    let mut tokens: Vec<&str> = Vec::new();
    let mut now = 1_000u64;
    let mut all = Vec::new();
    for action in actions {
        now += SESSION_GAP_MS + 1; // each one a Session of its own
        // Where token `k` starts in the text, in characters.
        let start = |tokens: &[&str], k: usize| tokens[..k].iter().map(|t| t.chars().count()).sum::<usize>();
        match action {
            Action::Write { at, token } => {
                let k = *at as usize % (tokens.len() + 1);
                all.push(doc.insert_local(start(&tokens, k), TOKENS[*token], now).id);
                tokens.insert(k, TOKENS[*token]);
            }
            Action::Erase { at } => {
                if tokens.is_empty() {
                    continue;
                }
                let k = *at as usize % tokens.len();
                all.push(doc.delete_local(start(&tokens, k), tokens[k].chars().count(), now).id);
                tokens.remove(k);
            }
        }
        assert_eq!(doc.text(), tokens.concat());
    }
    let words = tokens.iter().filter(|t| is_word(t)).count() as i64;
    let sessions = doc.sessions();
    let net: i64 = sessions.iter().map(|s| s.added_words as i64 - s.removed_words as i64).sum();
    assert_eq!(net, words, "Sessions: {:?}", sessions.iter().map(|s| (s.added_words, s.removed_words)).collect::<Vec<_>>());
    if let Some(summary) = doc.summary(&all) {
        assert_eq!(summary.added_words as i64 - summary.removed_words as i64, words, "the summary of all of it");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn added_minus_removed_words_is_the_words_now(actions in prop::collection::vec(action(), 1..40)) {
        run(&actions);
    }
}
