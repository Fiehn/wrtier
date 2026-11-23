//! The Grammar checker (CONTEXT.md, ADR 0023): Harper's grammar and punctuation rules, run over
//! the paragraphs the interface sends, as the text reads. The interface reads the text
//! (grammarmod.ts) and makes the Marks; this side only says what Harper found in each paragraph.
//!
//! Nothing of Harper is loaded until a Work with the Grammar checker asks its first question:
//! its dictionary (about 60 MB, half a second) and a set of rules for the Work's kind of English
//! (about 12 MB). The dictionary is Harper's own static and stays for the rest of the run; the
//! rules are kept here, one set per kind of English asked for.
//!
//! Harper's `SpellCheck` rule is never on: the Spellchecker spells (ADR 0019). Every other rule
//! is as Harper ships it, less those the Work has switched off. Nothing about a Work is kept
//! between calls: the rules' own caches go by the text, and hold 64 entries (the patch).

use harper_core::linting::{LintGroup, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
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

fn lint(group: &mut LintGroup, dictionary: &FstDictionary, text: &str) -> Vec<Found> {
    let document = Document::new(text, &PlainEnglish, dictionary);
    // Harper counts characters; JavaScript counts UTF-16 units. Where each character starts, and the end.
    let mut utf16 = Vec::with_capacity(text.len() + 1);
    let mut at = 0;
    for c in text.chars() {
        utf16.push(at);
        at += c.len_utf16();
    }
    utf16.push(at);
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
    fn rule_names_fit_where_dismissals_and_switches_are_kept() {
        // vault/marks.rs keeps a Dismissed Mark as "<mod>/<rule>", letters, digits and hyphens only.
        let mut grammar = Grammar::default();
        let names = grammar.rule_names();
        assert!(names.len() > 500, "{}", names.len());
        let bad: Vec<_> = names.iter().filter(|(n, _)| n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).collect();
        assert!(bad.is_empty(), "{bad:?}");
    }
}
