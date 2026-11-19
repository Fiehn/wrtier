// Footnotes (CONTEXT.md): `#footnote[…]` written in a Work's own text, Typst's own function, not
// a Story element. What the editor, the Footnotes tab and the counts need to know about them:
// where each one is, the number the editor gives it, and its text as it reads.
//
// The editor numbers them 1, 2, 3… in the order the Work reads, counting the ones in its
// Inclusions, whatever the Look does: the Page preview shows the printed number.

import type { EditorState } from "@codemirror/state";
import { parseMarkup, type Span } from "./markup";

export type FootnoteSpan = Extract<Span, { kind: "footnote" }>;

/** The Footnotes among some spans, in the order they start (an outer one before one inside it). */
export function footnoteSpans(spans: Span[]): FootnoteSpan[] {
  return spans.filter((s): s is FootnoteSpan => s.kind === "footnote").sort((a, b) => a.from - b.from);
}

/** The Footnotes written in some text, in order. */
export function footnotesInText(text: string): FootnoteSpan[] {
  return footnoteSpans(parseMarkup(text));
}

/** How many Footnotes some text has; remembered by text, since included text rarely changes. */
const counted = new Map<string, number>();
export function footnoteCount(text: string): number {
  let n = counted.get(text);
  if (n === undefined) {
    n = text.includes("#footnote[") ? footnotesInText(text).length : 0;
    if (counted.size > 200) counted.clear();
    counted.set(text, n);
  }
  return n;
}

/** Something that numbers in the sequence besides the Work's own Footnotes: an Inclusion's (at its character). */
export type Numbered = { at: number; count: number };

/**
 * The editor's number for each of the Work's own Footnotes (by where it starts), counting those
 * in `others` (Inclusions) that come before it.
 */
export function numberFootnotes(own: FootnoteSpan[], others: Numbered[] = []): Map<number, number> {
  const sorted = [...others].sort((a, b) => a.at - b.at);
  const out = new Map<number, number>();
  let n = 0;
  let k = 0;
  for (const f of own) {
    while (k < sorted.length && sorted[k].at < f.from) n += sorted[k++].count;
    out.set(f.from, ++n);
  }
  return out;
}

/** The text of the Footnote `f` as it reads, in one line: its markup taken off, Footnotes inside it as their markers. */
export function footnoteText(source: string, f: FootnoteSpan): string {
  const inner = source.slice(f.open[1], f.close[0]);
  const spans = parseMarkup(inner, f.open[1]);
  const cut: [number, number, string][] = [];
  for (const s of spans) {
    switch (s.kind) {
      case "footnote": cut.push([s.from, s.to, "*"]); break;
      case "strong": case "emph": case "raw":
        cut.push([s.open[0], s.open[1], ""], [s.close[0], s.close[1], ""]);
        break;
      case "heading": cut.push([s.mark[0], s.mark[1], ""]); break;
      case "escape": cut.push([s.from, s.from + 1, ""]); break;
      case "glyph": cut.push([s.from, s.to, s.glyph]); break;
      case "comment": cut.push([s.from, s.to, ""]); break;
      case "linebreak": cut.push([s.from, s.to, " "]); break;
    }
  }
  cut.sort((a, b) => a[0] - b[0]);
  let out = "";
  let at = f.open[1];
  for (const [from, to, put] of cut) {
    if (from < at) continue; // inside one already cut (a Footnote inside this one)
    out += source.slice(at, from) + put;
    at = to;
  }
  out += source.slice(at, f.close[0]);
  return out.replace(/\s+/g, " ").trim();
}

/** A Footnote as the Footnotes tab lists it. */
export type FootnoteEntry = { from: number; to: number; number: number; text: string };

/** The Work's own Footnotes, numbered, with their text: for the Footnotes tab. */
export function footnoteEntries(state: EditorState, spans: Span[] | null, others: Numbered[] = []): FootnoteEntry[] {
  const source = state.doc.toString();
  const own = footnoteSpans(spans ?? parseMarkup(source));
  const numbers = numberFootnotes(own, others);
  return own.map((f) => ({ from: f.from, to: f.to, number: numbers.get(f.from) ?? 0, text: footnoteText(source, f) }));
}

/** Words inside Footnotes in some text (only the outermost ones, so none is counted twice). */
export function footnoteStretches(text: string): string[] {
  if (!text.includes("#footnote[")) return [];
  const out: string[] = [];
  let end = -1;
  for (const f of footnotesInText(text)) {
    if (f.from < end) continue;
    out.push(text.slice(f.open[1], f.close[0]));
    end = f.to;
  }
  return out;
}

/** Some text without its Footnotes (a heading's, for the Chapter list). */
export function withoutFootnotes(text: string): string {
  if (!text.includes("#footnote[")) return text;
  let out = "";
  let at = 0;
  for (const f of footnotesInText(text)) {
    if (f.from < at) continue;
    out += text.slice(at, f.from);
    at = f.to;
  }
  return out + text.slice(at);
}

/** `#footnote[]` at the cursor, or around the selection, leaving the cursor in it. */
export function insertFootnote(state: EditorState): { changes: { from: number; insert: string }[]; selection: { anchor: number; head?: number } } {
  const { from, to } = state.selection.main;
  const head = "#footnote[";
  if (from === to) return { changes: [{ from, insert: `${head}]` }], selection: { anchor: from + head.length } };
  return {
    changes: [{ from, insert: head }, { from: to, insert: "]" }],
    selection: { anchor: from + head.length, head: to + head.length },
  };
}

// ---- Annotations (ADR 0020): notes the Work prints on an Inclusion's text ----

/** An Annotation printed in an Inclusion (vault/annotations.rs `NoteShown`). */
export type AnnotationShown = {
  id: string;
  /** The ID of the Work it belongs to. */
  owner: string;
  ownerLabel: string;
  /** It belongs to the Work in front: it can be changed there. */
  mine: boolean;
  /** Where its number goes in the included text (UTF-16). */
  at: number;
  /** What it covers there (UTF-16). */
  ranges: [number, number][];
  body: string;
  quote: string;
  /** The Annotations on it, printed at the end of its footnote. */
  notes: AnnotationShown[];
};

/** A note printed in an Inclusion's text: a Footnote written in it, or an Annotation (one on another at its end). */
export type IncludedNote =
  | { kind: "footnote"; at: number; span: FootnoteSpan; text: string }
  | { kind: "annotation"; at: number; note: AnnotationShown; on: AnnotationShown | null };

/** The notes printed in an Inclusion's text, in the order they print. */
export function includedNotes(text: string, annotations: AnnotationShown[] = []): IncludedNote[] {
  const out: IncludedNote[] = [];
  if (text.includes("#footnote[")) {
    for (const f of footnotesInText(text)) out.push({ kind: "footnote", at: f.from, span: f, text: footnoteText(text, f) });
  }
  const walk = (n: AnnotationShown, on: AnnotationShown | null) => {
    out.push({ kind: "annotation", at: n.at, note: n, on });
    for (const child of n.notes) walk(child, n);
  };
  for (const n of annotations) walk(n, null);
  // By place; at the same place, as given (a note on an Annotation right after it).
  return out.map((n, i) => ({ n, i })).sort((a, b) => a.n.at - b.n.at || a.i - b.i).map(({ n }) => n);
}

/** How many notes an Inclusion prints: its text's Footnotes and the Annotations on it. */
export function includedNoteCount(v: { text: string; annotations?: AnnotationShown[]; missing?: string | null }): number {
  if (v.missing) return 0;
  const count = (list: AnnotationShown[]): number => list.reduce((n, a) => n + 1 + count(a.notes), 0);
  return footnoteCount(v.text) + count(v.annotations ?? []);
}

/** An Annotation shown in some Inclusions (those on Annotations too), by its ID and its Work's ("": the Work in front's). */
export function findAnnotation(views: { annotations?: AnnotationShown[] }[], id: string, owner: string): { note: AnnotationShown; on: AnnotationShown | null } | null {
  const walk = (list: AnnotationShown[], on: AnnotationShown | null): { note: AnnotationShown; on: AnnotationShown | null } | null => {
    for (const n of list) {
      if (n.id === id && (owner ? n.owner === owner : n.mine)) return { note: n, on };
      const inner = walk(n.notes, n);
      if (inner) return inner;
    }
    return null;
  };
  for (const v of views) {
    const found = walk(v.annotations ?? [], null);
    if (found) return found;
  }
  return null;
}
