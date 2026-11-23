// A Work's text as it reads, for the Mods that check it (the Dialogue checker, the Spellchecker):
// emphasis and strong are seen through, `--`, `---` and `...` read as their glyphs, and
// comments, code, raw, math, labels, links and Inclusions are hard breaks that are never checked.
// Every character keeps its place in the text as typed, so a Suggestion edits the real source.
//
// The two Mods differ in what else they read. The Dialogue checker skips headings and Story
// elements whole, text and all, since neither is dialogue. The Spellchecker reads both (CONTEXT.md):
// a heading without its `=`s, and a Story element's text without its `#name(…)[` and `]`, which
// are code; a word never runs across either edge.
//
// A Footnote (CONTEXT.md) is a passage of its own: the sentence it sits in reads as if it weren't
// there (so `"Wait,"#footnote[…] she said.` is still a Dialogue tag), and its own text is read as
// a paragraph by itself (`passagesOf`). A word never runs across its edge: where one was, the
// paragraph has a seam.

import { parseMarkup } from "./markup";

/** Where nothing is read: a comment, code, a Story element, an Inclusion. Never part of a word or a sentence. */
export const BREAK = "\u0000";
const INCLUSION = "￼"; // inclusions.ts
/** Content blocks that are only formatting, read through like emphasis: `#emph[…]`. */
const TRANSPARENT = new Set(["emph", "strong", "smallcaps", "underline", "strike", "overline", "highlight", "sub", "super", "text"]);

/** A paragraph as it reads, each character mapped back to the stretch of source it stands for. */
export type Para = {
  text: string;
  /** Source start and end of each character as it reads. */
  from: number[];
  to: number[];
  /** Where the paragraph is in the source. */
  start: number;
  end: number;
  /** Where a Footnote was taken out (indexes into `text`): no word runs across one. */
  seams: number[];
};

/** What's read besides the prose: headings (without their `=`s), and the text inside Story elements. */
export type Reading = { headings?: boolean; elements?: boolean };

const PLAIN = 0;
const HIDE = 1;
const STOP = 2;
const GLYPH = 3;

export function readPara(source: string, start: number, end: number, reading: Reading = {}): Para {
  const src = source.slice(start, end);
  const n = src.length;
  const kind = new Uint8Array(n);
  const glyphs = new Map<number, { to: number; glyph: string }>();
  const set = (from: number, to: number, k: number) => kind.fill(k, Math.max(0, from - start), Math.min(n, to - start));
  /** Open content blocks: whether each is read through. */
  const blocks: boolean[] = [];
  let stopFrom = -1;
  const notes: { from: number; to: number }[] = [];
  for (const span of parseMarkup(src, start)) {
    switch (span.kind) {
      case "footnote":
        notes.push(span);
        break;
      case "heading":
        if (reading.headings) set(span.mark[0], span.mark[1], HIDE);
        else set(span.from, span.to, STOP);
        break;
      case "strong":
      case "emph":
        set(span.open[0], span.open[1], HIDE);
        set(span.close[0], span.close[1], HIDE);
        break;
      case "escape": set(span.from, span.from + 1, HIDE); break;
      case "linebreak":
      case "list": set(span.from, span.to, HIDE); break;
      case "glyph":
        set(span.from, span.to, GLYPH);
        glyphs.set(span.from - start, { to: span.to - start, glyph: span.glyph });
        break;
      case "code": {
        const code = source.slice(span.from, span.to);
        if (code.endsWith("[")) {
          const name = /^#([\p{L}_][\p{L}\p{N}_-]*)/u.exec(code)?.[1] ?? "";
          const through = TRANSPARENT.has(name) && blocks.every((b) => b);
          blocks.push(through);
          if (through) set(span.from, span.to, HIDE);
          else {
            if (stopFrom === -1 && !reading.elements) stopFrom = span.from;
            set(span.from, span.to, STOP);
          }
        } else if (code === "]" && blocks.length > 0) {
          const through = blocks.pop();
          if (through) set(span.from, span.to, HIDE);
          else if (reading.elements) set(span.from, span.to, STOP);
          else if (blocks.every((b) => b) && stopFrom !== -1) {
            set(stopFrom, span.to, STOP); // the whole Story element, its text too
            stopFrom = -1;
          }
        } else set(span.from, span.to, STOP);
        break;
      }
      default: set(span.from, span.to, STOP); // raw, comments, math, labels, links
    }
  }
  if (stopFrom !== -1) set(stopFrom, end, STOP);
  for (let i = src.indexOf(INCLUSION); i !== -1; i = src.indexOf(INCLUSION, i + 1)) kind[i] = STOP;
  // Footnotes, whatever is inside them: read apart, as if they weren't here.
  const noteAt = new Set<number>();
  for (const f of notes) {
    set(f.from, f.to, HIDE);
    noteAt.add(f.from - start);
  }

  let text = "";
  const from: number[] = [];
  const to: number[] = [];
  const seams: number[] = [];
  for (let i = 0; i < n;) {
    const k = kind[i];
    if (k === PLAIN) {
      text += src[i];
      from.push(start + i);
      to.push(start + i + 1);
      i++;
    } else if (k === GLYPH) {
      const g = glyphs.get(i);
      if (!g) { i++; continue; }
      text += g.glyph;
      from.push(start + i);
      to.push(start + g.to);
      i = g.to;
    } else {
      let j = i + 1;
      while (j < n && kind[j] === k) j++;
      if (k === HIDE && noteAt.size) {
        for (let x = i; x < j; x++) if (noteAt.has(x)) { seams.push(text.length); break; }
      }
      if (k === STOP) {
        text += BREAK;
        from.push(start + i);
        to.push(start + j);
      }
      i = j;
    }
  }
  return { text, from, to, start, end, seams };
}

/** A stretch of a Work's text read as a paragraph: a paragraph, or the text of a Footnote (`note`). */
export type Passage = { start: number; end: number; note: boolean };

/** The Footnotes' texts in a stretch of source (nested ones too), as passages, in order. */
export function notesOf(source: string, start: number, end: number): Passage[] {
  if (!source.slice(start, end).includes("#footnote[")) return [];
  return parseMarkup(source.slice(start, end), start)
    .filter((s): s is Extract<typeof s, { kind: "footnote" }> => s.kind === "footnote")
    .sort((a, b) => a.from - b.from)
    .map((f) => ({ start: f.open[1], end: f.close[0], note: true }));
}

/** Every passage of a Work's text: each paragraph, followed by the text of each Footnote in it. */
export function passagesOf(source: string): Passage[] {
  const out: Passage[] = [];
  for (const p of paragraphsOf(source)) {
    out.push({ ...p, note: false });
    out.push(...notesOf(source, p.start, p.end));
  }
  return out;
}

/** Splits a word (`at`..`end` in a paragraph's text) where a Footnote was taken out. */
export function splitAtSeams(p: Para, at: number, end: number): [number, number][] {
  const cuts = p.seams.filter((s) => s > at && s < end);
  if (cuts.length === 0) return [[at, end]];
  const out: [number, number][] = [];
  let from = at;
  for (const c of cuts) {
    out.push([from, c]);
    from = c;
  }
  out.push([from, end]);
  return out;
}

// ---- Suggestions, as edits to a paragraph as it reads ----

/** An edit to a paragraph as it reads (indexes into `text`); `shows`: what the preview shows for it, if not `insert`. */
export type ReadEdit = { at: number; to: number; insert: string; shows?: string };

/** The same edit to the text as typed. */
export function toSource(p: Para, e: ReadEdit): { from: number; to: number; insert: string } {
  if (e.at === e.to) {
    const from = e.at >= p.text.length ? (p.to[p.text.length - 1] ?? p.start) : p.from[e.at];
    return { from, to: from, insert: e.insert };
  }
  return { from: p.from[e.at], to: p.to[e.to - 1], insert: e.insert };
}

/** What the text around the edits reads like after them: `"Hi," she said.` A Suggestion's label. */
export function preview(p: Para, edits: ReadEdit[]): string {
  const lo = Math.min(...edits.map((e) => e.at));
  const hi = Math.max(...edits.map((e) => e.to));
  let from = lo;
  let words = 0;
  while (from > 0 && words < 4) {
    from--;
    if (/\s/.test(p.text[from]) && !/\s/.test(p.text[from + 1] ?? "")) words++;
    if (p.text[from] === BREAK || (p.text[from] === "\n" && words > 0)) { from++; break; }
  }
  let to = hi;
  words = 0;
  while (to < p.text.length && words < 4) {
    if (/\s/.test(p.text[to]) && !/\s/.test(p.text[to - 1] ?? "")) words++;
    if (p.text[to] === BREAK || (words > 2 && /[.!?]/.test(p.text[to - 1]))) break;
    to++;
  }
  let out = "";
  let at = from;
  for (const e of [...edits].sort((a, b) => a.at - b.at)) {
    out += p.text.slice(at, e.at) + (e.shows ?? e.insert);
    at = e.to;
  }
  out += p.text.slice(at, to);
  const text = out.replace(/\n\n/g, " ¶ ").replace(/\s+/g, " ").trim();
  return (from > 0 ? "…" : "") + text + (to < p.text.length ? "…" : "");
}

/** The paragraphs of a Work's text: split at blank lines, as the markup is. */
export function paragraphsOf(source: string): { start: number; end: number }[] {
  const out: { start: number; end: number }[] = [];
  const blank = /\n[ \t]*\n\s*/g;
  let at = 0;
  for (let m = blank.exec(source); m; m = blank.exec(source)) {
    if (m.index > at) out.push({ start: at, end: m.index });
    at = m.index + m[0].length;
  }
  if (at < source.length) out.push({ start: at, end: source.length });
  return out;
}
