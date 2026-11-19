// The Spellchecker's reading of a Work (CONTEXT.md, ADR 0019): the words to check, which look
// like names, which the Author's own words cover, and the edits a Suggestion makes. Pure: the
// Dictionary itself is Rust's (spelling.rs), asked by spellmod.ts.
//
// The text is read as it shows (prose.ts), headings and the text inside Story elements included;
// code (anything starting with `#`), comments, raw, math, labels and links never are. A word is
// letters with apostrophes inside it ("don’t"); a hyphen splits a compound into words checked
// one by one ("half-elf"), and a word with a digit in it ("3rd") isn't checked. Straight and
// curly apostrophes are the same. Every word keeps its place in the text as typed, emphasis
// markup inside it included, so a Suggestion edits the real source and leaves the markup be.

import type { Text } from "@codemirror/state";
import { BREAK, notesOf, passagesOf, readPara, splitAtSeams, type Para, type Reading } from "./prose";

const READING: Reading = { headings: true, elements: true };

/** A word to check: where it is in the text as typed, how it reads, and whether it looks like a name. */
export type SpellWord = {
  from: number;
  to: number;
  /** As it reads, with straight apostrophes: what the Dictionary is asked about. */
  word: string;
  /** Capitalised where no sentence starts: a name, most likely. */
  midSentence: boolean;
};

const WORD = /[\p{L}\p{M}\p{N}]+(?:['’][\p{L}\p{M}\p{N}]+)*/gu;
const DIGIT = /\p{N}/u;
const isUpper = (c: string | undefined) => c !== undefined && c !== c.toLowerCase() && c === c.toUpperCase();

/** Straight apostrophes for curly ones: the Dictionaries spell "don't" with a straight one. */
export const plain = (word: string) => word.replace(/’/g, "'");

/** Does a sentence start right before `at` in `text`: nothing before it, or a sentence's end, a break, a quote or a bracket? */
function sentenceStartsAt(text: string, at: number): boolean {
  let j = at - 1;
  while (j >= 0 && /\s/.test(text[j])) j--;
  return j < 0 || /[.!?…:;"“”'‘—–(\[\u0000]/.test(text[j]);
}

function wordsOfPara(p: Para, out: SpellWord[]) {
  WORD.lastIndex = 0;
  for (let m = WORD.exec(p.text); m; m = WORD.exec(p.text)) {
    // A word never runs across a Footnote (prose.ts): `word#footnote[…]s` is two.
    for (const [at, end] of splitAtSeams(p, m.index, m.index + m[0].length)) {
      const text = p.text.slice(at, end);
      if (DIGIT.test(text) || !/[\p{L}\p{M}\p{N}]/u.test(text)) continue;
      out.push({
        from: p.from[at],
        to: p.to[end - 1],
        word: plain(text),
        midSentence: isUpper(text[0]) && !sentenceStartsAt(p.text, at),
      });
    }
  }
}

/** The words of a Work's text to check, in reading order (each Footnote's after its paragraph's). */
export function wordsOf(source: string): SpellWord[] {
  const out: SpellWord[] = [];
  for (const r of passagesOf(source)) wordsOfPara(readPara(source, r.start, r.end, READING), out);
  return out;
}

/** The words a Work has with a capital where no sentence starts: its names, most likely. */
export function namesOf(words: SpellWord[]): Set<string> {
  return new Set(words.filter((w) => w.midSentence && !isAllCaps(w.word)).map((w) => w.word));
}

const isAllCaps = (word: string) => word.length > 1 && word === word.toUpperCase() && word !== word.toLowerCase();

// ---- The Author's own words ----

/**
 * The Author's own words (Added words and Allowed words), as Hunspell treats a word added to it:
 * one written with capitals is right with them or in all capitals ("Zampanò", "ZAMPANÒ"); one in
 * small letters is right however it's capitalised.
 */
export class OwnWords {
  private exact = new Set<string>();
  private anyCase = new Set<string>();
  private shouted = new Set<string>();
  readonly all: string[];

  constructor(words: Iterable<string>) {
    this.all = [...new Set([...words].map(plain))];
    for (const w of this.all) {
      if (w === w.toLowerCase()) this.anyCase.add(w);
      else this.exact.add(w);
      this.shouted.add(w.toUpperCase());
    }
  }

  knows(word: string): boolean {
    const w = plain(word);
    return this.exact.has(w) || this.anyCase.has(w.toLowerCase()) || (isAllCaps(w) && this.shouted.has(w));
  }
}

/** What to keep when the Author adds a word: as typed, but a capital only there because a sentence starts is dropped. */
export function wordToKeep(word: string, nameLike: boolean): string {
  const w = plain(word);
  if (nameLike || isAllCaps(w) || w.slice(1) !== w.slice(1).toLowerCase()) return w;
  return w.toLowerCase();
}

// ---- Suggestions ----

/** How many edits (a letter added, taken away, changed, or two swapped) apart `a` and `b` are, up to `max`; `max + 1` beyond. */
export function distance(a: string, b: string, max = 2): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  const prev2: number[] = [];
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    let best = i;
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      let d = Math.min(prev[j] + 1, row[j - 1] + 1, prev[j - 1] + cost);
      if (i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) d = Math.min(d, prev2[j - 2] + 1);
      row.push(d);
      best = Math.min(best, d);
    }
    if (best > max) return max + 1;
    prev2.splice(0, prev2.length, ...prev);
    prev = row;
  }
  return Math.min(prev[b.length], max + 1);
}

/** A suggestion capitalised as the word was typed: "Teh" → "The", "TEH" → "THE"; one with capitals of its own keeps them. */
export function capitalisedLike(typed: string, word: string): string {
  if (isAllCaps(typed)) return word.toUpperCase();
  if (isUpper(typed[0]) && word[0] === word[0]?.toLowerCase()) return word[0].toUpperCase() + word.slice(1);
  return word;
}

/** At most this many Suggestions on a spelling Mark. */
export const MAX_SUGGESTIONS = 5;

/**
 * The words to offer for `typed`: the Author's own one letter away first (they're this Work's
 * words), then the Dictionary's, best first, then the Author's own two letters away; each
 * capitalised as typed, at most five, none twice.
 */
export function suggestionsFor(typed: string, dictionary: string[], own: string[]): string[] {
  const w = plain(typed);
  const near = (d: number) => own.filter((o) => distance(w.toLowerCase(), o.toLowerCase()) === d);
  const out: string[] = [];
  for (const s of [...near(1), ...dictionary, ...near(2)]) {
    const c = capitalisedLike(w, plain(s));
    if (c !== w && !out.some((o) => o.toLowerCase() === c.toLowerCase())) out.push(c);
    if (out.length === MAX_SUGGESTIONS) break;
  }
  return out;
}

/** One paragraph of `doc` as it reads: the one holding `pos`. */
function paraAt(doc: Text, pos: number): Para {
  let first = doc.lineAt(pos);
  while (first.number > 1 && doc.line(first.number - 1).text.trim() !== "") first = doc.line(first.number - 1);
  let last = doc.lineAt(pos);
  while (last.number < doc.lines && doc.line(last.number + 1).text.trim() !== "") last = doc.line(last.number + 1);
  const source = doc.sliceString(0, last.to);
  // In a Footnote: its text, read apart (the innermost one holding `pos`).
  const note = notesOf(source, first.from, last.to).filter((n) => n.start <= pos && pos <= n.end).pop();
  if (note) return readPara(source, note.start, note.end, READING);
  return readPara(source, first.from, last.to, READING);
}

/**
 * The edits that make the word under `from..to` read `replacement`: only the letters that differ
 * change, and only letters are touched, so emphasis markup inside the word stays where it is
 * (`un*beliv*able` → `un*believ*able`). Apostrophes follow the word's: curly if it has one.
 */
export function editsFor(doc: Text, from: number, to: number, replacement: string): { from: number; to: number; insert: string }[] {
  const p = paraAt(doc, from);
  const chars: { from: number; to: number; ch: string }[] = [];
  for (let i = 0; i < p.text.length; i++) {
    if (p.from[i] >= from && p.to[i] <= to && p.text[i] !== BREAK) chars.push({ from: p.from[i], to: p.to[i], ch: p.text[i] });
  }
  const typed = chars.map((c) => c.ch).join("");
  const next = typed.includes("’") ? replacement.replace(/'/g, "’") : replacement;
  if (chars.length === 0) return [{ from, to, insert: next }];
  let pre = 0;
  while (pre < typed.length && pre < next.length && typed[pre] === next[pre]) pre++;
  let post = 0;
  while (post < typed.length - pre && post < next.length - pre && typed[typed.length - 1 - post] === next[next.length - 1 - post]) post++;
  const gone = chars.slice(pre, chars.length - post);
  const insert = next.slice(pre, next.length - post);
  if (gone.length === 0) {
    const at = pre > 0 ? chars[pre - 1].to : chars[0].from;
    return [{ from: at, to: at, insert }];
  }
  // The letters going, in runs next to each other in the source (markup between them stays).
  // The new letters go where the most go, so no markup is left around nothing.
  const runs: { from: number; to: number; n: number }[] = [];
  for (const c of gone) {
    const last = runs[runs.length - 1];
    if (last && last.to === c.from) {
      last.to = c.to;
      last.n++;
    } else runs.push({ from: c.from, to: c.to, n: 1 });
  }
  const most = runs.reduce((best, r, i) => (r.n > runs[best].n ? i : best), 0);
  return runs.map((r, i) => ({ from: r.from, to: r.to, insert: i === most ? insert : "" }));
}
