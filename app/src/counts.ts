// Counting a Work's words, sentences and characters, as a reader would see them: Typst's markup
// (function names, brackets, emphasis marks, heading marks, escapes) and comments aren't counted,
// and neither are line breaks. Used by the word count Mod and the Chapter list.

/**
 * A run of word characters: letters, digits and apostrophes, exactly as the History engine
 * counts them (history.rs `is_word_char`: Rust's `is_alphanumeric` is Unicode's Alphabetic, which
 * takes in combining vowel signs \p{L} leaves out, or Numeric). It's a word only if it has a
 * letter or digit (`WORD_CHAR`): a closing quote on its own isn't one.
 */
const WORD = /[\p{Alphabetic}\p{N}'’]+/gu;
const WORD_CHAR = /[\p{Alphabetic}\p{N}]/u;

/** The words in some text with its markup taken out. */
function wordsOfProse(prose: string): number {
  let count = 0;
  for (const m of prose.matchAll(WORD)) if (WORD_CHAR.test(m[0])) count++;
  return count;
}

/** The text with its markup and comments taken out (line breaks kept, so lines stay apart). */
export function proseOf(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/\/\/[^\n]*/g, "")
    .replace(/#[A-Za-z][\w-]*(\(\))?/g, "")
    .replace(/^[ \t]*=+[ \t]+/gm, "")
    // Emphasis marks and brackets go; an escaped character (`\*`) is text and stays.
    .replace(/\\(.)|[*_[\]]/g, (_, escaped: string | undefined) => escaped ?? "")
    .replace(/[ \t]+$/gm, ""); // spaces left at a line's end (before a comment, say) aren't seen
}

/** The words in some Typst text. */
export function wordsIn(text: string): number {
  return wordsOfProse(proseOf(text));
}

/** Titles that end in a full stop without ending a sentence ("Mr. Holm"). */
const TITLES = new Set(["mr", "mrs", "ms", "dr", "st", "jr", "sr", "prof", "mt", "vs", "etc", "no", "vol", "ch", "fig"]);

/** Where a sentence can end: its marks, then any closing quotes or brackets. */
const SENTENCE_END = /[.!?…]+["'”’»)\]]*/gu;

/** What starts the next sentence after an end mark: a capital, a digit, an opening quote or bracket, a dash. */
const SENTENCE_START = /^[\p{Lu}\p{Lt}\p{N}"'“‘«„(\[¿¡—–-]/u;

/**
 * The sentences in some text with its markup taken out. Typst joins a paragraph's lines, so a
 * sentence runs on across a line break and ends at a blank line. Within a paragraph, a sentence
 * ends at . ! ? or … (and any closing quotes) when what follows starts a new one; so in
 * “Go!” she said. the “!” ends nothing. A title (“Mr.”) or an initial (“J. R.”) ends nothing
 * either. Words left at a paragraph's end without a mark are a sentence too.
 */
export function sentencesOfProse(prose: string): number {
  let count = 0;
  for (const paragraph of prose.split(/\n[ \t]*\n/)) {
    const text = paragraph.replace(/\s+/g, " ").trim();
    let from = 0;
    for (const m of text.matchAll(SENTENCE_END)) {
      const end = m.index + m[0].length;
      const rest = text.slice(end);
      if (rest !== "" && !rest.startsWith(" ")) continue; // inside a word or a number: "U.S." "3.5"
      const next = rest.trimStart();
      if (next !== "" && !SENTENCE_START.test(next)) continue;
      if (m[0] === ".") {
        const before = text.slice(from, m.index).match(/(\p{L}+)$/u)?.[1] ?? "";
        if (TITLES.has(before.toLowerCase()) || /^\p{Lu}$/u.test(before)) continue;
      }
      if (WORD_CHAR.test(text.slice(from, end))) count++;
      from = end;
    }
    if (WORD_CHAR.test(text.slice(from))) count++;
  }
  return count;
}

/** The text without its headings, which aren't sentences (a Chapter's title). */
const withoutHeadings = (text: string) => text.replace(/^[ \t]*=+[ \t]+[^\n]*/gm, "");

/** The sentences in some Typst text; headings aren't. */
export function sentencesIn(text: string): number {
  return sentencesOfProse(proseOf(withoutHeadings(text)));
}

export type Counts = { words: number; sentences: number; characters: number; charactersNoSpaces: number };

/** Words, sentences and characters (with and without spaces; never line breaks) in some Typst text. */
export function countsOf(text: string): Counts {
  const prose = proseOf(text);
  const words = wordsOfProse(prose);
  const sentences = sentencesIn(text);
  let characters = 0;
  let spaces = 0;
  for (const c of prose) {
    if (c === "\n" || c === "\r") continue;
    characters++;
    if (/\s/.test(c)) spaces++;
  }
  return { words, sentences, characters, charactersNoSpaces: characters - spaces };
}

const n = (x: number) => x.toLocaleString();

/**
 * "12,340 words · 812 sentences · 68,201 characters", or with a selection "120 of 12,340 words ·
 * …". With Inclusions, how many of the words are included: "48,200 words (42,100 included) · …".
 */
export function countsLine(all: Counts, selected: Counts | null, included = 0): string {
  const part = (key: "words" | "sentences" | "characters", one: string) =>
    selected ? `${n(selected[key])} of ${n(all[key])} ${one}s` : `${n(all[key])} ${one}${all[key] === 1 ? "" : "s"}`;
  const words = part("words", "word") + (included > 0 ? ` (${n(included)} included)` : "");
  return `${words} · ${part("sentences", "sentence")} · ${part("characters", "character")}`;
}

/** How long the sentences are on average, in words ("14.2"); null without any. */
export function wordsPerSentence(counts: Counts): string | null {
  return counts.sentences > 0 ? (counts.words / counts.sentences).toLocaleString(undefined, { maximumFractionDigits: 1 }) : null;
}
