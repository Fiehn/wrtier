// Counting a Work's words and characters, as a reader would see them: Typst's markup (function
// names, brackets, emphasis marks, heading marks, escapes) and comments aren't counted, and
// neither are line breaks. Used by the word and character count Mod and the Chapter list.

/** A word: letters, digits and apostrophes, as the History engine counts them. */
const WORD = /[\p{L}\p{N}'’]+/gu;

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
  let count = 0;
  for (const m of proseOf(text).matchAll(WORD)) if (/[\p{L}\p{N}]/u.test(m[0])) count++;
  return count;
}

export type Counts = { words: number; characters: number; charactersNoSpaces: number };

/** Words and characters (with and without spaces; never line breaks) in some Typst text. */
export function countsOf(text: string): Counts {
  const prose = proseOf(text);
  let words = 0;
  for (const m of prose.matchAll(WORD)) if (/[\p{L}\p{N}]/u.test(m[0])) words++;
  let characters = 0;
  let spaces = 0;
  for (const c of prose) {
    if (c === "\n" || c === "\r") continue;
    characters++;
    if (/\s/.test(c)) spaces++;
  }
  return { words, characters, charactersNoSpaces: characters - spaces };
}

const n = (x: number) => x.toLocaleString();

/**
 * "12,340 words · 68,201 characters", or with a selection "120 of 12,340 words · …". With
 * Inclusions, how many of the words are included: "48,200 words (42,100 included) · …".
 */
export function countsLine(all: Counts, selected: Counts | null, included = 0): string {
  const part = (key: "words" | "characters", unit: string) =>
    selected ? `${n(selected[key])} of ${n(all[key])} ${unit}` : `${n(all[key])} ${unit}`;
  const words = part("words", all.words === 1 && !selected ? "word" : "words") + (included > 0 ? ` (${n(included)} included)` : "");
  return `${words} · ${part("characters", all.characters === 1 && !selected ? "character" : "characters")}`;
}
