//! Spike: how Harper (harper-core) behaves for Wrtier. Throwaway code.
//!
//! Run: `cargo run --release -- <path to a Gutenberg .txt novel>`
//! Checks: British English, extra words at runtime, word types, character offsets, grammar on
//! typical mistakes and on clean fiction, and speed and memory on a whole novel.

use harper_core::linting::{Lint, LintGroup, Suggestion};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::{Dictionary, FstDictionary, MergedDictionary, MutableDictionary};
use harper_core::{Dialect, DictWordMetadata, Document, TokenKind};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

// ---- Counting every byte allocated, for memory figures ----

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(now, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        CURRENT.fetch_sub(layout.size(), Relaxed);
    }
}

// Off with `--no-default-features`, to time without the counting's own cost.
#[cfg(feature = "count-memory")]
#[global_allocator]
static GLOBAL: Counting = Counting;

fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}
fn heap() -> usize {
    CURRENT.load(Relaxed)
}
fn reset_peak() {
    PEAK.store(CURRENT.load(Relaxed), Relaxed);
}
fn peak() -> usize {
    PEAK.load(Relaxed)
}

fn ms(t: Instant) -> String {
    format!("{:.1} ms", t.elapsed().as_secs_f64() * 1000.0)
}

// ---- Showing lints ----

fn suggestion(s: &Suggestion) -> String {
    match s {
        Suggestion::ReplaceWith(c) => format!("→ “{}”", c.iter().collect::<String>()),
        Suggestion::InsertAfter(c) => format!("+ “{}”", c.iter().collect::<String>()),
        Suggestion::Remove => "remove".into(),
    }
}

fn show(lints: &BTreeMap<String, Vec<Lint>>, doc: &Document) -> Vec<String> {
    let mut out = vec![];
    for (rule, ls) in lints {
        for l in ls {
            let sugg: Vec<_> = l.suggestions.iter().take(4).map(suggestion).collect();
            out.push(format!(
                "[{rule}] “{}” ({}..{}): {}  {}",
                l.get_str(doc.get_source()),
                l.span.start,
                l.span.end,
                l.message,
                sugg.join(", ")
            ));
        }
    }
    out
}

fn lint_text(group: &mut LintGroup, dict: &impl Dictionary, text: &str) -> Vec<String> {
    let doc = Document::new(text, &PlainEnglish, dict);
    show(&group.organized_lints(&doc), &doc)
}

fn print_case(group: &mut LintGroup, dict: &impl Dictionary, text: &str) {
    println!("  > {text}");
    let lines = lint_text(group, dict, text);
    if lines.is_empty() {
        println!("      (nothing)");
    }
    for l in lines {
        println!("      {l}");
    }
}

fn main() {
    let novel_path = std::env::args().nth(1);
    let base = heap();

    // ---- 1. Start-up cost ----
    println!("\n## 1. Start-up\n");
    reset_peak();
    let t = Instant::now();
    let curated = FstDictionary::curated();
    println!("curated dictionary: {} , heap +{}", ms(t), mb(heap() - base));
    let h1 = heap();
    let t = Instant::now();
    let mut british = LintGroup::new_curated(curated.clone(), Dialect::British);
    println!("LintGroup (British): {} , heap +{}", ms(t), mb(heap() - h1));
    println!("rules in the curated group: {}", british.iter_keys().count());
    let enabled = british.iter_keys().filter(|k| british.config.is_rule_enabled(k)).count();
    println!("rules enabled by default: {enabled}");
    let h2 = heap();
    let t = Instant::now();
    let first = Document::new("The first sentence loads the tagger and chunker.", &PlainEnglish, &*curated);
    println!("first Document (loads tagger + chunker): {} , heap +{}", ms(t), mb(heap() - h2));
    let t = Instant::now();
    let _ = british.organized_lints(&first);
    println!("first lint: {}", ms(t));
    println!("heap after start-up: {} (peak {})", mb(heap() - base), mb(peak() - base));
    println!("Dialect::try_from_bcp47(\"en-GB\") = {:?}", Dialect::try_from_bcp47("en-GB"));
    println!("Dialect::try_from_bcp47(\"en-US\") = {:?}", Dialect::try_from_bcp47("en-US"));

    let mut american = LintGroup::new_curated(curated.clone(), Dialect::American);

    // ---- 2. British vs American ----
    println!("\n## 2. British vs American\n");
    let dialect_cases = [
        "The colour of the organisation's flag was grey.",
        "The color of the organization's flag was gray.",
        "She will realise it soon, and he realized it yesterday.",
        "I need a licence to drive, so I must license the car.",
        "I need a license to drive.",
        "You should practise the piano; practice makes perfect.",
        "He was travelling to the theatre with his neighbour.",
        "He was traveling to the theater with his neighbor.",
        "We analysed the programme and the catalogue.",
        "I'll ring you from the lift in the flat.",
        "She put the tyres in the boot of the car.",
    ];
    for case in dialect_cases {
        println!("  > {case}");
        for (name, g) in [("GB", &mut british), ("US", &mut american)] {
            let lines = lint_text(g, &*curated, case);
            println!("      {name}: {}", if lines.is_empty() { "(nothing)".into() } else { lines.join("\n          ") });
        }
    }

    // ---- 3. Extra words at runtime ----
    println!("\n## 3. Added and Allowed words\n");
    let names = "Kvothe walked to Imre with Denna. Kvothe's lute was old. KVOTHE shouted.";
    println!("without extra words:");
    print_case(&mut british, &*curated, names);
    let t = Instant::now();
    let mut own = MutableDictionary::new();
    for w in ["Kvothe", "Imre", "Denna"] {
        own.append_word_str(w, DictWordMetadata::default());
    }
    let mut merged = MergedDictionary::new();
    merged.add_dictionary(curated.clone());
    merged.add_dictionary(Arc::new(own));
    let merged = Arc::new(merged);
    let built = ms(t);
    let t = Instant::now();
    let mut with_words = LintGroup::new_curated(merged.clone(), Dialect::British);
    println!("merged dictionary built in {built}, new LintGroup in {}", ms(t));
    println!("with Kvothe, Imre, Denna added (default metadata):");
    print_case(&mut with_words, &*merged, names);
    println!("lower-case use of an added name:");
    print_case(&mut with_words, &*merged, "the kvothe was here.");
    // A thousand Added words, as a long-used Vault might have.
    let t = Instant::now();
    let mut many = MutableDictionary::new();
    for i in 0..1000 {
        many.append_word_str(&format!("Zorblax{i}"), DictWordMetadata::default());
    }
    let mut merged2 = MergedDictionary::new();
    merged2.add_dictionary(curated.clone());
    merged2.add_dictionary(Arc::new(many));
    let merged2 = Arc::new(merged2);
    let _g = LintGroup::new_curated(merged2.clone(), Dialect::British);
    println!("1,000 added words: dictionary + LintGroup rebuilt in {}", ms(t));

    // Does the dictionary a Document is parsed with matter, separate from the group's?
    println!("group has the names, Document parsed with the curated dictionary only:");
    print_case(&mut with_words, &*curated, names);

    // Spelling suggestions, and how long one costs.
    println!("spelling suggestions (British):");
    for w in ["recieve", "teh", "colur", "realy", "definately", "thier"] {
        let t = Instant::now();
        let lines = lint_text(&mut british, &*curated, &format!("I {w} it."));
        println!("      {w}: {} ({})", lines.join(" | "), ms(t));
    }

    // ---- 4. Word types ----
    println!("\n## 4. Word types\n");
    let tag_cases = [
        "I will record a record.",
        "They saw the saw on the table.",
        "Time flies like an arrow; fruit flies like a banana.",
        "Kvothe's colour faded as the fast ships sailed fast.",
        "She can can fish in a can.",
        "Running is what the running man does when running out.",
    ];
    for (label, dict) in [("curated", &*curated as &dyn Dictionary), ("with names", &*merged as &dyn Dictionary)] {
        println!("dictionary: {label}");
        for case in tag_cases {
            let doc = Document::new(case, &PlainEnglish, &dict);
            let tags: Vec<String> = doc
                .tokens()
                .filter_map(|t| match &t.kind {
                    TokenKind::Word(Some(m)) => Some(format!(
                        "{}/{}{}",
                        t.get_str(doc.get_source()),
                        m.pos_tag.map(|p| format!("{p:?}")).unwrap_or("?".into()),
                        if m.np_member == Some(true) { "*" } else { "" }
                    )),
                    TokenKind::Word(None) => Some(format!("{}/UNKNOWN", t.get_str(doc.get_source()))),
                    _ => None,
                })
                .collect();
            println!("  {}", tags.join(" "));
        }
    }
    println!("  (* = in a noun phrase, from the chunker)");

    // ---- 5. Character offsets ----
    println!("\n## 5. Offsets\n");
    let tricky = "Café — “Its a dog,” she said 😀 and teh cat. Ünïcode teh.";
    let doc = Document::new(tricky, &PlainEnglish, &*curated);
    let lints = british.organized_lints(&doc);
    let utf16_at = |char_index: usize| tricky.chars().take(char_index).map(char::len_utf16).sum::<usize>();
    for (rule, ls) in &lints {
        for l in ls {
            let (s16, e16) = (utf16_at(l.span.start), utf16_at(l.span.end));
            let utf16: Vec<u16> = tricky.encode_utf16().collect();
            let by_utf16 = String::from_utf16_lossy(&utf16[s16..e16]);
            println!(
                "  [{rule}] chars {}..{} = “{}”; UTF-16 {s16}..{e16} = “{by_utf16}”; raw char span read as UTF-16 = “{}”",
                l.span.start,
                l.span.end,
                l.get_str(doc.get_source()),
                String::from_utf16_lossy(&utf16[l.span.start.min(utf16.len())..l.span.end.min(utf16.len())])
            );
        }
    }

    // ---- 6. Grammar ----
    println!("\n## 6. Grammar: typical mistakes (British)\n");
    let mistakes = [
        "She bought a apple and an banana.",
        "Their going to the market over there.",
        "I could of done it better.",
        "The dog wagged it's tail.",
        "Its going to rain.",
        "He is taller then his brother.",
        "I don't want to loose the game.",
        "The weather will effect our plans.",
        "Your going to love this.",
        "She walked to the the door.",
        "He go to school every day.",
        "The children was playing outside.",
        "Me and him went to the shop.",
        "I seen him yesterday.",
        "There is many reasons for this.",
        "She don't like it.",
        "He should have went home.",
        "Who's book is this?",
        "I am going to the shop , and then home.",
        "This is a sentence.This is another.",
        "irregardless of the outcome, we tried.",
        "We need to discuss about the plan.",
        "Between you and I, it's fine.",
        "Less people came than expected.",
        "He did it on accident.",
        "Alot of people agree.",
        "I should of known.",
        "She lead the team last year.",
        "The principle of the school spoke.",
    ];
    for m in mistakes {
        print_case(&mut british, &*curated, m);
    }

    println!("\n## 7. Grammar: clean fiction that should pass (British)\n");
    let clean = [
        "“Go!” she said. “Now.”",
        "He paused — then laughed.",
        "Not now. Not ever.",
        "‘I shan’t,’ said Mrs. Holm, ‘and neither will you.’",
        "The rain fell. And fell. And fell.",
        "Whatever. Fine.",
        "She had had enough of that.",
        "Mr. and Mrs. Dursley were proud to say that they were perfectly normal.",
        "It was the best of times, it was the worst of times.",
        "Yes, sir, I'll fetch the lorry and meet you at half nine.",
        "Ain't nobody got time for that, he muttered.",
        "Hmm. Well, I s'pose so.",
        "The old man the boat.",
        "Buffalo buffalo Buffalo buffalo.",
        "They whispered, “Whither goest thou?”",
        "He ran, he fell, he rose again.",
    ];
    for c in clean {
        print_case(&mut british, &*curated, c);
    }

    // ---- 8. A whole novel ----
    let Some(path) = novel_path else {
        println!("\n(no novel given, skipping speed test)");
        return;
    };
    println!("\n## 8. A whole novel\n");
    let raw = std::fs::read_to_string(&path).expect("read novel");
    let raw = raw.replace("\r\n", "\n");
    let start = raw.find("*** START OF").and_then(|i| raw[i..].find('\n').map(|j| i + j + 1)).unwrap_or(0);
    let end = raw.find("*** END OF").unwrap_or(raw.len());
    // Typst joins a paragraph's lines, as the app's reading would: one line per paragraph.
    let paragraphs: Vec<String> = raw[start..end]
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ").replace('_', ""))
        .filter(|p| !p.is_empty())
        .collect();
    let text = paragraphs.join("\n\n");
    let words = text.split_whitespace().count();
    println!("{} paragraphs, {words} words, {} chars", paragraphs.len(), text.chars().count());

    let h = heap();
    reset_peak();
    let t = Instant::now();
    let doc = Document::new(&text, &PlainEnglish, &*curated);
    let parse = t.elapsed();
    println!("Document::new (tokenise + tag + chunk): {:.0} ms, heap +{} (peak +{})", parse.as_secs_f64() * 1000.0, mb(heap() - h), mb(peak() - h));
    let h = heap();
    reset_peak();
    let t = Instant::now();
    let lints = british.organized_lints(&doc);
    let lint1 = t.elapsed();
    println!("lint whole novel, first time: {:.0} ms, heap +{} (peak +{})", lint1.as_secs_f64() * 1000.0, mb(heap() - h), mb(peak() - h));
    let t = Instant::now();
    let _ = british.organized_lints(&doc);
    println!("lint whole novel again (chunk cache warm): {}", ms(t));
    println!(
        "throughput: parse {:.0}k words/s, first lint {:.0}k words/s",
        words as f64 / parse.as_secs_f64() / 1000.0,
        words as f64 / lint1.as_secs_f64() / 1000.0
    );

    // Per rule, where the time goes: SpellCheck on its own vs everything else.
    let mut spell_only = LintGroup::new_curated(curated.clone(), Dialect::British);
    spell_only.set_all_rules_to(Some(false));
    spell_only.config.set_rule_enabled("SpellCheck", true);
    let t = Instant::now();
    let _ = spell_only.organized_lints(&doc);
    println!("lint whole novel, SpellCheck only: {}", ms(t));
    let mut no_spell = LintGroup::new_curated(curated.clone(), Dialect::British);
    no_spell.config.set_rule_enabled("SpellCheck", false);
    let t = Instant::now();
    let _ = no_spell.organized_lints(&doc);
    println!("lint whole novel, everything but SpellCheck (cold cache): {}", ms(t));

    // What a pause in typing costs: one paragraph re-read and re-linted.
    let mid = &paragraphs[paragraphs.len() / 2];
    let t = Instant::now();
    let pdoc = Document::new(mid, &PlainEnglish, &*curated);
    let _ = british.organized_lints(&pdoc);
    println!("one paragraph ({} words): parse + lint {}", mid.split_whitespace().count(), ms(t));
    // Every paragraph as its own Document (how the app could send them), cold cache.
    let mut fresh = LintGroup::new_curated(curated.clone(), Dialect::British);
    let t = Instant::now();
    let mut per_para = 0;
    for p in &paragraphs {
        let d = Document::new(p, &PlainEnglish, &*curated);
        per_para += fresh.organized_lints(&d).values().map(Vec::len).sum::<usize>();
    }
    println!("every paragraph as its own Document: {} ({per_para} lints)", ms(t));

    // A Document built on another thread (Tauri commands run on a pool).
    let c2 = curated.clone();
    let t = Instant::now();
    std::thread::spawn(move || {
        let d = Document::new("A sentence on a fresh thread.", &PlainEnglish, &*c2);
        drop(d);
    })
    .join()
    .unwrap();
    println!("first Document on a new thread: {}", ms(t));

    // What it found.
    let total: usize = lints.values().map(Vec::len).sum();
    println!("\nlints in the novel: {total} ({:.1} per 1,000 words)", total as f64 * 1000.0 / words as f64);
    let mut by_rule: Vec<_> = lints.iter().map(|(r, l)| (l.len(), r.clone())).filter(|(n, _)| *n > 0).collect();
    by_rule.sort_by(|a, b| b.cmp(a));
    for (n, r) in by_rule.iter().take(30) {
        println!("  {n:5}  {r}");
    }
    println!("\nsamples (up to 4 per rule, top 25 rules):");
    for (_, r) in by_rule.iter().take(25) {
        for l in lints[r].iter().take(4) {
            let src = doc.get_source();
            let (a, b) = (l.span.start.saturating_sub(30), (l.span.end + 30).min(src.len()));
            let ctx: String = src[a..b].iter().collect::<String>().replace('\n', " ");
            let sugg: Vec<_> = l.suggestions.iter().take(3).map(suggestion).collect();
            println!("  [{r}] “{}” in “…{ctx}…” {}", l.get_str(src), sugg.join(", "));
        }
    }
    println!("\nheap at the end: {} above start (peak {})", mb(heap() - base), mb(peak() - base));

    // ---- 9. Which rules cost the time ----
    // The first ~10,000 words, each rule on its own (a changed config means a cold cache).
    println!("\n## 9. Cost per rule (first ~10k words)\n");
    let mut slice = String::new();
    for p in &paragraphs {
        if slice.split_whitespace().count() > 10_000 {
            break;
        }
        slice.push_str(p);
        slice.push_str("\n\n");
    }
    let sdoc = Document::new(&slice, &PlainEnglish, &*curated);
    let mut prof = LintGroup::new_curated(curated.clone(), Dialect::British);
    let t = Instant::now();
    let _ = prof.organized_lints(&sdoc);
    println!("all rules together: {}", ms(t));
    let keys: Vec<String> = prof.iter_keys().map(String::from).collect();
    let mut costs = vec![];
    let t_all = Instant::now();
    for k in &keys {
        prof.set_all_rules_to(Some(false));
        prof.config.set_rule_enabled(k, true);
        let t = Instant::now();
        let _ = prof.organized_lints(&sdoc);
        costs.push((t.elapsed().as_secs_f64() * 1000.0, k.clone()));
    }
    prof.set_all_rules_to(Some(false));
    let t = Instant::now();
    let _ = prof.organized_lints(&sdoc);
    println!("no rules enabled (the group's own overhead): {}", ms(t));
    costs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let sum: f64 = costs.iter().map(|c| c.0).sum();
    println!("sum of rules one at a time: {sum:.0} ms (loop took {})", ms(t_all));
    for (c, k) in costs.iter().take(25) {
        println!("  {c:8.1} ms  {k}");
    }
    let top: f64 = costs.iter().take(25).map(|c| c.0).sum();
    println!("  top 25 = {:.0}% of the sum", top / sum * 100.0);

    // ---- 10. The whole novel across threads, one LintGroup each ----
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    println!("\n## 10. Whole novel on {threads} threads\n");
    let t = Instant::now();
    let per: usize = paragraphs.len().div_ceil(threads);
    let total: usize = std::thread::scope(|s| {
        let handles: Vec<_> = paragraphs
            .chunks(per)
            .map(|chunk| {
                let c = curated.clone();
                s.spawn(move || {
                    let mut g = LintGroup::new_curated(c.clone(), Dialect::British);
                    chunk
                        .iter()
                        .map(|p| g.organized_lints(&Document::new(p, &PlainEnglish, &*c)).values().map(Vec::len).sum::<usize>())
                        .sum::<usize>()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    println!("all paragraphs: {} ({total} lints)", ms(t));

    // ---- 11. Only the rules a novelist would want on by default ----
    // Style rules off (long sentences, dashes, Oxford comma, word choice); grammar left on.
    let mut lean = LintGroup::new_curated(curated.clone(), Dialect::British);
    for k in ["LongSentences", "Dashes", "OxfordComma", "BoringWords", "Excellent", "SentenceCapitalization", "SpellCheck"] {
        lean.config.set_rule_enabled(k, false);
    }
    let t = Instant::now();
    for p in &paragraphs {
        let _ = lean.organized_lints(&Document::new(p, &PlainEnglish, &*curated));
    }
    println!("\nwithout 7 style rules and SpellCheck, one thread: {}", ms(t));
}
