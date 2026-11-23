// The Grammar checker's reading of a Work (CONTEXT.md, ADR 0023): each passage as Harper is sent
// it, and what Harper found as Marks whose Suggestions edit the real source. Pure: Harper itself
// is Rust's (grammar.rs), asked by grammarmod.ts.
//
// The text is read as it shows (prose.ts), as the Dialogue checker reads it: emphasis seen through,
// `--`, `---` and `...` as their glyphs, each Footnote a passage of its own, and headings, Story
// elements, comments, code, raw, math, labels and links left out. Harper gets each passage on one
// line, as Typst joins it. Where something was left out between two words, Harper sees a space,
// and nothing Harper finds across that place is marked: a Suggestion there would edit the code.

import type { Mark, MarkLevel, MarkSuggestion } from "./marks";
import { BREAK, preview, toSource, type Para, type ReadEdit } from "./prose";

/** What Harper found in a passage (grammar.rs `Found`): `from`..`to` in the text it was sent. */
export type Found = { from: number; to: number; rule: string; kind: string; message: string; suggestions: Fix[] };
/** Replace what's found with `text`, insert `text` after it, or remove it. */
export type Fix = { kind: "replace" | "insert" | "remove"; text: string };

/** A passage as Harper is sent it: `at[i]` is where `text[i]` is in the passage as it reads, -1 for a space standing in for something left out. */
export type Sent = { para: Para; text: string; at: number[] };

const WORDY = /[\p{L}\p{M}\p{N}]/u;

export function toSend(para: Para): Sent {
  let text = "";
  const at: number[] = [];
  const t = para.text;
  for (let i = 0; i < t.length; i++) {
    if (t[i] !== BREAK) {
      text += /\s/.test(t[i]) ? " " : t[i];
      at.push(i);
      continue;
    }
    // Something left out: a space if it sat between two words, else nothing, and never two spaces.
    while (i + 1 < t.length && t[i + 1] === BREAK) i++;
    if (WORDY.test(text.slice(-1)) && WORDY.test(t[i + 1] ?? "")) {
      text += " ";
      at.push(-1);
    } else if (/\s$/.test(text)) {
      while (i + 1 < t.length && /\s/.test(t[i + 1])) i++;
    }
  }
  return { para, text, at };
}

/** Harper's kinds of finding, as the three kinds of Mark (CONTEXT.md). */
const MISTAKES = new Set(["Typo", "Spelling", "Repetition", "BoundaryError", "Eggcorn", "Malapropism"]);
const RULE_BREAKS = new Set(["Grammar", "Agreement", "Punctuation", "Capitalization", "WordOrder", "Usage", "Nonstandard", "Formatting"]);

/**
 * A Mistake (red) where the text is surely wrong: a typo, a repeated word, a word mistaken for one
 * that sounds like it. A Rule break (yellow) for grammar and punctuation, which fiction breaks on
 * purpose. A Query (blue) for style and word choice, and anything else.
 */
export function levelOf(kind: string): MarkLevel {
  if (MISTAKES.has(kind)) return "mistake";
  if (RULE_BREAKS.has(kind)) return "rule";
  return "query";
}

/** The edit a Fix makes to the passage as it reads, `a`..`b` being what's found there; null if it changes nothing. */
function readEdit(s: Sent, found: Found, a: number, b: number, fix: Fix): ReadEdit | null {
  if (fix.kind === "insert") return fix.text ? { at: b, to: b, insert: fix.text } : null;
  if (fix.kind === "remove") return { at: a, to: b, insert: "" };
  // Only what differs changes, so markup in the unchanged part stays where it is.
  const typed = s.text.slice(found.from, found.to);
  const next = fix.text;
  let pre = 0;
  while (pre < typed.length && pre < next.length && typed[pre] === next[pre]) pre++;
  let post = 0;
  while (post < typed.length - pre && post < next.length - pre && typed[typed.length - 1 - post] === next[next.length - 1 - post]) post++;
  if (pre + post === typed.length && pre + post === next.length) return null;
  return { at: a + pre, to: b - post, insert: next.slice(pre, next.length - post) };
}

/** What Harper found in a passage, as Marks (without their Mod), in the text as typed. */
export function marksFrom(s: Sent, found: Found[], mod: string): Mark[] {
  const p = s.para;
  const marks: Mark[] = [];
  for (const f of found) {
    if (f.from < 0 || f.from >= f.to || f.to > s.at.length) continue;
    const span = s.at.slice(f.from, f.to);
    if (span.some((i) => i < 0)) continue; // across something left out
    const a = span[0];
    const b = span[span.length - 1] + 1;
    if (p.text.slice(a, b).includes(BREAK)) continue;
    const suggestions: MarkSuggestion[] = [];
    for (const fix of f.suggestions) {
      const e = readEdit(s, f, a, b, fix);
      if (!e) continue;
      const label = preview(p, [e]);
      // Letters added inside what's found go right after the letter before them, inside any markup around it ("*a*" → "*an*").
      const edit = fix.kind === "replace" && e.at === e.to && e.at > a ? { from: p.to[e.at - 1], to: p.to[e.at - 1], insert: e.insert } : toSource(p, e);
      if (!suggestions.some((x) => x.label === label)) suggestions.push({ label, edits: [edit] });
    }
    marks.push({ mod, rule: f.rule, level: levelOf(f.kind), from: p.from[a], to: p.to[b - 1], why: f.message, suggestions });
  }
  return marks;
}
