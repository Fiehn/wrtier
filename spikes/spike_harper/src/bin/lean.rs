//! Spike: Harper without its FST copy. SpellCheck stays on: it is what uses the FST.
//! `cargo run --release --bin lean -- <novel.txt> <fst|mutable>`

use harper_core::linting::LintGroup;
use harper_core::parsers::PlainEnglish;
use harper_core::spell::{Dictionary, FstDictionary, MutableDictionary};
use harper_core::{Dialect, Document};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

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
#[global_allocator]
static GLOBAL: Counting = Counting;
fn mb(b: usize) -> String {
    format!("{:.1} MB", b as f64 / 1_048_576.0)
}

fn run<D: Dictionary + 'static>(dict: Arc<D>, paragraphs: &[String], base: usize) {
    if std::env::var("PARSE_ONLY").is_ok() {
        let h = CURRENT.load(Relaxed);
        let _ = Document::new("Warm up.", &PlainEnglish, &*dict);
        let h1 = CURRENT.load(Relaxed);
        for p in paragraphs { let _ = Document::new(p, &PlainEnglish, &*dict); }
        println!("parse only (tagger + chunker, no lints): load +{}, after all paragraphs +{} more", mb(h1 - h), mb(CURRENT.load(Relaxed) - h1));
        return;
    }
    let mut group = LintGroup::new_curated(dict.clone(), Dialect::British);
    if std::env::var("NO_SPELL").is_ok() { group.config.set_rule_enabled("SpellCheck", false); }
    println!("after dictionary + LintGroup: heap +{}", mb(CURRENT.load(Relaxed) - base));
    let t = Instant::now();
    let mut by_rule: BTreeMap<String, usize> = BTreeMap::new();
    for (i, p) in paragraphs.iter().enumerate() {
        // A fresh group every 200 paragraphs keeps its LRU cache from building up.
        let d = Document::new(p, &PlainEnglish, &*dict);
        for (r, l) in group.organized_lints(&d) {
            *by_rule.entry(r).or_default() += l.len();
        }
        if [0, 9, 99, 499, 999, 1999].contains(&i) { println!("  after paragraph {}: heap +{}", i + 1, mb(CURRENT.load(Relaxed) - base)); }
    }
    let total: usize = by_rule.values().sum();
    println!("whole novel, a paragraph at a time: {:.1} s, {total} lints", t.elapsed().as_secs_f64());
    println!("SplitWords: {}", by_rule.get("SplitWords").copied().unwrap_or(0));
    println!("heap at the end +{}, peak +{}", mb(CURRENT.load(Relaxed) - base), mb(PEAK.load(Relaxed) - base));
    let sig: Vec<String> = by_rule.iter().filter(|(_, n)| **n > 0).map(|(r, n)| format!("{r}={n}")).collect();
    std::fs::write(format!("lean-{}.txt", std::env::args().nth(2).unwrap()), sig.join("\n")).unwrap();
}

fn main() {
    let path = std::env::args().nth(1).expect("novel");
    let which = std::env::args().nth(2).expect("fst|mutable");
    let raw = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    let start = raw.find("*** START OF").and_then(|i| raw[i..].find('\n').map(|j| i + j + 1)).unwrap_or(0);
    let end = raw.find("*** END OF").unwrap_or(raw.len());
    let paragraphs: Vec<String> = raw[start..end]
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ").replace('_', ""))
        .filter(|p| !p.is_empty())
        .collect();
    let base = CURRENT.load(Relaxed);
    PEAK.store(base, Relaxed);
    let t = Instant::now();
    if which == "fst" {
        let d = FstDictionary::curated();
        println!("FstDictionary::curated: {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        run(d, &paragraphs, base);
    } else {
        let d = MutableDictionary::curated();
        println!("MutableDictionary::curated: {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        run(d, &paragraphs, base);
    }
}
