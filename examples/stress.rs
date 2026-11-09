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
        // A new sitting every 150 Changes or so, and now and then a Milestone, so the History
        // view has Sessions to list and cut. (Counted from the Changes, so the random stream,
        // and with it the rest of the workload, stays the same.)
        if changes.len() % 150 == 149 {
            now += wrtier::doc::SESSION_GAP_MS + 1;
        }
        if changes.len() % 2_000 == 1_999 {
            changes.push(doc.make_milestone_local("Draft", now));
            continue;
        }
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

    // An External edit: one word changed in the middle of the Text file.
    let version = doc.version();
    let written = doc.committed_text();
    let middle = written.char_indices().nth(written.chars().count() / 2).map_or(0, |(i, _)| i);
    let edited = format!("{}EDITED{}", &written[..middle], &written[middle..]);
    now += 1_000;
    let start = Instant::now();
    let imported = doc.import_text(Some(&version), &edited, now);
    let import_time = start.elapsed();
    assert!(!imported.is_empty());

    // The History view: every Session listed and counted (the tab shows the newest 50).
    let start = Instant::now();
    let sessions = doc.sessions();
    let sessions_time = start.elapsed();
    let session_count = sessions.len();
    // Opening a Session: its snippets. The newest 20, as the Author would open them.
    let snippet_times: Vec<Duration> = sessions.iter().take(20)
        .map(|part| {
            let start = Instant::now();
            doc.snippets(&part.changes);
            start.elapsed()
        })
        .collect();

    // Comparing the oldest Milestone with now, and the History of the paragraph in the middle.
    // (A History under 2,000 Changes has no Milestone: then the oldest Session's start stands in, as now vs now.)
    let oldest = doc.milestones().first().map(|m| m.id);
    let start = Instant::now();
    let then = oldest.map_or(wrtier::doc::Version::Now, wrtier::doc::Version::Milestone);
    let compared = doc.compare(then, wrtier::doc::Version::Now);
    let compare_time = start.elapsed();
    let start = Instant::now();
    let stretch = doc.stretch_at(doc.committed_text().chars().count() / 2, 0).expect("a paragraph");
    let touched = doc.changes_in(&stretch);
    doc.snippets_in(&touched, &stretch);
    let history_time = start.elapsed();

    // Previews: reverting the newest Session (with what it leaves out and its dependents), and
    // returning to the oldest Milestone (the comparison, what it reverts, Comments it detaches).
    let start = Instant::now();
    let newest = &sessions[0].changes;
    let targets = doc.revertable(newest);
    doc.revert_preview(&targets);
    for (_, group) in doc.by_session(&doc.dependents(&targets)) {
        doc.revert_preview(&group);
    }
    doc.left_out(newest);
    let revert_preview_time = start.elapsed();
    let start = Instant::now();
    if let Some(m) = oldest {
        doc.compare(wrtier::doc::Version::Now, wrtier::doc::Version::Milestone(m));
        doc.return_preview(m);
        doc.detached_by_return(m);
    }
    let return_preview_time = start.elapsed();

    // "Colour text by Session": the newest 5 Sessions' text, asked for each time the History grows.
    let start = Instant::now();
    let tinted = doc.newest_sessions_text(5);
    let tint_time = start.elapsed();

    // Opening from a real file: SQLite reading included.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("work.db");
    storage::save(&path, &changes, &doc).unwrap();
    let start = Instant::now();
    let (_, loaded) = storage::load(&path, DeviceId(1)).unwrap();
    let file_time = start.elapsed();
    assert_eq!(loaded.len(), len, "the file opens to the same text");

    println!(
        "{:>7} Changes {:>8} chars | write {:>7.0} ms | open {:>6.1} ms (from file {:>6.1} ms) | keystroke {} | delete {} | move {} | revert {} | from another Device {} | External edit {:.1} ms | {} Sessions listed in {:.1} ms | opening one {} | Milestone vs now {:.1} ms ({} snippets) | History of a paragraph {:.1} ms ({} Changes) | Revert preview {:.1} ms | return preview {:.1} ms | colour by Session {:.1} ms ({} ranges)",
        n, len, ms(write_time), ms(open_time), ms(file_time),
        summary(keystrokes), summary(deletes), summary(moves), summary(reverts), summary(remote), ms(import_time),
        session_count, ms(sessions_time), summary(snippet_times),
        ms(compare_time), compared.snippets.len(), ms(history_time), touched.len(),
        ms(revert_preview_time), ms(return_preview_time),
        ms(tint_time), tinted.iter().map(|s| s.ranges.len()).sum::<usize>(),
    );
}

fn main() {
    let sizes: Vec<usize> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let sizes = if sizes.is_empty() { vec![1_000, 3_000, 10_000, 30_000] } else { sizes };
    for n in sizes {
        run(n);
    }
}
