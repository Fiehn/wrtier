// A Work's text as it reads, for the Mods that check it (the Dialogue checker, the Spellchecker):
// emphasis and strong are seen through, `--`, `---` and `...` read as their glyphs, and
// comments, code, raw, math, labels, links and Inclusions are hard breaks that are never checked.
// Every character keeps its place in the text as typed, so a Suggestion edits the real source.
//
// The two Mods differ in what else they read. The Dialogue checker skips headings and Story
// elements whole, text and all, since neither is dialogue. The Spellchecker reads both (CONTEXT.md):
// a heading without its `=`s, and a Story element's text without its `#name(…)[` and `]`, which
// are code; a word never runs across either edge.

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
  for (const span of parseMarkup(src, start)) {
    switch (span.kind) {
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

  let text = "";
  const from: number[] = [];
  const to: number[] = [];
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
      if (k === STOP) {
        text += BREAK;
        from.push(start + i);
        to.push(start + j);
      }
      i = j;
    }
  }
  return { text, from, to, start, end };
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
