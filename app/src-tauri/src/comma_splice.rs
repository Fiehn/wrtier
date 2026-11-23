//! The Grammar checker's own rule `CommaSplice` (CONTEXT.md "Comma splice", docs/v3-plan.md): two
//! sentences joined by only a comma ("She reached the door, it was locked."). Harper has nothing
//! for these, so it's ours, added to Harper's rules (grammar.rs) and read from the same parse:
//! Harper's word types in context, and its verb forms.
//!
//! Harper can't find clauses, so this guesses from the words, and would rather miss a splice than
//! mark good prose. A comma is marked when:
//! - before it, since the sentence began (or a semicolon, colon, dash or quote), ends a clause
//!   that could stand alone: a subject and a finite verb that no subordinating or relative word
//!   claims ("When she reached the door, it was locked", "and though his pride was hurt, he
//!   suffered" are fine);
//! - straight after it comes a subject and a finite verb, with no joining word ("and") or
//!   relative word ("which") between.
//!
//! What counts as subject and verb comes in two tiers. A personal pronoun (I, you, he, she, it,
//! we, they, or one with its verb: "it's") takes any verb that can be finite. Any other subject
//! (a noun phrase or a name) needs a verb that can only be finite there: an auxiliary ("was"),
//! a past tense unlike its participle ("broke"), a present "-s" form, or a past form with its
//! object after it ("clenched the rail"). So absolute phrases are never marked ("He turned, his
//! hands clenched", "his coat flapping"), at the cost of missing "The wind rose, the shutters
//! rattled."
//!
//! Never marked: anything inside double quotes (people talk in comma splices), clauses in a
//! series closed by a joining word ("I came, I saw, and I conquered"), and "he said" or "I
//! believe" set off by commas. Imperatives before the comma ("Come here, I need you") are missed.

use harper_core::linting::{Lint, LintKind, Linter, Suggestion};
use harper_core::{DictWordMetadata, Document, Punctuation, Span, TokenKind};

pub const NAME: &str = "CommaSplice";

#[derive(Debug, Clone, Copy, Default)]
pub struct CommaSplice;

impl Linter for CommaSplice {
    fn lint(&mut self, document: &Document) -> Vec<Lint> {
        let items = items_of(document);
        let mut lints = Vec::new();
        for (c, item) in items.iter().enumerate() {
            if !matches!(item.kind, Kind::Comma) || item.quoted {
                continue;
            }
            let Some(next) = items.get(c + 1).filter(|n| matches!(n.kind, Kind::Word(_))) else { continue };
            let Some(v) = clause_at(&items, c + 1) else { continue };
            if parenthetical(&items, v) || !clause_before(&items, c) || in_closed_series(&items, c) {
                continue;
            }
            lints.push(lint(document, item.span, next.span));
        }
        lints
    }

    fn description(&self) -> &str {
        "Looks for comma splices: two sentences joined by only a comma, where a full stop or a semicolon belongs."
    }
}

/// The Mark: the comma and the word after it (", it"), with a full stop (". It") and a semicolon ("; it").
fn lint(document: &Document, comma: Span<char>, word: Span<char>) -> Lint {
    let source = document.get_source();
    let after: String = source[comma.end..word.end].iter().collect();
    let mut chars = after.chars();
    let mut stop = String::from(".");
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            stop.push(ch);
        } else {
            stop.extend(ch.to_uppercase());
            stop.extend(chars);
            break;
        }
    }
    Lint {
        span: Span::new(comma.start, word.end),
        lint_kind: LintKind::Punctuation,
        suggestions: vec![
            Suggestion::ReplaceWith(stop.chars().collect()),
            Suggestion::ReplaceWith(format!(";{after}").chars().collect()),
        ],
        message: "Two sentences joined by only a comma.".to_string(),
        ..Default::default()
    }
}

// ---- The paragraph as words and the punctuation between them ----

/// A word's type, as Harper's tagger read it where it stands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tag {
    Noun,
    Propn,
    Verb,
    Aux,
    Adj,
    Adv,
    Det,
    Pron,
    Cconj,
    Sconj,
    Adp,
    Num,
    Other,
    /// Harper's tagger said nothing.
    Unknown,
}

struct Word<'a> {
    /// Lower case, with ’ as '.
    text: String,
    tag: Tag,
    meta: Option<&'a DictWordMetadata>,
    capital: bool,
}

enum Kind<'a> {
    Word(Word<'a>),
    Comma,
    /// Where a clause can't run across: a sentence's end, a semicolon, a colon, a dash, a quote, a bracket.
    Edge,
    Other,
}

struct Item<'a> {
    kind: Kind<'a>,
    span: Span<char>,
    /// Inside double quotes: dialogue.
    quoted: bool,
}

fn items_of(document: &Document) -> Vec<Item<'_>> {
    let source = document.get_source();
    let mut items = Vec::new();
    let mut quoted = false;
    for token in document.get_tokens() {
        let kind = match &token.kind {
            TokenKind::Space(_) | TokenKind::Newline(_) => continue,
            TokenKind::Word(meta) => {
                let chars = &source[token.span.start..token.span.end];
                Kind::Word(Word {
                    text: chars.iter().map(|&c| if c == '’' { '\'' } else { c }).collect::<String>().to_lowercase(),
                    tag: meta.as_ref().map_or(Tag::Unknown, tag_of),
                    meta: meta.as_ref(),
                    capital: chars.first().is_some_and(|c| c.is_uppercase()),
                })
            }
            TokenKind::Punctuation(Punctuation::Comma) => Kind::Comma,
            TokenKind::Punctuation(Punctuation::Period)
                if items.last().is_some_and(|item: &Item| matches!(&item.kind, Kind::Word(w) if TITLES.contains(&w.text.as_str()))) =>
            {
                continue;
            }
            TokenKind::Punctuation(Punctuation::Quote(_)) => {
                quoted = !quoted;
                items.push(Item { kind: Kind::Edge, span: token.span, quoted: true });
                continue;
            }
            TokenKind::Punctuation(
                Punctuation::Period
                | Punctuation::Bang
                | Punctuation::Question
                | Punctuation::Semicolon
                | Punctuation::Colon
                | Punctuation::EmDash
                | Punctuation::EnDash
                | Punctuation::Ellipsis
                | Punctuation::OpenRound
                | Punctuation::CloseRound
                | Punctuation::OpenSquare
                | Punctuation::CloseSquare,
            )
            | TokenKind::ParagraphBreak
            | TokenKind::HeadingStart => Kind::Edge,
            _ => Kind::Other,
        };
        items.push(Item { kind, span: token.span, quoted });
    }
    items
}

fn tag_of(meta: &DictWordMetadata) -> Tag {
    let Some(tag) = meta.pos_tag else { return Tag::Unknown };
    match format!("{tag:?}").as_str() {
        "NOUN" => Tag::Noun,
        "PROPN" => Tag::Propn,
        "VERB" => Tag::Verb,
        "AUX" => Tag::Aux,
        "ADJ" => Tag::Adj,
        "ADV" => Tag::Adv,
        "DET" => Tag::Det,
        "PRON" => Tag::Pron,
        "CCONJ" => Tag::Cconj,
        "SCONJ" => Tag::Sconj,
        "ADP" => Tag::Adp,
        "NUM" => Tag::Num,
        _ => Tag::Other,
    }
}

fn word<'i, 'a>(items: &'i [Item<'a>], i: usize) -> Option<&'i Word<'a>> {
    match items.get(i).map(|item| &item.kind) {
        Some(Kind::Word(w)) => Some(w),
        _ => None,
    }
}

// ---- Clauses ----

const PERSONAL: [&str; 7] = ["i", "you", "he", "she", "it", "we", "they"];

/// A personal pronoun with its verb in one word: a subject and a finite verb already.
const WITH_VERB: [&str; 26] = [
    "i'm", "i've", "i'll", "i'd", "you're", "you've", "you'll", "you'd", "he's", "he'd", "he'll", "she's", "she'd",
    "she'll", "it's", "it'd", "it'll", "we're", "we've", "we'll", "we'd", "they're", "they've", "they'll", "they'd",
    "i'd've",
];

/// Words that always open a clause that can't stand alone.
const SUBORDINATING: [&str; 13] =
    ["when", "whenever", "while", "whilst", "because", "although", "though", "if", "unless", "whereas", "whether", "wherever", "where"];

/// Words that open such a clause only when a clause follows ("After she left" but not "After dinner").
const SUBORDINATING_BEFORE_A_CLAUSE: [&str; 7] = ["after", "before", "since", "until", "till", "as", "once"];

/// Words before a clause that make it part of another: "the man who left", "the door that…".
const RELATIVE: [&str; 11] = ["who", "whom", "whose", "which", "that", "what", "whatever", "whoever", "whichever", "how", "why"];

/// Auxiliaries that open a condition when they come before their subject: "Had she known, …", "were it not for".
const INVERTED: [&str; 4] = ["had", "were", "should", "could"];

/// Verbs that set off what they report: "he said", "I believe", between commas.
const REPORTING: [&str; 42] = [
    "say", "says", "said", "think", "thinks", "thought", "believe", "believes", "believed", "suppose", "supposed",
    "observed", "fear", "feared", "know", "knew", "hope", "hoped", "imagine", "imagined", "guess", "mean", "meant",
    "trust", "suspect", "suspected", "confess", "own", "admit", "admitted", "added", "replied", "continued", "cried",
    "answered", "assure", "assured", "dare", "presume", "understand", "doubt", "doubted",
];

/// Verbs of thinking, whose clause can come after an interruption: "She supposed, if he meant it, he…".
const THINKING: [&str; 22] = [
    "think", "thought", "believe", "believed", "suppose", "supposed", "fear", "feared", "know", "knew", "hope",
    "hoped", "imagine", "imagined", "guess", "trust", "suspect", "suspected", "doubt", "doubted", "felt", "understood",
];

/// Titles whose full stop doesn't end a sentence ("Mrs. Philips").
const TITLES: [&str; 16] = ["mr", "mrs", "ms", "messrs", "dr", "st", "prof", "rev", "capt", "col", "gen", "lt", "sgt", "hon", "jr", "sr"];

/// The subject's kind, which decides how sure the verb after it must be.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Subject {
    Pronoun,
    Other,
}

/// If a subject and a finite verb start at `i`, where the clause's verb is.
fn clause_at(items: &[Item], i: usize) -> Option<usize> {
    let first = word(items, i)?;
    if WITH_VERB.contains(&first.text.as_str()) {
        return Some(i);
    }
    let (mut subject, mut v) = subject_at(items, i)?;
    // "he and the females", "Jane or her sister"
    while word(items, v).is_some_and(|w| w.text == "and" || w.text == "or") {
        let (_, after) = subject_at(items, v + 1)?;
        subject = Subject::Other; // what agrees with "he and she" is plural: no help from the pronoun
        v = after;
    }
    // "she slowly turned", "he never came"
    for _ in 0..2 {
        if word(items, v).is_some_and(|w| w.tag == Tag::Adv) {
            v += 1;
        }
    }
    finite(items, v, subject).then_some(v)
}

/// If a subject starts at `i`, its kind and where the word after it is.
fn subject_at(items: &[Item], i: usize) -> Option<(Subject, usize)> {
    let first = word(items, i)?;
    if PERSONAL.contains(&first.text.as_str()) && first.tag != Tag::Det {
        Some((Subject::Pronoun, i + 1))
    } else {
        Some((Subject::Other, noun_phrase(items, i)?))
    }
}

/// If a noun phrase starts at `i` ("the old seal", "his hands", "Mara"), where the word after it is.
fn noun_phrase(items: &[Item], i: usize) -> Option<usize> {
    let mut j = i;
    let first = word(items, j)?;
    let determiner = first.tag == Tag::Det || (first.tag == Tag::Pron && first.meta.and_then(|m| m.pronoun).and_then(|p| p.is_possessive) == Some(true))
        || ["his", "her", "its", "their", "my", "your", "our"].contains(&first.text.as_str());
    if determiner {
        // "that Mr. Darcy…": "that" opening a clause, not a determiner, since names take none.
        if first.text == "that" && word(items, j + 1).is_some_and(|n| n.capital || n.tag == Tag::Pron) {
            return None;
        }
        j += 1;
    }
    let mut last_noun = false;
    while let Some(w) = word(items, j) {
        if w.text.ends_with('.') {
            break; // "etc."
        }
        // A word with a capital that the tagger said nothing about ("Mara"), or past a sentence's
        // first word, taken for some other word ("as Miss De Bourgh chose"): a name.
        let mid_sentence = j > 0 && !matches!(items[j - 1].kind, Kind::Edge);
        let name = w.capital
            && w.text != "i"
            && (w.tag == Tag::Unknown || (mid_sentence && matches!(w.tag, Tag::Verb | Tag::Noun | Tag::Adj | Tag::Propn)));
        match w.tag {
            Tag::Noun | Tag::Propn => last_noun = true,
            Tag::Adj | Tag::Num => last_noun = false,
            _ if name => last_noun = true,
            _ => break,
        }
        j += 1;
    }
    (last_noun && j > i).then_some(j)
}

/// What a verb's form says about whether it's finite.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Form {
    /// Finite wherever it stands: an auxiliary ("was", "could", "didn't"), a past tense unlike its
    /// participle ("broke"), a present "-s" form ("cracks").
    Finite,
    /// A past that may be a participle ("clenched", "said"), or a base form that may be one ("run").
    MaybeParticiple,
    /// A base form that can't be a participle ("go", "bark").
    Base,
    /// Never finite: "broken", "been", "flapping".
    Never,
    /// Nothing known.
    Unknown,
}

fn form_of(w: &Word, meta: &DictWordMetadata) -> Form {
    let Some(verb) = meta.verb else {
        // Not in Harper's dictionary as a verb, though tagged as one where it stands ("gripped"): by its ending.
        return match () {
            _ if w.tag == Tag::Aux => Form::Finite,
            _ if w.text.ends_with("ing") => Form::Never,
            _ if w.text.ends_with("ed") => Form::MaybeParticiple,
            _ if w.tag == Tag::Verb && w.text.ends_with('s') => Form::Finite,
            _ => Form::Unknown,
        };
    };
    if meta.is_verb_progressive_form() && !meta.is_verb_lemma() {
        return Form::Never;
    }
    if meta.is_verb_past_participle_only() {
        return Form::Never;
    }
    if w.tag == Tag::Aux || verb.is_auxiliary == Some(true) || meta.is_verb_simple_past_only() || meta.is_verb_third_person_singular_present_form() {
        return Form::Finite;
    }
    if meta.is_verb_past_form() || meta.is_verb_simple_past_form() || meta.is_verb_past_participle_form() {
        return Form::MaybeParticiple;
    }
    if meta.is_verb_lemma() {
        return Form::Base;
    }
    Form::Unknown
}

/// Is the word at `v` a finite verb after a subject of this kind?
fn finite(items: &[Item], v: usize, subject: Subject) -> bool {
    let Some(w) = word(items, v) else { return false };
    if w.text.ends_with("n't") {
        return true; // "didn't", "wasn't": not always in Harper's dictionary
    }
    if !matches!(w.tag, Tag::Verb | Tag::Aux | Tag::Unknown) {
        return false;
    }
    let Some(meta) = w.meta else { return false };
    match form_of(w, meta) {
        Form::Finite | Form::Base => true,
        Form::Never => false,
        // After a pronoun, or with its object after it ("his hands clenched the rail"); never in an absolute phrase.
        Form::MaybeParticiple => {
            subject == Subject::Pronoun || word(items, v + 1).is_some_and(|o| matches!(o.tag, Tag::Det | Tag::Pron | Tag::Noun | Tag::Propn | Tag::Num))
        }
        Form::Unknown => subject == Subject::Pronoun,
    }
}

/// Does what's before the comma at `c` end with a clause that could stand alone?
///
/// Read from the last sentence end (or semicolon, colon, dash, quote): each subordinating or
/// relative word claims the next clause as its own ("and though his pride was hurt", "that, as
/// she hoped, it would"), as does a reporting verb before a comma ("She supposed, if…"), and a
/// clause joined by "and" to a claimed one is claimed too ("When they had come, and tea was
/// over"). Only a clause nobody claimed stands alone. One must, with no claim still waiting for
/// its clause ("He protested that, except his sister,"), and no joining word waiting for one
/// either ("and, to crown it all,", "and though it rained,").
fn clause_before(items: &[Item], c: usize) -> bool {
    let start = items[..c].iter().rposition(|item| matches!(item.kind, Kind::Edge)).map_or(0, |e| e + 1);
    let (mut waiting, mut stands, mut joined) = (0usize, false, false);
    let (mut opened, mut last_claimed) = (false, false);
    let mut i = start;
    while i < c {
        let joining = word(items, i).is_some_and(|w| w.tag == Tag::Cconj);
        if opens(items, i, start) {
            waiting += 1;
            opened = true;
            i += 1;
            continue;
        }
        if joining && (matches!(items.get(i + 1).map(|x| &x.kind), Some(Kind::Comma)) || opens(items, i + 1, start)) {
            joined = true;
        } else if let Some(v) = clause_at(items, i).filter(|&v| v < c && (opened || !inside_noun_phrase(items, i, start))) {
            let after_and = i > start && word(items, i - 1).is_some_and(|w| w.tag == Tag::Cconj);
            if waiting > 0 {
                waiting -= 1;
                last_claimed = true;
            } else if !(after_and && last_claimed) {
                stands = true;
                joined = false;
                last_claimed = false;
            }
            // "She supposed, if he meant anything, he must…": what she supposed is still to come.
            // Not "she said,", which after dialogue is a Dialogue tag ending its clause.
            if word(items, v).is_some_and(|w| THINKING.contains(&w.text.as_str())) && matches!(items.get(v + 1).map(|x| &x.kind), Some(Kind::Comma)) {
                waiting += 1;
            }
            opened = false;
            i = v + 1;
            continue;
        }
        opened = false;
        i += 1;
    }
    stands && waiting == 0 && !joined
}

/// Does the word at `i` open a clause that can't stand alone, which is the next one along?
fn opens(items: &[Item], i: usize, start: usize) -> bool {
    let Some(w) = word(items, i) else { return false };
    let text = w.text.as_str();
    let clause_starts_here = i == start || matches!(items[i - 1].kind, Kind::Comma) || word(items, i - 1).is_some_and(|p| p.tag == Tag::Cconj);
    if INVERTED.contains(&text) && clause_starts_here && subject_at(items, i + 1).is_some() {
        return true; // "Had she known", "could Wickham be prevailed on"
    }
    let next_is_comma = matches!(items.get(i + 1).map(|x| &x.kind), Some(Kind::Comma));
    if SUBORDINATING.contains(&text) {
        return true;
    }
    if text == "however" {
        // "However little known his views may be": not "However, she left".
        return word(items, i + 1).is_some_and(|n| matches!(n.tag, Tag::Adj | Tag::Adv));
    }
    if SUBORDINATING_BEFORE_A_CLAUSE.contains(&text) {
        return next_is_comma || clause_at(items, i + 1).is_some();
    }
    if RELATIVE.contains(&text) || w.tag == Tag::Sconj {
        if w.tag == Tag::Det {
            return false; // "that day"
        }
        // "who left", "the door that was locked": the word is its clause's subject, and the clause is done.
        let mut v = i + 1;
        while word(items, v).is_some_and(|n| n.tag == Tag::Adv) {
            v += 1;
        }
        return !finite(items, v, Subject::Pronoun);
    }
    false
}

/// Is the word at `i` inside a noun phrase begun before it ("the old man", "the praises he asked
/// for")? Then no clause starts there.
///
/// Nor right after a pronoun ("everything he might say") or a preposition ("his engagement with
/// Jane had been formed"), where the clause began earlier.
fn inside_noun_phrase(items: &[Item], i: usize, start: usize) -> bool {
    i > start
        && (word(items, i - 1).is_some_and(|w| matches!(w.tag, Tag::Det | Tag::Adj | Tag::Num | Tag::Noun | Tag::Propn | Tag::Pron | Tag::Adp) || (w.tag == Tag::Unknown && w.capital))
            // "the praises he asked for", "praises" taken for a verb
            || (i > start + 1 && word(items, i - 2).is_some_and(|w| w.tag == Tag::Det)))
}

/// Is the clause after a comma, whose verb is at `v`, a reporting one set off by commas ("He was
/// then, he said, on his way", "It has, I believe, been urged")?
fn parenthetical(items: &[Item], v: usize) -> bool {
    let mut end = v + 1;
    if word(items, end).is_some_and(|w| matches!(w.tag, Tag::Verb | Tag::Aux | Tag::Adv | Tag::Other)) {
        end += 1; // "she could tell", "I dare say", "she doubted not"
    }
    matches!(items.get(end).map(|x| &x.kind), Some(Kind::Comma)) && (v..end).any(|i| word(items, i).is_some_and(|w| REPORTING.contains(&w.text.as_str())))
}

/// Is the comma at `c` one of a series closed by a joining word before the sentence ends ("I came, I saw, and I conquered")?
fn in_closed_series(items: &[Item], c: usize) -> bool {
    let end = items[c + 1..].iter().position(|item| matches!(item.kind, Kind::Edge)).map_or(items.len(), |e| c + 1 + e);
    (c + 1..end).any(|i| matches!(items[i].kind, Kind::Comma) && word(items, i + 1).is_some_and(|w| w.tag == Tag::Cconj))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harper_core::parsers::PlainEnglish;
    use harper_core::spell::FstDictionary;

    /// What's marked in `text`, as the text the Marks cover.
    fn marked(text: &str) -> Vec<String> {
        let document = Document::new(text, &PlainEnglish, &FstDictionary::curated());
        let source = document.get_source();
        CommaSplice.lint(&document).iter().map(|l| source[l.span.start..l.span.end].iter().collect()).collect()
    }

    fn marks(text: &str) {
        assert!(!marked(text).is_empty(), "should be marked: {text}");
    }

    fn passes(text: &str) {
        assert_eq!(marked(text), Vec::<String>::new(), "should pass: {text}");
    }

    #[test]
    fn two_sentences_joined_by_a_comma_are_marked_on_the_comma_and_the_word_after() {
        assert_eq!(marked("She reached the door, it was locked."), [", it"]);
        assert_eq!(marked("She nodded, he smiled."), [", he"]);
        assert_eq!(marked("Mara opened the letter, the seal was broken."), [", the"]);
        assert_eq!(marked("I came, I saw, I conquered."), [", I", ", I"]);
        assert_eq!(marked("He was tired, she could tell."), [", she"]);
    }

    #[test]
    fn a_pronoun_with_its_verb_in_one_word_is_a_clause() {
        marks("It was late, it's dark now.");
        marks("She shrugged, I’m going anyway.");
        marks("He laughed, they'd heard it before.");
    }

    #[test]
    fn a_first_part_that_cannot_stand_alone_is_fine() {
        passes("When she reached the door, it was locked.");
        passes("As she walked, it rained.");
        passes("If you go, I go.");
        passes("Because it rained, we stayed.");
        passes("After she left, he cried.");
        // But "after" before a noun isn't subordinate: the clause after it still stands alone.
        marks("After dinner, she left, he stayed.");
        passes("In the morning, she left.");
        passes("Yes, I know.");
    }

    #[test]
    fn a_joining_or_relative_word_after_the_comma_is_fine() {
        passes("She reached the door, and it was locked.");
        passes("She reached the door, but it was locked.");
        passes("She reached the door, which was locked.");
        passes("She reached the door, panting.");
    }

    #[test]
    fn absolute_phrases_are_never_marked() {
        passes("He turned, his hands clenched into fists.");
        passes("He turned, his coat flapping behind him.");
        passes("She sat down, her face pale.");
        passes("Mara opened the letter, the seal broken.");
    }

    #[test]
    fn a_noun_phrase_needs_a_verb_that_can_only_be_finite() {
        marks("He turned, his hands gripped the rail.");
        marks("The dog barks, the cat runs.");
        marks("The ice broke, the river ran free.");
        // Missed on purpose: "rattled" might be a participle, as in an absolute phrase.
        passes("The wind rose, the shutters rattled.");
        // So is a noun phrase before the comma with such a verb: it may be an absolute phrase too ("The door locked, she went to bed").
        passes("The ice cracked, the river broke free.");
        passes("His work done, he left.");
    }

    #[test]
    fn dialogue_is_left_alone() {
        passes("\"I'm cold,\" she said.");
        passes("“I'm cold, I want to go home,” she said.");
        passes("She said, \"It's late.\"");
        passes("\"I'm cold,\" she said, \"let's go.\"");
        // A splice in the narration after a Dialogue tag is still one.
        marks("\"Go,\" she said, he left.");
    }

    #[test]
    fn clauses_in_a_series_closed_by_a_joining_word_are_fine() {
        passes("I came, I saw, and I conquered.");
        passes("She cooked, he cleaned, and they ate.");
    }

    #[test]
    fn a_subordinating_word_anywhere_claims_the_clause_after_it() {
        passes("He thought well of himself; and though his pride was hurt, he suffered in no other way.");
        passes("And, as he spoke, he left the room.");
        passes("He protested that, except his sister, he had never seen a more elegant woman.");
        passes("When, after a long silence, she turned to the window, she saw him.");
        passes("However little known his views may be, this truth is well fixed.");
        passes("He was tired of the books, which, as he said himself, he had never read.");
        // A clause after the claimed one stands alone again.
        marks("She smiled when he left, it was over.");
        marks("When the game was over, the tables were cleared, the carriage was called.");
    }

    #[test]
    fn more_that_claims_a_clause() {
        passes("Had she found Jane in any danger, Mrs. Bennet would have been miserable.");
        passes("He had never supposed that, could Wickham be persuaded, it would be done so cheaply.");
        passes("Her best comfort was, that Mr. Bingley must be down again in the summer.");
        passes("She knew how materially the credit of both must be hurt, she felt depressed.");
        passes("She supposed, if he meant anything, he must mean an allusion to her.");
        passes("He was as generous, she doubted not, as the most generous of his sex.");
        passes("She could not be insensible to it, and though her intentions did not vary, she was sorry.");
        passes("When the gentlemen had joined them, and tea was over, the card tables were placed.");
        passes("One morning, as he and the girls were sitting together, their attention was drawn to the window.");
        passes("A week after his engagement with Jane had been formed, they went to town.");
        passes("With a strong prejudice against everything he might say, she began.");
        passes("Collins sat down; and as Miss De Bourgh chose to play cards, the two girls helped.");
    }

    #[test]
    fn a_title_does_not_end_the_sentence() {
        passes("As Mrs. Philips began to speak, he assured her it was nothing.");
        marks("Mrs. Philips began to speak, he assured her it was nothing.");
    }

    #[test]
    fn a_reporting_clause_set_off_by_commas_is_fine() {
        passes("He was then, he said, on his way to town.");
        passes("It has sometimes, I believe, been urged that he was proud.");
        // At the sentence's end, it's still a splice.
        marks("He was tired, she could tell.");
    }

    #[test]
    fn a_joining_word_before_an_interruption_joins() {
        passes("She was young and clever, and, to crown it all, she was rich.");
    }

    #[test]
    fn a_clause_inside_a_noun_phrase_does_not_stand_alone() {
        passes("Here, scarcely allowing them the praises he asked for, every view was pointed out.");
    }

    #[test]
    fn imperatives_before_the_comma_are_missed() {
        passes("Come here, I need you.");
    }

    #[test]
    fn the_suggestions_are_a_full_stop_and_a_semicolon() {
        let text = "She reached the door, it was locked.";
        let document = Document::new(text, &PlainEnglish, &FstDictionary::curated());
        let lints = CommaSplice.lint(&document);
        let fixes: Vec<String> = lints[0].suggestions.iter().map(|s| match s {
            Suggestion::ReplaceWith(c) => c.iter().collect(),
            other => panic!("{other:?}"),
        }).collect();
        assert_eq!(fixes, [". It", "; it"]);
        assert_eq!(lints[0].lint_kind, LintKind::Punctuation);
        // A name keeps its capital.
        let document = Document::new("She laughed, Mara didn't.", &PlainEnglish, &FstDictionary::curated());
        let lints = CommaSplice.lint(&document);
        assert!(matches!(&lints[0].suggestions[0], Suggestion::ReplaceWith(c) if c.iter().collect::<String>() == ". Mara"), "{lints:?}");
    }
}

/// The rule over a whole text, to see what it marks in real prose: every Mark with its sentence,
/// then how many. A plain text file, paragraphs between blank lines:
/// `WRTIER_SPLICE_TEXT=chapter.txt cargo test --lib comma_splice::over_a_text -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore]
fn over_a_text() {
    use harper_core::parsers::PlainEnglish;
    use harper_core::spell::FstDictionary;
    let path = std::env::var("WRTIER_SPLICE_TEXT").expect("WRTIER_SPLICE_TEXT: a text file");
    let text = std::fs::read_to_string(path).expect("readable");
    let dictionary = FstDictionary::curated();
    let (mut marks, mut words) = (0, 0);
    for paragraph in text.replace("\r\n", "\n").split("\n\n") {
        let paragraph = paragraph.split_whitespace().collect::<Vec<_>>().join(" ");
        words += paragraph.split(' ').filter(|w| !w.is_empty()).count();
        let document = Document::new(&paragraph, &PlainEnglish, &dictionary);
        let source = document.get_source();
        for lint in CommaSplice.lint(&document) {
            marks += 1;
            let from = source[..lint.span.start].iter().rposition(|c| ".!?\"“”".contains(*c)).map_or(0, |i| i + 1);
            let to = source[lint.span.end..].iter().position(|c| ".!?\"“”".contains(*c)).map_or(source.len(), |i| lint.span.end + i + 1);
            let before: String = source[from..lint.span.start].iter().collect();
            let marked: String = source[lint.span.start..lint.span.end].iter().collect();
            let after: String = source[lint.span.end..to].iter().collect();
            println!("{}[{}]{}", before.trim_start(), marked, after);
        }
    }
    println!("\n{marks} Marks in {words} words");
}
