//! Spike: lint one paragraph one rule at a time, and report the rule that allocates most and keeps it.
//! `cargo run --release --bin jumprule -- <novel.txt> <paragraph index>`
use harper_core::linting::LintGroup;
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            CURRENT.fetch_add(layout.size(), Relaxed);
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

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let k: usize = std::env::args().nth(2).unwrap().parse().unwrap();
    let raw = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    let start = raw.find("*** START OF").and_then(|i| raw[i..].find('\n').map(|j| i + j + 1)).unwrap_or(0);
    let end = raw.find("*** END OF").unwrap_or(raw.len());
    let paragraphs: Vec<String> = raw[start..end]
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ").replace('_', ""))
        .filter(|p| !p.is_empty())
        .collect();
    let dict = FstDictionary::curated();
    let mut group = LintGroup::new_curated(dict.clone(), Dialect::British);
    let doc = Document::new(&paragraphs[k], &PlainEnglish, &*dict);
    let keys: Vec<String> = group.iter_keys().map(String::from).collect();
    for key in keys {
        group.set_all_rules_to(Some(false));
        group.config.set_rule_enabled(&key, true);
        let before = CURRENT.load(Relaxed) as isize;
        let _ = group.organized_lints(&doc);
        let d = CURRENT.load(Relaxed) as isize - before;
        if d > 1_048_576 {
            println!("{key}: kept +{:.1} MB", d as f64 / 1_048_576.0);
        }
    }
}
