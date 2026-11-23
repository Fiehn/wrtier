//! The Grammar checker (CONTEXT.md, ADR 0023): Harper's grammar and punctuation rules, run over
//! the paragraphs the interface sends, as the text reads. The interface reads the text
//! (grammarmod.ts) and makes the Marks; this side only says what Harper found in each paragraph.
//! The same parse gives each word's type, for the Syntax highlighting Mod (`word_types`).
//!
//! All of it runs on Harper's own thread (harper.rs).
//!
//! Nothing of Harper is loaded until a Work with the Grammar checker asks its first question:
//! its dictionary (about 60 MB, half a second) and a set of rules for the Work's kind of English
//! (about 12 MB). The dictionary is Harper's own static and stays for the rest of the run; the
//! rules are kept here, one set per kind of English asked for.
//!
//! Harper's `SpellCheck` rule is never on: the Spellchecker spells (ADR 0019). Every other rule
//! is as Harper ships it, plus our own `CommaSplice` (comma_splice.rs), less those switched off
//! (the interface sends which: the Work's, and those off by default, grammarmod.ts). Nothing
//! about a Work is kept between calls: the rules' own caches go by the text, and hold 64 entries
//! (the patch).

use crate::comma_splice;
use harper_core::linting::{LintGroup, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document, TokenKind};
use serde::Serialize;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Harper's own spelling rule: always off.
const SPELLING: &str = "SpellCheck";

/// Something Harper found in a paragraph. `from`..`to` are in UTF-16 units of the paragraph as
/// sent, as JavaScript counts.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub from: usize,
    pub to: usize,
    /// The rule's name in Harper ("AnA"): what dismissing it and switching it off are kept under.
    pub rule: String,
    /// Harper's kind of finding ("Grammar", "Typo", "Style"…): which kind of Mark it is.
    pub kind: String,
    /// One line in plain words.
    pub message: String,
    pub suggestions: Vec<Fix>,
}

/// One of Harper's Suggestions for what a finding covers: "replace" it with `text`, "insert"
/// `text` after it, or "remove" it.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub kind: &'static str,
    pub text: String,
}

/// The kind of English a Work in `language` is checked as: its region's, where Harper has it
/// (GB, US, CA, AU, IN), and British otherwise, as for no region at all. None if it isn't
/// English.
pub fn dialect(language: &str) -> Option<Dialect> {
    let language = language.trim();
    let (lang, region) = language.split_once('-').unwrap_or((language, ""));
    if !(lang.is_empty() || lang.eq_ignore_ascii_case("en")) {
        return None;
    }
    Some(Dialect::try_from_abbr(&region.to_ascii_uppercase()).unwrap_or(Dialect::British))
}

/// Harper's rules for one kind of English, and which of them a Work switched off last.
struct Rules {
    dialect: Dialect,
    group: LintGroup,
    /// The rules on as Harper ships them: those to switch back on when a Work stops switching one off.
    shipped_on: HashSet<String>,
    off: Vec<String>,
}

/// Harper's rules, loaded when first asked for.
#[derive(Default)]
pub struct Grammar {
    rules: Vec<Rules>,
}

impl Grammar {
    /// What Harper finds in each of `paragraphs` of a Work in `language`, with the rules in `off`
    /// switched off. None if the Work isn't in English (nothing is checked).
    pub fn check(&mut self, language: &str, paragraphs: &[String], off: &[String]) -> Option<Vec<Vec<Found>>> {
        let dialect = dialect(language)?;
        let dictionary = FstDictionary::curated();
        Some(paragraphs.iter().map(|text| {
            let group = self.rules_for(dialect, off);
            match catch_unwind(AssertUnwindSafe(|| lint(group, &dictionary, text))) {
                Ok(found) => found,
                // A paragraph Harper can't read gets no Marks; its rules are built afresh next time,
                // in case the panic left them half-way through something.
                Err(_) => {
                    self.rules.retain(|r| r.dialect != dialect);
                    Vec::new()
                }
            }
        }).collect())
    }

    /// The rules for `dialect`, built if they aren't yet, with exactly `off` switched off.
    fn rules_for(&mut self, dialect: Dialect, off: &[String]) -> &mut LintGroup {
        let i = match self.rules.iter().position(|r| r.dialect == dialect) {
            Some(i) => i,
            None => {
                let mut group = LintGroup::new_curated(FstDictionary::curated(), dialect);
                group.config.set_rule_enabled(SPELLING, false);
                // Ours, beside Harper's. If Harper ever ships a rule of the same name, the test below says so.
                if group.add(comma_splice::NAME, comma_splice::CommaSplice) {
                    group.config.set_rule_enabled(comma_splice::NAME, true);
                }
                let shipped_on = group.iter_keys().filter(|k| group.config.is_rule_enabled(k)).map(String::from).collect();
                self.rules.push(Rules { dialect, group, shipped_on, off: Vec::new() });
                self.rules.len() - 1
            }
        };
        let rules = &mut self.rules[i];
        if rules.off != off {
            for rule in &rules.off {
                if rules.shipped_on.contains(rule) {
                    rules.group.config.set_rule_enabled(rule, true);
                }
            }
            for rule in off {
                rules.group.config.set_rule_enabled(rule, false);
            }
            rules.group.config.set_rule_enabled(SPELLING, false);
            rules.off = off.to_vec();
        }
        &mut rules.group
    }

    /// The names of the rules Harper has, and whether each is on as it ships (SpellCheck never is).
    #[cfg(test)]
    fn rule_names(&mut self) -> Vec<(String, bool)> {
        let rules: &LintGroup = self.rules_for(Dialect::British, &[]);
        let mut names: Vec<(String, bool)> = rules.iter_keys().map(|k| (k.to_string(), rules.config.is_rule_enabled(k))).collect();
        names.sort();
        names
    }
}

/// Harper counts characters; JavaScript counts UTF-16 units. Where each character of `text`
/// starts in UTF-16 units, and then where it ends.
fn utf16_starts(text: &str) -> Vec<usize> {
    let mut utf16 = Vec::with_capacity(text.len() + 1);
    let mut at = 0;
    for c in text.chars() {
        utf16.push(at);
        at += c.len_utf16();
    }
    utf16.push(at);
    utf16
}

fn lint(group: &mut LintGroup, dictionary: &FstDictionary, text: &str) -> Vec<Found> {
    let document = Document::new(text, &PlainEnglish, dictionary);
    let utf16 = utf16_starts(text);
    let last = utf16.len() - 1;
    let mut found = Vec::new();
    for (rule, lints) in group.organized_lints(&document) {
        for lint in lints {
            let (start, end) = (lint.span.start.min(last), lint.span.end.min(last));
            if start >= end {
                continue;
            }
            found.push(Found {
                from: utf16[start],
                to: utf16[end],
                rule: rule.clone(),
                kind: format!("{:?}", lint.lint_kind),
                message: plain(&lint.message),
                suggestions: lint.suggestions.iter().map(fix).collect(),
            });
        }
    }
    found.sort_by(|a, b| (a.from, a.to, &a.rule).cmp(&(b.from, b.to, &b.rule)));
    found
}

// ---- Word types, for the Syntax highlighting Mod ----

/// A word and its type where it stands, as Harper reads it: `from`..`to` in UTF-16 units of the
/// paragraph as sent.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Word {
    pub from: usize,
    pub to: usize,
    /// Its Universal POS tag, as Harper names it: "NOUN", "PROPN", "VERB", "AUX", "ADJ", "ADV",
    /// "CCONJ", "SCONJ", "PRON", "DET", "ADP", "PART", "NUM", "INTJ"…
    pub tag: String,
}

/// The type of each word in each of `paragraphs` of a Work in `language`, from the same parse
/// Harper's rules read (its tagger looks at the words around each one: "record/VERB a
/// record/NOUN"). None if the Work isn't in English. Only Harper's dictionary is loaded, not its rules.
///
/// A word Harper's dictionary doesn't know has no type, except one that starts with a capital,
/// which is taken for a name (PROPN): a character's or a place's, made up. Others are left out.
pub fn word_types(language: &str, paragraphs: &[String]) -> Option<Vec<Vec<Word>>> {
    dialect(language)?;
    let dictionary = FstDictionary::curated();
    Some(paragraphs.iter().map(|text| catch_unwind(AssertUnwindSafe(|| words_of(&dictionary, text))).unwrap_or_default()).collect())
}

fn words_of(dictionary: &FstDictionary, text: &str) -> Vec<Word> {
    let document = Document::new(text, &PlainEnglish, dictionary);
    let source = document.get_source();
    let utf16 = utf16_starts(text);
    let last = utf16.len() - 1;
    let mut words = Vec::new();
    for token in document.tokens() {
        let TokenKind::Word(meta) = &token.kind else { continue };
        let tag = match meta {
            Some(meta) => match meta.pos_tag {
                Some(tag) => format!("{tag:?}"),
                None => continue,
            },
            None if source.get(token.span.start).is_some_and(|c| c.is_uppercase()) => "PROPN".to_string(),
            None => continue,
        };
        let (start, end) = (token.span.start.min(last), token.span.end.min(last));
        if start < end {
            words.push(Word { from: utf16[start], to: utf16[end], tag });
        }
    }
    words
}

fn fix(s: &Suggestion) -> Fix {
    match s {
        Suggestion::ReplaceWith(c) => Fix { kind: "replace", text: c.iter().collect() },
        Suggestion::InsertAfter(c) => Fix { kind: "insert", text: c.iter().collect() },
        Suggestion::Remove => Fix { kind: "remove", text: String::new() },
    }
}

/// Harper's messages are Markdown: without its marks, they read as plain words.
fn plain(message: &str) -> String {
    message.replace("**", "").replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(grammar: &mut Grammar, language: &str, text: &str, off: &[&str]) -> Vec<Found> {
        let off: Vec<String> = off.iter().map(|s| s.to_string()).collect();
        grammar.check(language, &[text.to_string()], &off).expect("English").remove(0)
    }

    fn rules(found: &[Found]) -> Vec<&str> {
        found.iter().map(|f| f.rule.as_str()).collect()
    }

    #[test]
    fn a_work_is_checked_as_its_kind_of_english_and_british_without_one() {
        assert_eq!(dialect("en-GB"), Some(Dialect::British));
        assert_eq!(dialect("en-us"), Some(Dialect::American));
        assert_eq!(dialect("en-AU"), Some(Dialect::Australian));
        assert_eq!(dialect("en"), Some(Dialect::British));
        assert_eq!(dialect(""), Some(Dialect::British));
        assert_eq!(dialect("en-ZA"), Some(Dialect::British));
        assert_eq!(dialect("da"), None);
        assert_eq!(dialect("de-CH"), None);
        // Not English: nothing is checked, and nothing of Harper is loaded for it.
        let mut grammar = Grammar::default();
        assert_eq!(grammar.check("da", &["Det regner.".to_string()], &[]), None);
        assert!(grammar.rules.is_empty());
    }

    #[test]
    fn it_finds_grammar_with_its_place_in_utf16_units_and_its_suggestions() {
        let mut grammar = Grammar::default();
        // An emoji is two UTF-16 units but one character: what comes after it is counted as JavaScript counts.
        let text = "😀 She bought a apple.";
        let found = check(&mut grammar, "en-GB", text, &[]);
        let an = found.iter().find(|f| f.rule == "AnA").unwrap_or_else(|| panic!("{found:?}"));
        let utf16: Vec<u16> = text.encode_utf16().collect();
        assert_eq!(String::from_utf16_lossy(&utf16[an.from..an.to]), "a");
        assert!(an.suggestions.iter().any(|s| s.kind == "replace" && s.text == "an"), "{an:?}");
        assert!(!an.message.contains("**") && !an.message.contains('`'));
    }

    #[test]
    fn it_never_spells_since_the_spellchecker_does() {
        let mut grammar = Grammar::default();
        let found = check(&mut grammar, "en-GB", "I will recieve teh letter tomorow.", &[]);
        assert!(!rules(&found).contains(&SPELLING), "{found:?}");
        assert!(found.iter().all(|f| f.kind != "Spelling"), "{found:?}");
        assert!(grammar.rule_names().iter().any(|(name, on)| name == SPELLING && !on));
    }

    #[test]
    fn a_rule_the_work_switched_off_is_off_until_it_is_switched_on_again() {
        let mut grammar = Grammar::default();
        let text = "She bought a apple.";
        assert!(rules(&check(&mut grammar, "en-US", text, &[])).contains(&"AnA"));
        assert!(!rules(&check(&mut grammar, "en-US", text, &["AnA"])).contains(&"AnA"));
        assert!(rules(&check(&mut grammar, "en-US", text, &[])).contains(&"AnA"));
        // A name Harper hasn't got (from an older or newer version) changes nothing.
        assert!(rules(&check(&mut grammar, "en-US", text, &["NoSuchRule"])).contains(&"AnA"));
    }

    #[test]
    fn comma_splices_are_found_beside_harpers_rules_and_can_be_switched_off() {
        let mut grammar = Grammar::default();
        let text = "She reached the door, it was locked.";
        let found = check(&mut grammar, "en-GB", text, &[]);
        let splice = found.iter().find(|f| f.rule == comma_splice::NAME).unwrap_or_else(|| panic!("{found:?}"));
        assert_eq!(&text[splice.from..splice.to], ", it");
        assert_eq!(splice.kind, "Punctuation");
        assert_eq!(splice.suggestions, [Fix { kind: "replace", text: ". It".into() }, Fix { kind: "replace", text: "; it".into() }]);
        assert!(!rules(&check(&mut grammar, "en-GB", text, &[comma_splice::NAME])).contains(&comma_splice::NAME));
        assert!(rules(&check(&mut grammar, "en-GB", text, &[])).contains(&comma_splice::NAME));
        // Harper has no rule of its own by that name: ours was added, not dropped as a clash.
        assert!(grammar.rule_names().iter().any(|(name, on)| name == comma_splice::NAME && *on));
    }

    #[test]
    fn the_rules_off_by_default_are_harpers() {
        // grammarmod.ts OFF_BY_DEFAULT: an upgrade that renames one shows up here.
        let mut grammar = Grammar::default();
        let names = grammar.rule_names();
        for rule in ["LongSentences", "OxfordComma", "BoringWords", "Excellent", "SentenceCapitalization"] {
            assert!(names.iter().any(|(name, _)| name == rule), "{rule}");
        }
    }

    /// Each word of `text` with its type, as "word/TAG".
    fn typed(text: &str) -> Vec<String> {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        word_types("en-GB", &[text.to_string()]).expect("English").remove(0).iter()
            .map(|w| format!("{}/{}", String::from_utf16_lossy(&utf16[w.from..w.to]), w.tag))
            .collect()
    }

    #[test]
    fn a_word_gets_its_type_from_where_it_stands() {
        let words = typed("I will record a record.");
        assert!(words.contains(&"record/VERB".to_string()), "{words:?}");
        assert!(words.contains(&"record/NOUN".to_string()), "{words:?}");
        let words = typed("She walked slowly and quietly to the old house.");
        for w in ["walked/VERB", "slowly/ADV", "and/CCONJ", "old/ADJ", "house/NOUN"] {
            assert!(words.contains(&w.to_string()), "{w} in {words:?}");
        }
    }

    #[test]
    fn a_word_it_does_not_know_is_a_name_if_it_has_a_capital_and_nothing_otherwise() {
        let words = typed("Kvothe smiled at the glimmerous lights.");
        assert!(words.contains(&"Kvothe/PROPN".to_string()), "{words:?}");
        assert!(!words.iter().any(|w| w.starts_with("glimmerous/")), "{words:?}");
    }

    #[test]
    fn word_types_are_placed_in_utf16_units_and_only_for_english() {
        // The emoji is two UTF-16 units: "cat" is counted as JavaScript counts.
        let words = typed("😀 The cat sat.");
        assert!(words.contains(&"cat/NOUN".to_string()), "{words:?}");
        assert_eq!(word_types("da", &["Katten sad.".to_string()]), None);
        assert_eq!(word_types("en", &[String::new()]), Some(vec![vec![]]));
    }

    #[test]
    fn rule_names_fit_where_dismissals_and_switches_are_kept() {
        // vault/marks.rs keeps a Dismissed Mark as "<mod>/<rule>", letters, digits and hyphens only.
        let mut grammar = Grammar::default();
        let names = grammar.rule_names();
        assert!(names.len() > 500, "{}", names.len());
        let bad: Vec<_> = names.iter().filter(|(n, _)| n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).collect();
        assert!(bad.is_empty(), "{bad:?}");
    }
}
