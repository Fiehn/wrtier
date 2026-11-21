# Spike: Harper for grammar, word types and spelling

Throwaway code to see how `harper-core` 3.2.0 behaves before planning spellbook + Harper.

- `cargo run --release -- <novel.txt>`: everything (`results.txt`, with memory counted).
- `cargo run --release --no-default-features --features thesaurus -- <novel.txt>`: the same timings
  without the counting allocator (`results-nocount.txt`). The times below come from this run.
- `cargo run --release --bin memory -- <novel.txt> <harper|spellbook>`: memory in a fresh
  process, Harper used one paragraph at a time, and spellbook with the app's en-GB dictionary.

The novel was Pride and Prejudice from Project Gutenberg: 2,509 paragraphs and 127k words,
one line per paragraph as Typst would join them. Timings are from one desktop machine with
12 threads.

## What was unclear, and the answer

| Question | Answer |
|---|---|
| British English | Works. `Dialect::try_from_bcp47("en-GB")` gives British. colour/realise/licence/practise/travelling/theatre/tyres pass, and color/gray/traveling/theater are flagged with the British spelling suggested first. The reverse holds for American. |
| Added/Allowed words at runtime | Works. A `MergedDictionary` (curated + `MutableDictionary`) passed to both `LintGroup::new_curated` **and** `Document::new` stops them being flagged. Rebuilding with 1,000 added words takes 11 ms. However, "Kvothe's", "KVOTHE" and "kvothe" are still flagged: an added word has no possessive or case forms, so we'd add those ourselves or filter them. |
| Word types | Works. `TokenKind::Word(Some(meta))`, with `meta.pos_tag: Option<UPOS>` set from context: "record/VERB a record/NOUN", "saw/VERB the saw/NOUN". It is fooled by garden paths ("Time flies/NOUN"), and the chunker gives noun-phrase membership. **Unknown words get no tag at all** (`Word(None)`), not even PROPN, so names the dictionary doesn't know have no word type. `UPOS` comes from `harper-brill`, which harper-core doesn't re-export, so we'd depend on it too. |
| Offsets | `Span<char>` counts Unicode characters. After an emoji, the same index read as UTF-16 (JavaScript) lands one place early. We'd convert (cheap) or send UTF-16-safe text. |
| Grammar | Catches a/an, their/they're, could of, it's/its (possessive), then/than, to loose, effect/affect, repeated words, he go, she don't, should have went, comma spacing, discuss about, on accident, a lot. Misses: "Its going to rain", "Your going to", "the children was", "Me and him went", "I seen", "There is many", "Who's book", "between you and I", "less people", "the principle of the school", "sentence.This". |
| False alarms on fiction | Fine on dialogue, fragments and dashes. It flags "had had" (repeated word), "Mr. and Mrs." (thinks "and" starts a sentence), and dialect or archaic words ("s'pose", "goest"). On the novel it found 26 lints per 1,000 words: SpellCheck 1,853, LongSentences 551, Dashes 442 (for `--`), SentenceCapitalization 105, then a long tail. The tail is mostly wrong for period prose: TheirToTheyre on "their being", ItsContraction on "its being", PronounContraction on "Were the whole…", AnA on "an union". |

## Cost

| | Harper | spellbook (en-GB) |
|---|---|---|
| Load | 460 ms dictionary + 134 ms rule set + 12 ms tagger/chunker | 10 ms |
| Heap | **124 MB** dictionary + 11.5 MB rule set + 4.6 MB tagger | **2.1 MB** |
| Release binary | 16.3 MB (baseline exe 0.1 MB) | already in the app |
| Spelling, whole novel | 2.3 s (the SpellCheck rule) | 1.9 ms for the 7,263 distinct words |

The dictionary keeps two full copies of the word list: a `MutableDictionary` behind a static,
plus a clone inside the `FstDictionary`. It lives as long as the process does.

The rule set has 971 rules, 959 of them on by default.

| Run | Time |
|---|---|
| Parse the whole novel (tokenise, tag, chunk) | 7.8 s (16k words/s) |
| Lint the whole novel | 26.7 s (5k words/s); 24.8 s again with the cache "warm" |
| Every paragraph as its own Document | 24.8 s, same lints (3,316), so it's linear |
| 12 threads, one rule set each | 9.5 s |
| One paragraph (16 words), parse + lint | 4.4 ms |
| A first `Document` on a new thread | 7 ms (thread-local chunker) |

The time is spread across all the rules, about 2 ms each per 10k words. On 10k words, no rules
at all costs 37 ms of the group's own overhead, and all rules together take 2.1 s. Turning off 7
style rules plus SpellCheck saved almost nothing (23.6 s). Only `BoringWords` stood out, at
449 ms per 10k words.

### The rule set's cache works against a long Work

`LintGroup` keeps an LRU cache of 1,000 chunks and 1,000 sentences. Each entry is a map with
about 960 rule-name keys. The cache size is fixed and there's no public way to clear it.

- Linting a novel paragraph by paragraph through one group grew the heap by **172 MB** (peak 190).
- A cache hit on a recently linted paragraph took 26 ms, compared with 4–5 ms cold.
- A fresh `LintGroup` (134 ms, 11.5 MB) is the only way to reset it.

Total in the paragraph-at-a-time run: **308 MB** heap above start.
