// Find and replace in the Work in front (the v2 plan). Matching follows the view: in the
// Formatted view it matches the text as it reads, with hidden markup (`*`, `_`, backticks, `\`
// escapes, heading marks, Story element brackets) skipped and `--`, `---` and `...` read as the
// dashes and ellipsis they show; in the Raw view, every character as typed. No match runs across
// a Story element's chip or brackets, or an Inclusion. Included text is searched too, as its
// block shows it (the Raw view: its `#inclusion(…)` line), but never changed here.
//
// Case is ignored unless asked; straight and curly quotes and apostrophes always match each
// other; dashes and accents never do. Replacing compares a match with its replacement word by
// word, so only the words that differ are rewritten: markup inside stays put, and the rest
// keeps its History. With case ignored, the replacement copies the match's capitals.

import { EditorState, StateEffect, StateField, type Extension, type Text } from "@codemirror/state";
import { Decoration, EditorView } from "@codemirror/view";
import { markupSpans, pairsIn, readingFields } from "./formatted";
import { callAt, elementsFacet, type ElementInfo } from "./elements";
import { INCLUSION, inclusionsOf, includedFinds, rawLine, type IncludedFind, type InclusionView } from "./inclusions";

export type Query = { text: string; matchCase: boolean; wholeWords: boolean };

// ---- Text as it reads ----

/** Stands where a match can't run across (a chip, a bracket, an Inclusion). Never searched for. */
const BREAK = "\u0000";

/** A stretch of the text as it reads (`at`, `len`) and the stretch of the text it stands for. */
type Seg = { at: number; len: number; from: number; to: number };

/** Text as it reads, mapped back to the text as typed. */
export type Reading = { text: string; segs: Seg[] };

type Cut = { from: number; to: number } & ({ kind: "hide" | "break" } | { kind: "glyph"; glyph: string });

const PLAIN = 0;
const HIDE = 1;
const STOP = 2;
const GLYPH = 3;

/**
 * Marks each character of a text as plain, hidden, a stop or part of a glyph, then reads it in
 * one pass: no sorting and few objects, which matters on every key in a novel.
 */
class Marks {
  readonly mark: Uint8Array;
  readonly glyphs = new Map<number, { to: number; glyph: string }>();

  constructor(readonly text: string) {
    this.mark = new Uint8Array(text.length);
  }

  set(from: number, to: number, kind: typeof HIDE | typeof STOP) {
    this.mark.fill(kind, from, Math.min(to, this.text.length));
  }

  glyph(from: number, to: number, glyph: string) {
    this.mark.fill(GLYPH, from, Math.min(to, this.text.length));
    this.glyphs.set(from, { to, glyph });
  }

  read(): Reading {
    const { text, mark, glyphs } = this;
    const n = text.length;
    const pieces: string[] = [];
    const segs: Seg[] = [];
    let at = 0;
    let i = 0;
    while (i < n) {
      const kind = mark[i];
      let j = i + 1;
      if (kind === GLYPH) {
        const g = glyphs.get(i);
        if (g) {
          segs.push({ at, len: g.glyph.length, from: i, to: g.to });
          pieces.push(g.glyph);
          at += g.glyph.length;
          i = g.to;
          continue;
        }
        while (j < n && mark[j] === GLYPH && !glyphs.has(j)) j++; // the rest of one taken already
        i = j;
        continue;
      }
      while (j < n && mark[j] === kind) j++;
      if (kind === PLAIN) {
        segs.push({ at, len: j - i, from: i, to: j });
        pieces.push(text.slice(i, j));
        at += j - i;
      } else if (kind === STOP) {
        segs.push({ at, len: 1, from: i, to: j });
        pieces.push(BREAK);
        at += 1;
      }
      i = j;
    }
    return { text: pieces.join(""), segs };
  }
}

/** `text` as it reads, given what's hidden, what shows as a glyph, and where matches stop. */
export function readingOf(text: string, cuts: Cut[]): Reading {
  const marks = new Marks(text);
  for (const cut of cuts) {
    if (cut.kind === "glyph") marks.glyph(cut.from, cut.to, cut.glyph);
    else marks.set(cut.from, cut.to, cut.kind === "hide" ? HIDE : STOP);
  }
  return marks.read();
}

/** The segment holding the character at `i` as it reads. */
function segAt(r: Reading, i: number): Seg {
  let lo = 0;
  let hi = r.segs.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (r.segs[mid].at <= i) lo = mid;
    else hi = mid - 1;
  }
  return r.segs[lo];
}

const isPlain = (s: Seg) => s.len === s.to - s.from;

/** Where the character at `i` (as it reads) starts in the text as typed. */
export function startOf(r: Reading, i: number): number {
  if (r.segs.length === 0) return 0;
  const s = segAt(r, Math.min(i, r.text.length - 1));
  if (i >= r.text.length) return s.to;
  return isPlain(s) ? s.from + (i - s.at) : s.from;
}

/** Where the text up to `i` (as it reads) ends in the text as typed: just after character `i - 1`. */
export function endOf(r: Reading, i: number): number {
  if (i <= 0) return startOf(r, 0);
  const s = segAt(r, i - 1);
  return isPlain(s) ? s.from + (i - s.at) : s.to;
}

/** Included text as its block shows it: headings without their `=` marks (inclusions.ts `readable`). */
export function includedReading(text: string): Reading {
  const cuts: Cut[] = [];
  let at = 0;
  for (const line of text.split("\n")) {
    const heading = /^(=+\s+)/.exec(line);
    if (heading) cuts.push({ from: at, to: at + heading[1].length, kind: "hide" });
    at += line.length + 1;
  }
  return readingOf(text, cuts);
}

/** The editor's own text as the view shows it: as it reads in the Formatted view, as typed in the Raw view. */
export function readingOfState(state: EditorState): Reading {
  const doc = state.doc.toString();
  const marks = new Marks(doc);
  // An Inclusion's character: its text is searched apart.
  for (let i = doc.indexOf(INCLUSION); i !== -1; i = doc.indexOf(INCLUSION, i + 1)) marks.set(i, i + 1, STOP);
  const spans = markupSpans(state);
  if (spans) {
    const elements = state.facet(elementsFacet);
    for (const span of spans) {
      switch (span.kind) {
        case "heading":
          marks.set(span.mark[0], span.mark[1], HIDE);
          break;
        case "strong":
        case "emph":
        case "raw":
          marks.set(span.open[0], span.open[1], HIDE);
          marks.set(span.close[0], span.close[1], HIDE);
          break;
        case "escape":
          marks.set(span.from, span.from + 1, HIDE);
          break;
        case "glyph":
          marks.glyph(span.from, span.to, span.glyph);
          break;
        case "code": {
          // A Story element shows as a chip: a match stops there.
          const call = callAt(state.doc, span.from, span.to, elements);
          if (call) marks.set(call.from, call.close ? call.head[1] - 1 : call.head[1], STOP);
          break;
        }
      }
    }
    // Every Story element's `[` and `]`: hidden, and where its body starts and ends.
    for (const p of pairsIn(state)) {
      marks.set(p.open, p.open + 1, STOP);
      marks.set(p.close, p.close + 1, STOP);
    }
  }
  return marks.read();
}

/**
 * Any Work's own text as it reads in the Formatted view, whichever view it's in or whether it's
 * open at all (Find in Vault). `elements`: the Style library's Story elements, whose calls show
 * as chips and stop a match, as in the editor.
 */
export function readingOfText(doc: string | Text, elements: ElementInfo[]): Reading {
  const state = EditorState.create({ doc, extensions: [readingFields, elementsFacet.of(elements)] });
  return readingOfState(state);
}

// ---- Matching ----

const QUOTES = /[‘’‚‛′]/g;
const DOUBLE_QUOTES = /[“”„‟″]/g;

/** `s` folded for comparing: quotes made straight, and lower case unless case matters. Same length as `s`. */
export function fold(s: string, matchCase: boolean): string {
  let out = s;
  if (!matchCase) {
    const lower = s.toLowerCase();
    // A few characters grow when lowered (İ): those are compared as they are, keeping places.
    out = lower.length === s.length ? lower : [...s].map((c) => (c.toLowerCase().length === c.length ? c.toLowerCase() : c)).join("");
  }
  return out.replace(QUOTES, "'").replace(DOUBLE_QUOTES, '"');
}

const isWordChar = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}]/u.test(c);

/** Where `query` matches in `text`: from..to pairs, in order, not overlapping. */
export function matchesIn(text: string, query: Query): [number, number][] {
  const needle = fold(query.text.split(BREAK).join(""), query.matchCase);
  if (!needle) return [];
  const hay = fold(text, query.matchCase);
  const found: [number, number][] = [];
  for (let i = hay.indexOf(needle); i !== -1; i = hay.indexOf(needle, i + 1)) {
    const end = i + needle.length;
    if (query.wholeWords && (isWordChar(text[i - 1]) && isWordChar(text[i]) || isWordChar(text[end - 1]) && isWordChar(text[end]))) continue;
    found.push([i, end]);
    i = end - 1; // not overlapping
  }
  return found;
}

/**
 * A match: in the editor's own text (`from`..`to`, and `read`, where it is as it reads), or in
 * an Inclusion's text (`included`; `from` is the Inclusion's character).
 */
export type FindMatch = {
  from: number;
  to: number;
  read: [number, number] | null;
  included: { at: number; from: number; to: number; label: string; work: string | null; starts: number } | null;
};

/** What an Inclusion's block is searched as: its text as the block shows it, or its Raw view line. */
function includedHaystack(v: InclusionView, raw: boolean): Reading {
  return raw ? readingOf(rawLine(v), []) : includedReading(v.text);
}

/** Every match in a state, own text and included text, in the order they read. */
export function findAll(state: EditorState, reading: Reading, query: Query, scope: { from: number; to: number } | null): FindMatch[] {
  const inScope = (from: number, to: number) => !scope || (from >= scope.from && to <= scope.to);
  const out: FindMatch[] = [];
  for (const [a, b] of matchesIn(reading.text, query)) {
    const from = startOf(reading, a);
    const to = endOf(reading, b);
    if (inScope(from, to)) out.push({ from, to, read: [a, b], included: null });
  }
  const raw = markupSpans(state) === null;
  for (const v of inclusionsOf(state)) {
    if (v.missing && !raw) continue;
    if (!inScope(v.at, v.at + 1)) continue;
    const hay = includedHaystack(v, raw);
    for (const [a, b] of matchesIn(hay.text, query)) {
      out.push({ from: v.at, to: v.at + 1, read: null, included: { at: v.at, from: startOf(hay, a), to: endOf(hay, b), label: v.label.split(": ")[0] || v.label, work: v.work, starts: v.starts } });
    }
  }
  return out.sort((x, y) => x.from - y.from || (x.included?.from ?? 0) - (y.included?.from ?? 0));
}

// ---- Replacing ----

/** `replacement` with the capitals of `matched`: all capitals, a capital first, or as typed. */
export function withCase(replacement: string, matched: string): string {
  const letters = [...matched].filter((c) => /\p{L}/u.test(c));
  if (letters.length === 0) return replacement;
  const upper = (c: string) => c !== c.toLowerCase() && c === c.toUpperCase();
  const lower = (c: string) => c !== c.toUpperCase() && c === c.toLowerCase();
  if (letters.length >= 2 && letters.every(upper)) return replacement.toUpperCase();
  if (upper(letters[0]) && letters.slice(1).every(lower)) {
    const first = [...replacement].findIndex((c) => /\p{L}/u.test(c));
    if (first === -1) return replacement;
    const chars = [...replacement];
    chars[first] = chars[first].toUpperCase();
    return chars.join("");
  }
  return replacement;
}

/** Words, runs of spaces, and single other characters. */
function tokens(s: string): string[] {
  return s.match(/[\p{L}\p{N}]+|\s+|[^\p{L}\p{N}\s]/gu) ?? [];
}

/** Edits (offsets in `old`) that turn `old` into `next`, rewriting only whole words that differ. */
export function wordEdits(old: string, next: string): { from: number; to: number; text: string }[] {
  const a = tokens(old);
  const b = tokens(next);
  // Too long to compare word by word (a pasted chapter): one edit.
  if (a.length * b.length > 250_000) return old === next ? [] : [{ from: 0, to: old.length, text: next }];
  // The longest run of tokens the two share, then the edits between.
  const lcs: number[][] = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) lcs[i][j] = a[i] === b[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
  }
  const edits: { from: number; to: number; text: string }[] = [];
  let i = 0;
  let j = 0;
  let at = 0;
  let open: { from: number; to: number; text: string } | null = null;
  const flush = () => {
    if (open && (open.to > open.from || open.text)) edits.push(open);
    open = null;
  };
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      flush();
      at += a[i].length;
      i++;
      j++;
    } else if (j < b.length && (i === a.length || lcs[i][j + 1] >= lcs[i + 1][j])) {
      open ??= { from: at, to: at, text: "" };
      open.text += b[j++];
    } else {
      open ??= { from: at, to: at, text: "" };
      at += a[i].length;
      open.to = at;
      i++;
    }
  }
  flush();
  return edits;
}

/** The changes to the editor's text that replace one own-text match, as a reader would expect. */
export function replacementChanges(reading: Reading, match: FindMatch, replacement: string, matchCase: boolean): { from: number; to: number; insert: string }[] {
  if (!match.read) return [];
  const [ra, rb] = match.read;
  const matched = reading.text.slice(ra, rb);
  const next = matchCase ? replacement : withCase(replacement, matched);
  return wordEdits(matched, next).map((e) => {
    // An insertion goes right after what's before it (inside markup that ends there).
    const from = e.from === e.to && e.from > 0 ? endOf(reading, ra + e.from) : startOf(reading, ra + e.from);
    const to = e.to === e.from ? from : endOf(reading, ra + e.to);
    return { from, to, insert: e.text };
  });
}

// ---- In the editor ----

type FindState = {
  query: Query | null;
  /** "In selection": only matches inside this, followed through edits. */
  scope: { from: number; to: number } | null;
  /** The own text as it reads, for replacing. */
  reading: Reading | null;
  matches: FindMatch[];
  /** The current match (an index into `matches`), -1 if there are none. */
  current: number;
};

const EMPTY: FindState = { query: null, scope: null, reading: null, matches: [], current: -1 };

/** Starts (or, with null, ends) finding in an editor. */
export const setFind = StateEffect.define<{ query: Query | null; scope: { from: number; to: number } | null }>();
/** Makes this match (its index) the current one. */
export const setCurrent = StateEffect.define<number>();

/** The first match at or after `pos`, wrapping round to the first. */
function firstFrom(matches: FindMatch[], pos: number, inner = 0): number {
  if (matches.length === 0) return -1;
  const i = matches.findIndex((m) => m.from > pos || (m.from === pos && (m.included?.from ?? 0) >= inner));
  return i === -1 ? 0 : i;
}

export const findField = StateField.define<FindState>({
  create: () => EMPTY,
  update(value, tr) {
    let { query, scope, current } = value;
    let fresh = false;
    let wanted: number | null = null;
    for (const e of tr.effects) {
      if (e.is(setFind)) {
        ({ query, scope } = e.value);
        fresh = true;
      } else if (e.is(setCurrent)) {
        wanted = e.value;
      }
    }
    if (!query) return value.query ? EMPTY : value;
    if (tr.docChanged && scope && !fresh) scope = { from: tr.changes.mapPos(scope.from, 1), to: tr.changes.mapPos(scope.to, -1) };
    const redo = fresh || tr.docChanged || tr.reconfigured || inclusionsOf(tr.startState) !== inclusionsOf(tr.state);
    if (!redo) {
      if (wanted === null || wanted === current) return value;
      return { ...value, current: value.matches.length ? ((wanted % value.matches.length) + value.matches.length) % value.matches.length : -1 };
    }
    const reading = readingOfState(tr.state);
    const matches = findAll(tr.state, reading, query, scope);
    if (wanted !== null) {
      current = matches.length ? ((wanted % matches.length) + matches.length) % matches.length : -1;
    } else if (fresh) {
      // A new search starts at the cursor.
      current = firstFrom(matches, tr.state.selection.main.from);
    } else {
      // The current match stays current while it's there; otherwise the next one after it.
      const was = value.matches[value.current];
      current = was ? firstFrom(matches, tr.changes.mapPos(was.from, 1), was.included?.from ?? 0) : firstFrom(matches, 0);
    }
    return { query, scope, reading, matches, current };
  },
});

/** The matches in the editor's own text, highlighted (the current one more). */
const highlights = EditorView.decorations.compute([findField], (state) => {
  const { matches, current } = state.field(findField);
  const ranges = matches.flatMap((m, i) => (m.included || m.to <= m.from ? [] : [(i === current ? currentMark : matchMark).range(m.from, m.to)]));
  return Decoration.set(ranges, true);
});
const matchMark = Decoration.mark({ class: "cm-find-match" });
const currentMark = Decoration.mark({ class: "cm-find-match cm-find-current" });

/** The matches inside each Inclusion, for its block to highlight. */
const inIncluded = includedFinds.compute([findField], (state) => {
  const { matches, current } = state.field(findField);
  const out = new Map<number, IncludedFind>();
  matches.forEach((m, i) => {
    if (!m.included) return;
    const entry = out.get(m.included.at) ?? { ranges: [], current: null };
    entry.ranges.push([m.included.from, m.included.to]);
    if (i === current) entry.current = [m.included.from, m.included.to];
    out.set(m.included.at, entry);
  });
  return out;
});

/** Told when the matches or the current one change in some editor (the bar's count follows). */
const listeners = new Set<(view: EditorView) => void>();
export function onFindChange(listener: (view: EditorView) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Find and replace in an editor. */
export function findExtension(): Extension {
  return [
    findField,
    highlights,
    inIncluded,
    EditorView.updateListener.of((update) => {
      if (update.startState.field(findField) !== update.state.field(findField)) for (const l of listeners) l(update.view);
    }),
  ];
}

/** The state of finding in an editor. */
export function findState(state: EditorState): FindState {
  return state.field(findField, false) ?? EMPTY;
}

