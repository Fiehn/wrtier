//! Spike: memory and speed as the app would use them, each in a fresh process.
//! `cargo run --release --bin memory -- <novel.txt> <harper|spellbook|both>`

use harper_core::linting::LintGroup;
use harper_core::parsers::PlainEnglish;
use harper_core::spell::FstDictionary;
use harper_core::{Dialect, Document};
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashSet;
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
fn heap() -> usize {
    CURRENT.load(Relaxed)
}
fn peak() -> usize {
    PEAK.load(Relaxed)
}

const AFF: &str = include_str!("../../../../app/src-tauri/dictionaries/en-GB/index.aff");
const DIC: &str = include_str!("../../../../app/src-tauri/dictionaries/en-GB/index.dic");

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("novel path");
    let which = args.next().unwrap_or("both".into());
    let raw = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    let start = raw.find("*** START OF").and_then(|i| raw[i..].find('\n').map(|j| i + j + 1)).unwrap_or(0);
    let end = raw.find("*** END OF").unwrap_or(raw.len());
    let paragraphs: Vec<String> = raw[start..end]
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ").replace('_', ""))
        .filter(|p| !p.is_empty())
        .collect();
    // Distinct words, roughly as the interface collects them for spellbook.
    let words: HashSet<String> = paragraphs
        .iter()
        .flat_map(|p| p.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '’')).map(String::from).collect::<Vec<_>>())
        .filter(|w| w.chars().any(char::is_alphabetic))
        .collect();
    let words: Vec<String> = words.into_iter().collect();
    let base = heap();
    PEAK.store(base, Relaxed);

    if which == "spellbook" || which == "both" {
        let h = heap();
        let t = Instant::now();
        let dict = spellbook::Dictionary::new(AFF, DIC).unwrap();
        println!("spellbook en-GB: load {:.0} ms, heap +{}", t.elapsed().as_secs_f64() * 1000.0, mb(heap() - h));
        let t = Instant::now();
        let unknown = words.iter().filter(|w| !dict.check(w)).count();
        println!("spellbook: {} distinct words checked in {:.1} ms, {unknown} unknown", words.len(), t.elapsed().as_secs_f64() * 1000.0);
        let mut out = vec![];
        let t = Instant::now();
        dict.suggest("recieve", &mut out);
        println!("spellbook: one suggestion list in {:.1} ms: {out:?}", t.elapsed().as_secs_f64() * 1000.0);
        println!("spellbook: heap now +{}, peak +{}", mb(heap() - base), mb(peak() - base));
        std::mem::forget(dict); // keep it counted if both run
    }

    if which == "harper" || which == "both" {
        let h = heap();
        let t = Instant::now();
        let curated = FstDictionary::curated();
        println!("harper dictionary: load {:.0} ms, heap +{}", t.elapsed().as_secs_f64() * 1000.0, mb(heap() - h));
        let h = heap();
        let mut group = LintGroup::new_curated(curated.clone(), Dialect::British);
        println!("harper LintGroup: heap +{}", mb(heap() - h));
        let h = heap();
        let t = Instant::now();
        let mut lints = 0;
        for p in &paragraphs {
            let d = Document::new(p, &PlainEnglish, &*curated);
            lints += group.organized_lints(&d).values().map(Vec::len).sum::<usize>();
        }
        println!(
            "harper: every paragraph linted one at a time in {:.1} s ({lints} lints), heap +{} after (caches), peak during +{}",
            t.elapsed().as_secs_f64(),
            mb(heap() - h),
            mb(peak() - h)
        );
        // Again: every paragraph's chunks are now cached? (LRU of 1000 chunks)
        let t = Instant::now();
        for p in paragraphs.iter().take(200) {
            let d = Document::new(p, &PlainEnglish, &*curated);
            let _ = group.organized_lints(&d);
        }
        println!("harper: first 200 paragraphs again: {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
        let last = &paragraphs[paragraphs.len() - 5];
        let d = Document::new(last, &PlainEnglish, &*curated);
        let t = Instant::now();
        let _ = group.organized_lints(&d);
        println!("harper: a recently linted paragraph again (cache hit): {:.2} ms", t.elapsed().as_secs_f64() * 1000.0);
        println!("harper: total heap +{}, peak +{}", mb(heap() - base), mb(peak() - base));
    }
}
