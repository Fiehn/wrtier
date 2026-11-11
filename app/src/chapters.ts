// The Chapters of a Work's text, for the Chapter list Mod: where each one is, how many words it
// has, and how to move one elsewhere as Moves, so its History and Comments go with it.
//
// A Chapter starts at a line beginning "= " (as the History engine reads them) and runs to the
// next Chapter, or to the next Part (a line starting `#part`), which stays where it is. Text
// before the first of them isn't in any Chapter.
//
// An Inclusion (ADR 0015) on a line of its own is an entry too: one Chapter of another Work
// counts as a Chapter here, and a whole Work as one section under its title. Like a Chapter, it
// runs to the next entry, so text written after it goes with it when it's moved.

import { wordsIn } from "./counts";

export type Item = {
  kind: "chapter" | "part";
  title: string;
  /** Where its line starts. */
  from: number;
  /** Where the next Chapter or Part starts, or the end of the text. */
  to: number;
  /** Words in it, not counting its heading line (an Inclusion's words included). */
  words: number;
  /** It's an Inclusion: a Chapter of another Work, or a whole Work. */
  included?: boolean;
};

/** An Inclusion as the Chapter list needs it: where its character is, and what it shows. */
export type IncludedEntry = { at: number; title: string; words: number };

/** One Move: the text at `from`..`to` goes to `at` (positions before the Move). */
export type MoveOp = { from: number; to: number; at: number };

const PART = /^#part\s*[[(]/;

/** A heading's title as shown in the list: its markup taken off. */
function plainTitle(raw: string): string {
  return raw.replace(/[*_`]/g, "").replace(/\\(.)/g, "$1").trim();
}

/** A Part's title: what's in its brackets on that line, or "Part". */
function partTitle(line: string): string {
  const inside = /\[(.*)\]/.exec(line)?.[1] ?? /\("?([^)"]*)"?\)/.exec(line)?.[1] ?? "";
  return plainTitle(inside) || "Part";
}

/** The Chapters and Parts of a text, in order. */
export function chaptersIn(text: string, included: IncludedEntry[] = []): Item[] {
  const items: (Omit<Item, "to" | "words"> & { more: number })[] = [];
  const inclusions = new Map(included.map((e) => [e.at, e]));
  let start = 0;
  while (start <= text.length) {
    let end = text.indexOf("\n", start);
    if (end === -1) end = text.length;
    const line = text.slice(start, end);
    const inclusion = line === "￼" ? inclusions.get(start) : undefined;
    if (inclusion) items.push({ kind: "chapter", title: inclusion.title, from: start, included: true, more: inclusion.words });
    else if (line.startsWith("= ")) items.push({ kind: "chapter", title: plainTitle(line.slice(2)) || "Untitled", from: start, more: 0 });
    else if (PART.test(line)) items.push({ kind: "part", title: partTitle(line), from: start, more: 0 });
    start = end + 1;
  }
  return items.map(({ more, ...item }, i) => {
    const to = items[i + 1]?.from ?? text.length;
    const lineEnd = text.indexOf("\n", item.from);
    const body = lineEnd === -1 || lineEnd >= to ? "" : text.slice(lineEnd + 1, to);
    return { ...item, to, words: wordsIn(body) + more };
  });
}

/** The Chapter or Part the position `at` is in (its index), or -1 (before the first). */
export function itemAt(items: Item[], at: number): number {
  for (let i = items.length - 1; i >= 0; i--) if (items[i].from <= at) return i;
  return -1;
}

/**
 * Moves the text `from`..`to` (whole lines: a Chapter) to `dest` (the start of a line, or the
 * end of the text), as Moves. Usually one. A text that doesn't end with a line break needs a
 * second, carrying a line break that's already there to where one is missing, so no line is
 * ever joined to another and nothing is typed. Each Move's positions are in the text as the
 * Moves before it left it. Nothing if `dest` is inside or at either edge of the text moved.
 */
export function planMove(text: string, from: number, to: number, dest: number): MoveOp[] {
  const length = text.length;
  if (dest >= from && dest <= to) return [];
  const atLineStart = (p: number) => p === 0 || text[p - 1] === "\n";
  if (text[to - 1] === "\n") {
    if (atLineStart(dest)) return [{ from, to, at: dest }];
    // To the end of a text whose last line has no break: take the break before it instead.
    if (from > 0) return [{ from: from - 1, to: to - 1, at: length }];
    return [{ from, to, at: length }, { from: length - 1, to: length, at: length - to }];
  }
  // The last lines, with no break after them, go earlier: take the break before them.
  if (dest > 0) return [{ from: from - 1, to: length, at: dest - 1 }];
  return [{ from, to: length, at: 0 }, { from: length - 1, to: length, at: length - from }];
}

/** The text after a Move. */
export function applyMove(text: string, m: MoveOp): string {
  const piece = text.slice(m.from, m.to);
  return m.at <= m.from
    ? text.slice(0, m.at) + piece + text.slice(m.at, m.from) + text.slice(m.to)
    : text.slice(0, m.from) + text.slice(m.to, m.at) + piece + text.slice(m.at);
}

/** Where a position ends up after a Move (one inside the moved text goes with it). */
export function mapThrough(pos: number, m: MoveOp): number {
  const length = m.to - m.from;
  const at = m.at <= m.from ? m.at : m.at - length;
  if (pos >= m.from && pos < m.to) return at + (pos - m.from);
  const rest = pos >= m.to ? pos - length : pos;
  return rest >= at ? rest + length : rest;
}

/**
 * Where a Chapter goes when moved up or down past its neighbour (a Chapter or a Part): the
 * start of the line it goes before. Null if it can't go that way, or `index` isn't a Chapter.
 */
export function neighbourDest(items: Item[], index: number, direction: -1 | 1, length: number): number | null {
  if (items[index]?.kind !== "chapter") return null;
  if (direction === -1) return index > 0 ? items[index - 1].from : null;
  if (index >= items.length - 1) return null;
  return items[index + 2]?.from ?? length;
}
