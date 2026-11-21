//! Spike: where the curated dictionary's memory goes. `cargo run --release --bin dictsize`

use harper_core::spell::{Dictionary, FstDictionary, MutableDictionary};
use harper_core::{CharString, DictWordMetadata};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

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

fn main() {
    let base = CURRENT.load(Relaxed);
    let m = MutableDictionary::curated();
    let after_mutable = CURRENT.load(Relaxed);
    println!("MutableDictionary::curated() alone (copy A, a static): +{}", mb(after_mutable - base));
    println!("  words: {}", m.word_count());
    println!("  size_of DictWordMetadata = {} bytes, CharString = {} bytes", size_of::<DictWordMetadata>(), size_of::<CharString>());
    println!("  so the entries' inline part alone ≈ {}", mb(m.word_count() * (size_of::<DictWordMetadata>() + size_of::<CharString>() + 16)));
    PEAK.store(CURRENT.load(Relaxed), Relaxed);
    let before = CURRENT.load(Relaxed);
    let f = FstDictionary::curated();
    let after = CURRENT.load(Relaxed);
    println!("FstDictionary::curated() on top (copy C + the FST): +{} (peak while building +{})", mb(after - before), mb(PEAK.load(Relaxed) - before));
    println!("both: +{}", mb(after - base));
    // How much of the metadata is ever non-default? (the cost is the struct's size either way)
    let derived = m.words_iter().filter(|w| m.get_word_metadata(w).is_some_and(|md| !md.derived_from.is_empty())).count();
    println!("  words with a non-empty derived_from (a BTreeSet each): {derived}");
    drop(f);
    let before = CURRENT.load(Relaxed);
    let t = harper_core::spell::TrieDictionary::curated();
    println!("TrieDictionary::curated() (SplitWords): +{}", mb(CURRENT.load(Relaxed) - before));
    drop(t);
    // The chunker and tagger: parse 20,000 different short sentences, watch the heap.
    let before = CURRENT.load(Relaxed);
    let d0 = harper_core::Document::new_plain_english_curated("Warm up.");
    drop(d0);
    println!("tagger + chunker loaded: +{}", mb(CURRENT.load(Relaxed) - before));
    let before = CURRENT.load(Relaxed);
    for i in 0..20_000 {
        let d = harper_core::Document::new_plain_english_curated(&format!("The {i} cats sat on mat number {}.", i * 7));
        drop(d);
        if i == 9_999 { println!("after 10,000 distinct sentences: +{}", mb(CURRENT.load(Relaxed) - before)); }
    }
    println!("after 20,000 distinct sentences: +{} (the chunker LRU holds 10,000)", mb(CURRENT.load(Relaxed) - before));
}
