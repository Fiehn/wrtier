//! Stress harness for the History engine.
//!
//! Writes realistic Histories at increasing sizes with a fixed seed, so every run does the
//! same work, then times what the Author feels: opening the Work and single edits on it.
//!
//!     cargo run --release --example stress                 # the default ladder
//!     cargo run --release --example stress -- 1000 30000   # chosen sizes (number of Changes)
//!
//! Not part of `cargo test`: timings depend on the machine and on debug vs release builds.

use std::time::{Duration, Instant};
use wrtier::change::Change;
use wrtier::doc::Doc;
use wrtier::ids::DeviceId;
use wrtier::storage;

/// Small fixed-seed random generator (xorshift64*). Written out here so the workload never
/// changes because a library updated its random streams.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// A number in 0..n.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    /// A number in lo..=hi.
    fn between(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }
}

const WORDS: &[&str] = &[
    "the", "rain", "had", "stopped", "and", "she", "walked", "along", "quiet", "harbour",
    "wall", "while", "gulls", "circled", "over", "boats", "that", "would", "not", "leave",
    "before", "morning", "light", "came", "back", "to", "town", "again", "slowly", "eased",
];

/// A burst of typing: whole words, sometimes ending a sentence or paragraph.
fn burst(rng: &mut Rng) -> String {
    let mut text = String::new();
    let target = rng.between(1, 40);
    while text.chars().count() < target {
        text.push_str(WORDS[rng.below(WORDS.len())]);
        text.push(match rng.below(20) {
            0 => '.',
            1 => '\n',
            _ => ' ',
        });
    }
    text
}

/// Writes `n` Changes the way one Author would: mostly typing where they last were.
/// Returns the Doc and its History.
fn write_history(n: usize, rng: &mut Rng) -> (Doc, Vec<Change>) {
    let mut doc = Doc::new(DeviceId(1));
    let mut changes: Vec<Change> = Vec::with_capacity(n);
    let mut cursor = 0usize;
    let mut now = 1_000_000u64;
    while changes.len() < n {
        now += 1_000;
        let len = doc.len();
        cursor = cursor.min(len);
        let change = match rng.below(100) {
            // Typing at the cursor.
            0..=69 => {
                let text = burst(rng);
                let change = doc.insert_local(cursor, &text, now);
                cursor += text.chars().count();
                change
            }
            // Backspacing.
            70..=84 => {
                if cursor == 0 {
                    continue;
                }
                let k = rng.between(1, 8).min(cursor);
                cursor -= k;
                doc.delete_local(cursor, k, now)
            }
            // Jumping somewhere else in the text (no Change).
            85..=94 => {
                cursor = rng.below(len + 1);
                continue;
            }
            // Moving a passage.
            95..=97 => {
                if len < 50 {
                    continue;
                }
                let k = rng.between(10, 300.min(len - 1));
                let from = rng.below(len - k + 1);
                let to = rng.below(len + 1);
                if to > from && to < from + k {
                    continue;
                }
                doc.move_local(from, k, to, now)
            }
            // Reverting one of the last 50 Changes.
            _ => {
                if changes.is_empty() {
                    continue;
                }
                let back = rng.below(changes.len().min(50));
                let target = changes[changes.len() - 1 - back].id;
                doc.revert_local(target, false, now)
            }
        };
        changes.push(change);
    }
    (doc, changes)
}

/// Median and slowest of a set of timings.
fn summary(mut times: Vec<Duration>) -> String {
    times.sort();
    let median = times[times.len() / 2];
    let max = *times.last().unwrap();
    format!("{:.2} ms (max {:.2})", ms(median), ms(max))
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn run(n: usize) {
    let mut rng = Rng(0x5EED_0000 + n as u64);

    let start = Instant::now();
    let (_, changes) = write_history(n, &mut rng);
    let write_time = start.elapsed();

    // Opening the Work: replay the whole History into a fresh Doc.
    let start = Instant::now();
    let mut doc = Doc::new(DeviceId(1));
    doc.apply_all(&changes);
    let open_time = start.elapsed();
    let len = doc.len();

    // Single edits on the full text, at random places.
    let mut now = 10_000_000_000u64;
    let mut time = |doc: &mut Doc, f: &mut dyn FnMut(&mut Doc, u64)| {
        now += 1_000;
        let start = Instant::now();
        f(doc, now);
        start.elapsed()
    };
    let keystrokes: Vec<Duration> = (0..20)
        .map(|_| {
            let at = rng.below(doc.len() + 1);
            time(&mut doc, &mut |d, t| { d.insert_local(at, "x", t); })
        })
        .collect();
    let deletes: Vec<Duration> = (0..5)
        .map(|_| {
            let at = rng.below(doc.len());
            time(&mut doc, &mut |d, t| { d.delete_local(at, 1, t); })
        })
        .collect();
    let moves: Vec<Duration> = (0..5)
        .map(|_| {
            let k = 200.min(doc.len() / 2);
            let from = rng.below(doc.len() - k);
            let to = if from > k { 0 } else { doc.len() };
            time(&mut doc, &mut |d, t| { d.move_local(from, k, to, t); })
        })
        .collect();
    let reverts: Vec<Duration> = (0..5)
        .map(|i| {
            let target = changes[changes.len() - 1 - i].id;
            time(&mut doc, &mut |d, t| { d.revert_local(target, false, t); })
        })
        .collect();

    // A Change arriving from another Device that has the same History.
    let mut other = Doc::new(DeviceId(2));
    other.apply_all(&changes);
    let remote: Vec<Duration> = (0..5)
        .map(|_| {
            let at = rng.below(other.len() + 1);
            now += 1_000;
            let change = other.insert_local(at, "y", now);
            let start = Instant::now();
            doc.apply(&change);
            start.elapsed()
        })
        .collect();

    // Opening from a real file: SQLite reading included.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("work.db");
    storage::save(&path, &changes, &doc).unwrap();
    let start = Instant::now();
    let (_, loaded) = storage::load(&path, DeviceId(1)).unwrap();
    let file_time = start.elapsed();
    assert_eq!(loaded.len(), len, "the file opens to the same text");

    println!(
        "{:>7} Changes {:>8} chars | write {:>7.0} ms | open {:>6.1} ms (from file {:>6.1} ms) | keystroke {} | delete {} | move {} | revert {} | from another Device {}",
        n, len, ms(write_time), ms(open_time), ms(file_time),
        summary(keystrokes), summary(deletes), summary(moves), summary(reverts), summary(remote),
    );
}

fn main() {
    let sizes: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let sizes = if sizes.is_empty() { vec![1_000, 3_000, 10_000, 30_000] } else { sizes };
    for n in sizes {
        run(n);
    }
}
