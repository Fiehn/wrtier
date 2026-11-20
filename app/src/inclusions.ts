// Inclusions (ADR 0015): another Work, or one Chapter of it, shown live where its one
// character stands in the text. The editor shows the included text read-only under a bar that
// names it, with "Go to" to open that Work; each one folds to just its bar. The character is
// one unit: the cursor goes before or after it, never inside, and deleting it removes it whole.

import { Facet, StateEffect, StateField, type EditorState, type Extension, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { countsOf } from "./counts";
import type { CommentHighlight } from "./sync";
import { isCommand } from "./keys";
import { footnotesInText, includedNoteCount, includedNotes, type AnnotationShown, type IncludedNote } from "./footnotes";

/** The character that stands for an Inclusion (change.rs `INCLUSION`). */
export const INCLUSION = "￼";

/** An Inclusion as Rust shows it (vault/inclusions.rs `InclusionView`). */
export type InclusionView = {
  /** Where its character is in the editor's text (UTF-16). */
  at: number;
  /** The included Work's ID in the Vault, to open it; null if it wasn't found. */
  work: string | null;
  label: string;
  /** The included Chapter's title, if it includes one Chapter and it was found. */
  chapter?: string | null;
  /** The included text, read-only. */
  text: string;
  /** Why it can't be shown, if it can't. */
  missing: string | null;
  language: string;
  whole: boolean;
  /** Where the included text starts in its own Work (UTF-16): "Go to" opens it there. */
  starts: number;
  /** The included Work's Comments on this text (they belong to it, and show in both); ranges in `text`. */
  comments: CommentHighlight[];
  /** A new Chapter right after the included one, not included (it was split, or one was put
   *  after it), until the Author includes it too or dismisses this. */
  split?: { title: string; at: number } | null;
  /** Styles given to this Inclusion only. */
  styles?: string[];
  /** The Annotations printed in its text (ADR 0020), by this Work and the Works it includes. */
  annotations?: AnnotationShown[];
};

/** A Chapter that can be included (vault/inclusions.rs `ChapterChoice`). */
export type ChapterChoice = { title: string; at: number };

/** The Inclusions as Rust last gave them, for the text as it is now. */
export const setInclusions = StateEffect.define<InclusionView[]>();
/** Folds or unfolds the Inclusion whose character is here. */
const toggleFold = StateEffect.define<number>();
/** Unfolds the Inclusion whose character is here (Find and replace's current match is in it). */
export const unfoldInclusion = StateEffect.define<number>();

/** Find and replace's matches in one Inclusion's text (UTF-16, in its text or its Raw view line). */
export type IncludedFind = {
  ranges: [number, number][];
  current: [number, number] | null;
  /** Annotations (their IDs) with matches in their text, and the one with the current match. */
  notes?: string[];
  currentNote?: string | null;
};

/** Find and replace's matches in each Inclusion, by its character (find.ts provides them). */
export const includedFinds = Facet.define<Map<number, IncludedFind>, Map<number, IncludedFind>>({
  combine: (values) => values[0] ?? new Map(),
  compare: (a, b) => a.size === b.size && [...a].every(([at, f]) => JSON.stringify(f) === JSON.stringify(b.get(at))),
});

/**
 * In the Reading panel (reading.ts): Inclusions are only read there, so their bar has no
 * "Styles" and no note about a split Chapter, and "Go to" reads the included Work there.
 */
export const readOnlyInclusions = Facet.define<boolean, boolean>({ combine: (values) => values.some(Boolean) });

type Shown = { views: InclusionView[]; folded: number[] };

/** Only Inclusions whose character is still where they say. */
function fitting(doc: Text, views: InclusionView[]): InclusionView[] {
  return views.filter((v) => v.at < doc.length && doc.sliceString(v.at, v.at + 1) === INCLUSION);
}

const shownField = StateField.define<Shown>({
  create: () => ({ views: [], folded: [] }),
  update(value, tr) {
    let { views, folded } = value;
    if (tr.docChanged) {
      // Typing elsewhere moves them along; one whose character was deleted goes.
      views = fitting(tr.newDoc, views.map((v) => ({ ...v, at: tr.changes.mapPos(v.at, 1) })));
      folded = folded.map((at) => tr.changes.mapPos(at, 1));
    }
    for (const e of tr.effects) {
      if (e.is(setInclusions)) views = fitting(tr.newDoc, e.value);
      if (e.is(toggleFold)) folded = folded.includes(e.value) ? folded.filter((at) => at !== e.value) : [...folded, e.value];
      if (e.is(unfoldInclusion) && folded.includes(e.value)) folded = folded.filter((at) => at !== e.value);
    }
    return views === value.views && folded === value.folded ? value : { views, folded };
  },
});

class IncludedText extends WidgetType {
  /** `base`: how many notes print before this Inclusion's (its first is numbered `base + 1`). */
  constructor(readonly v: InclusionView, readonly folded: boolean, readonly finds: IncludedFind | null, readonly base: number) {
    super();
  }

  eq(other: IncludedText) {
    const a = this.v;
    const b = other.v;
    return this.folded === other.folded && this.base === other.base && JSON.stringify(this.finds) === JSON.stringify(other.finds) && a.label === b.label && a.text === b.text && a.missing === b.missing && a.work === b.work && a.at === b.at
      && JSON.stringify(a.comments) === JSON.stringify(b.comments) && JSON.stringify(a.split) === JSON.stringify(b.split) && JSON.stringify(a.styles) === JSON.stringify(b.styles)
      && JSON.stringify(a.annotations) === JSON.stringify(b.annotations);
  }

  toDOM(view: EditorView) {
    const v = this.v;
    const readOnly = view.state.facet(readOnlyInclusions);
    const box = document.createElement("div");
    box.className = "cm-inclusion" + (v.missing ? " missing" : "") + (this.folded ? " folded" : "");
    box.contentEditable = "false";
    box.dataset.at = String(v.at);
    // Focusable, so pressing the mouse in it moves the focus here rather than into the editor:
    // the editor then leaves the selection alone, and the included text can be selected (to
    // copy it, or comment on it).
    box.tabIndex = -1;
    // The editor's keys don't reach here (it isn't focused): Add Comment's does, on the selection.
    box.addEventListener("keydown", (e) => {
      // Insert footnote's key, on included text: an Annotation (ADR 0020).
      const name = isCommand(e, "addComment") ? "wrtier-add-comment" : isCommand(e, "footnote") ? "wrtier-annotate" : null;
      if (!name) return;
      e.preventDefault();
      box.dispatchEvent(new CustomEvent(name, { bubbles: true }));
    });
    const bar = document.createElement("div");
    bar.className = "cm-inclusion-bar";
    const fold = document.createElement("button");
    fold.type = "button";
    fold.className = "cm-inclusion-fold";
    fold.textContent = this.folded ? "▸" : "▾";
    fold.title = this.folded ? "Show the included text" : "Fold to this line";
    fold.addEventListener("mousedown", (e) => e.preventDefault());
    fold.addEventListener("click", () => view.dispatch({ effects: toggleFold.of(v.at) }));
    const label = document.createElement("span");
    label.className = "cm-inclusion-label";
    label.textContent = v.label;
    label.title = v.whole ? "Included Work: shown as it is now, read-only" : "Included Chapter: shown as it is now, read-only";
    bar.append(fold, label);
    if (!v.missing) {
      const words = document.createElement("span");
      words.className = "cm-inclusion-words";
      const n = countsOf(v.text).words;
      words.textContent = `${n.toLocaleString()} ${n === 1 ? "word" : "words"}`;
      bar.append(words);
    }
    // Styles for this Inclusion only (Courier for one narrator…), picked from the library.
    const styles = document.createElement("button");
    styles.type = "button";
    styles.className = "cm-inclusion-styles";
    styles.textContent = v.styles?.length ? `Styles (${v.styles.length})` : "Styles";
    styles.title = v.styles?.length ? `Its own Styles: ${v.styles.join(", ")}` : "Give it Styles of its own, on top of the Work's Look";
    styles.addEventListener("mousedown", (e) => e.preventDefault());
    styles.addEventListener("click", (e) => {
      box.dispatchEvent(new CustomEvent("wrtier-inclusion-styles", { bubbles: true, detail: { at: v.at, styles: v.styles ?? [], x: e.clientX, y: e.clientY } }));
    });
    if (!readOnly) bar.append(styles);
    if (v.work) {
      const go = document.createElement("button");
      go.type = "button";
      go.className = "cm-inclusion-go";
      go.textContent = "Go to";
      go.title = readOnly ? `Read ${v.work} here` : `Open ${v.work} to change this text`;
      go.addEventListener("mousedown", (e) => e.preventDefault());
      go.addEventListener("click", () => {
        box.dispatchEvent(new CustomEvent("wrtier-open-work", { bubbles: true, detail: { work: v.work, at: v.starts } }));
      });
      bar.append(go);
    }
    box.append(bar);
    if (v.split && !readOnly) box.append(this.splitNote(v, v.split));
    if (v.missing) {
      const why = document.createElement("p");
      why.className = "cm-inclusion-missing";
      why.textContent = v.missing;
      box.append(why);
    } else if (!this.folded) {
      const text = document.createElement("div");
      text.className = "cm-inclusion-text";
      if (v.language) text.lang = v.language;
      text.append(...readable(v.text, v.comments, this.finds, { base: this.base, notes: includedNotes(v.text, v.annotations) }));
      // A highlight clicked: its Comments, in the Comments panel. An Annotation's number: it, to
      // change (this Work's own) or read.
      text.addEventListener("click", (e) => {
        const mark = (e.target as Element).closest<HTMLElement>("[data-annotation]");
        if (mark) {
          box.dispatchEvent(new CustomEvent("wrtier-annotation", { bubbles: true, detail: { at: v.at, id: mark.dataset.annotation, owner: mark.dataset.owner } }));
          return;
        }
        const ids: string[] = [];
        for (let el = e.target as Element | null; el && el !== text; el = el.parentElement) {
          if (el instanceof HTMLElement && el.dataset.comment) ids.push(el.dataset.comment);
        }
        if (ids.length > 0) box.dispatchEvent(new CustomEvent("wrtier-open-comments", { bubbles: true, detail: ids }));
      });
      box.append(text);
    }
    // The space around the box is the wrapper's padding, never a margin: the editor measures a
    // block without its margins, and every line below would then be placed (numbered, clicked)
    // that much too high.
    const block = document.createElement("div");
    block.className = "cm-inclusion-block";
    block.contentEditable = "false";
    block.append(box);
    return block;
  }

  /** "Zampanò has a new Chapter after this one, not included", with what to do about it. */
  private splitNote(v: InclusionView, split: { title: string; at: number }): HTMLElement {
    const note = document.createElement("div");
    note.className = "cm-inclusion-split";
    const text = document.createElement("span");
    text.textContent = `${v.label.split(": ")[0]} has a new Chapter after this one, not included: “${split.title}”.`;
    note.append(text);
    const act = (label: string, include: boolean) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.addEventListener("mousedown", (e) => e.preventDefault());
      b.addEventListener("click", () => {
        note.dispatchEvent(new CustomEvent("wrtier-split", { bubbles: true, detail: { at: v.at, work: v.work, chapter: include ? split.at : null } }));
      });
      note.append(b);
    };
    if (v.work) act("Include it here too", true);
    act("Dismiss", false);
    return note;
  }

  // Clicks inside it are its own (buttons, selecting text to copy), not the editor's.
  ignoreEvent() {
    return true;
  }
}

/** A Typst string literal. */
function typstString(s: string): string {
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n")}"`;
}

/**
 * How the Raw view writes an Inclusion: one line, `#inclusion("Zampanò", chapter: "The Navidson
 * Record")`. The names are for reading; what it includes is the Work's hidden ID. One that can't
 * be shown says why after it, as a comment.
 */
export function rawLine(v: InclusionView): string {
  const name = v.work?.split("/").pop() ?? (v.label.split(": ")[0] || "?");
  const args = [typstString(name)];
  if (v.chapter) args.push(`chapter: ${typstString(v.chapter)}`);
  if (v.styles?.length) args.push(`styles: (${v.styles.map(typstString).join(", ")}${v.styles.length === 1 ? "," : ""})`);
  const line = `#inclusion(${args.join(", ")})`;
  return v.missing ? `${line} // ${v.missing}` : line;
}

/** An Inclusion in the Raw view: its one uneditable line. */
class RawInclusion extends WidgetType {
  constructor(readonly v: InclusionView, readonly finds: IncludedFind | null) {
    super();
  }

  eq(other: RawInclusion) {
    return rawLine(this.v) === rawLine(other.v) && this.v.at === other.v.at && JSON.stringify(this.finds) === JSON.stringify(other.finds);
  }

  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-inclusion-raw" + (this.v.missing ? " missing" : "");
    span.append(...marked(rawLine(this.v), 0, findHighlights(this.finds)));
    span.title = "An Inclusion: shown in full in the Formatted view and on the pages. Change its text in the Work it includes.";
    return span;
  }

  // The cursor goes before or after it, as around any character.
  ignoreEvent() {
    return false;
  }
}

/** The Raw view is on: each Inclusion shows as its one `#inclusion(…)` line instead of the block. */
export const rawInclusions = Facet.define<boolean, boolean>({ combine: (values) => values.some(Boolean) });

/** A highlight in included text: where (UTF-16, in the included text), and a Comment's (whose), a Find match, or what an Annotation covers. */
type Highlight = { from: number; to: number } & ({ id: string; colour: string; resolved: boolean } | { find: "match" | "current" } | { annotation: string; mine: boolean });

/**
 * A note's number in included text: put in at `from` (an Annotation's, after what it covers),
 * or over `from..to` (a Footnote written there, whose text is kept, hidden, so places in the
 * text still count right).
 */
type NoteMark = { from: number; to: number; node: () => HTMLElement };

/** The notes printed in some included text, numbered from `base + 1`, as marks and highlights. */
function noteMarks(notes: { base: number; notes: IncludedNote[] }, finds: IncludedFind | null = null): { marks: NoteMark[]; highlights: Highlight[] } {
  const marks: NoteMark[] = [];
  const highlights: Highlight[] = [];
  let outer = -1; // the end of the last Footnote marked: those inside it are under its mark
  notes.notes.forEach((n, i) => {
    const number = notes.base + i + 1;
    if (n.kind === "footnote") {
      if (n.span.from < outer) return;
      outer = n.span.to;
      marks.push({ from: n.span.from, to: n.span.to, node: () => {
        const el = document.createElement("span");
        el.className = "cm-incl-note";
        el.dataset.n = String(number);
        el.title = `${number}. ${n.text}`;
        return el;
      } });
      return;
    }
    const a = n.note;
    if (!n.on) for (const [from, to] of a.ranges) highlights.push({ from, to, annotation: a.id, mine: a.mine });
    marks.push({ from: a.at, to: a.at, node: () => {
      const el = document.createElement("span");
      const found = finds?.notes?.includes(a.id) ? (finds.currentNote === a.id ? " cm-find-match cm-find-current" : " cm-find-match") : "";
      el.className = `cm-incl-note cm-incl-annotation${a.mine ? " mine" : ""}${found}`;
      el.dataset.n = String(number);
      el.dataset.annotation = a.id;
      el.dataset.owner = a.owner;
      const whose = a.mine ? "This Work's Annotation" : `${a.ownerLabel || "Another Work"}'s Annotation`;
      el.title = `${number}. ${whose}${n.on ? " on another" : ""}: ${a.body}`;
      return el;
    } });
  });
  return { marks, highlights };
}

/** Find and replace's matches as highlights. */
function findHighlights(finds: IncludedFind | null): Highlight[] {
  if (!finds) return [];
  const current = finds.current;
  return finds.ranges.map(([from, to]) => ({ from, to, find: current && current[0] === from && current[1] === to ? "current" as const : "match" as const }));
}

/** `text` (which starts at `start` in the included text) with the highlights over it, and the notes' numbers. */
function marked(text: string, start: number, highlights: Highlight[], marks: NoteMark[] = []): (Node | string)[] {
  const end = start + text.length;
  const cuts = new Set([start, end]);
  for (const h of highlights) {
    if (h.to > start && h.from < end) {
      cuts.add(Math.max(h.from, start));
      cuts.add(Math.min(h.to, end));
    }
  }
  const here = marks.filter((m) => (m.from === m.to ? m.from >= start && m.from <= end : m.to > start && m.from < end));
  for (const m of here) {
    cuts.add(Math.max(m.from, start));
    cuts.add(Math.min(m.to, end));
  }
  const points = [...cuts].sort((a, b) => a - b);
  const out: (Node | string)[] = [];
  // Numbers put in at a place come before the text from there.
  const putIn = (at: number) => {
    for (const m of here) if (m.from === at && (m.to === m.from || at === Math.max(m.from, start))) out.push(m.node());
  };
  for (let i = 0; i + 1 < points.length; i++) {
    const [a, b] = [points[i], points[i + 1]];
    putIn(a);
    const piece = text.slice(a - start, b - start);
    // Inside a Footnote shown as its number: kept, hidden.
    if (here.some((m) => m.to > m.from && m.from <= a && m.to >= b)) {
      const hidden = document.createElement("span");
      hidden.className = "cm-incl-hidden";
      hidden.textContent = piece;
      out.push(hidden);
      continue;
    }
    const over = highlights.filter((h) => h.from <= a && h.to >= b);
    if (over.length === 0) {
      out.push(piece);
      continue;
    }
    // Nested, one per Comment, so overlapping highlights darken as they do in the editor.
    let node: Node = document.createTextNode(piece);
    for (const h of over) {
      const span = document.createElement("span");
      if ("find" in h) {
        span.className = h.find === "current" ? "cm-find-match cm-find-current" : "cm-find-match";
      } else if ("annotation" in h) {
        span.className = h.mine ? "cm-annotated mine" : "cm-annotated";
      } else {
        span.className = `cm-comment cm-comment-${h.colour}${h.resolved ? " cm-comment-resolved" : ""}`;
        span.dataset.comment = h.id;
      }
      span.append(node);
      node = span;
    }
    out.push(node);
  }
  for (const m of here) if (m.from === m.to && m.from === end && end !== start) out.push(m.node());
  return out;
}

/**
 * Included text as it reads: its headings as headings (their `=` marks off, one size per
 * level), the rest as written, with its Comments highlighted. Paragraphs are runs of lines
 * between blank lines. Each block says where its text starts in `text` (`data-from`), so a
 * selection in it can be traced back.
 */
export function readable(text: string, comments: CommentHighlight[] = [], finds: IncludedFind | null = null, notes: { base: number; notes: IncludedNote[] } = { base: 0, notes: [] }): HTMLElement[] {
  const { marks, highlights: annotated } = noteMarks(notes, finds);
  const highlights: Highlight[] = [
    ...annotated,
    ...comments.flatMap((m) => m.ranges.map(([from, to]) => ({ from, to, id: m.id, colour: m.colour, resolved: m.resolved }))),
    ...findHighlights(finds),
  ];
  const out: HTMLElement[] = [];
  let paragraph: { from: number; lines: string[] } | null = null;
  const flush = () => {
    if (!paragraph) return;
    const p = document.createElement("p");
    p.dataset.from = String(paragraph.from);
    p.append(...marked(paragraph.lines.join("\n"), paragraph.from, highlights, marks));
    out.push(p);
    paragraph = null;
  };
  let at = 0;
  for (const line of text.split("\n")) {
    const heading = /^(=+\s+)(.*)$/.exec(line);
    if (heading) {
      flush();
      const h = document.createElement("div");
      h.className = `cm-inclusion-heading level-${Math.min(heading[1].trim().length, 3)}`;
      const from = at + heading[1].length;
      h.dataset.from = String(from);
      h.append(...marked(heading[2], from, highlights, marks));
      out.push(h);
    } else if (line.trim() === "") {
      flush();
    } else if (paragraph) {
      paragraph.lines.push(line);
    } else {
      paragraph = { from: at, lines: [line] };
    }
    at += line.length + 1;
  }
  flush();
  return out;
}

/**
 * The text selected inside an Inclusion's text in `view`, if that's where the selection is:
 * the Inclusion's character (`at`), and where the selection is in its text (UTF-16).
 */
export function includedSelection(view: EditorView): { at: number; from: number; to: number; quote: string } | null {
  const selection = view.dom.ownerDocument.getSelection();
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null;
  const range = selection.getRangeAt(0);
  const place = (node: Node, offset: number): { box: HTMLElement; pos: number } | null => {
    const element = node instanceof HTMLElement ? node : node.parentElement;
    const block = element?.closest<HTMLElement>("[data-from]");
    const box = block?.closest<HTMLElement>(".cm-inclusion");
    if (!block || !box || !view.dom.contains(box)) return null;
    const before = document.createRange();
    before.selectNodeContents(block);
    before.setEnd(node, offset);
    return { box, pos: Number(block.dataset.from) + before.toString().length };
  };
  const start = place(range.startContainer, range.startOffset);
  const end = place(range.endContainer, range.endOffset);
  if (!start || !end || start.box !== end.box || start.pos >= end.pos) return null;
  return { at: Number(start.box.dataset.at), from: start.pos, to: end.pos, quote: range.toString() };
}

function decorations(state: EditorState): DecorationSet {
  const { views, folded } = state.field(shownField);
  const doc = state.doc;
  const raw = state.facet(rawInclusions);
  const finds = state.facet(includedFinds);
  // The notes' numbers run on through the Work: its own Footnotes and earlier Inclusions' notes.
  const sorted = [...views].sort((a, b) => a.at - b.at);
  const own = !raw && sorted.length ? footnotesInText(doc.toString()).map((f) => f.from) : [];
  let before = 0;
  const ranges = sorted.map((v) => {
    if (raw) return Decoration.replace({ widget: new RawInclusion(v, finds.get(v.at) ?? null) }).range(v.at, v.at + 1);
    const base = before + own.filter((f) => f < v.at).length;
    before += includedNoteCount(v);
    const line = doc.lineAt(v.at);
    // On a line of its own (as Rust puts it): a block. Anywhere else (typed oddly in another
    // editor): inline, so the rest of the line still shows.
    const alone = line.from === v.at && line.to === v.at + 1;
    return Decoration.replace({ widget: new IncludedText(v, folded.includes(v.at), finds.get(v.at) ?? null, base), block: alone }).range(v.at, v.at + 1);
  });
  return Decoration.set(ranges);
}

const decorationField = StateField.define<DecorationSet>({
  create: (state) => decorations(state),
  update(deco, tr) {
    const same = tr.startState.field(shownField) === tr.state.field(shownField)
      && tr.startState.facet(rawInclusions) === tr.state.facet(rawInclusions)
      && tr.startState.facet(includedFinds) === tr.state.facet(includedFinds);
    return same ? deco.map(tr.changes) : decorations(tr.state);
  },
  provide: (field) => [EditorView.decorations.from(field), EditorView.atomicRanges.of((view) => view.state.field(field))],
});

/** Shows a Work's Inclusions in its editor. */
export function inclusions(): Extension {
  return [shownField, decorationField];
}

/** What a Work dragged from the Vault browser carries: its ID in the Vault. */
const WORK_DRAG = "application/x-wrtier-work";

/**
 * The Work being dragged from the Vault browser, noted here too: WebKit doesn't always hand a
 * drag's own data type on to where it's dropped. Kept a moment past the drag's end, since
 * WebKitGTK reports the end before the drop (see dragging.ts).
 */
let draggedWork: string | null = null;
let forgetDrag: number | undefined;

/**
 * A Work's pill in the Vault browser started being dragged: it can be dropped in a Work's text
 * (a copy: it's included) or on a folder in the Vault browser (a move).
 */
export function startWorkDrag(event: DragEvent, work: string) {
  draggedWork = work;
  window.clearTimeout(forgetDrag);
  event.dataTransfer?.setData(WORK_DRAG, work);
  if (event.dataTransfer) event.dataTransfer.effectAllowed = "copyMove";
}

/** The Work (its ID) being dragged from the Vault browser in this drag, if one is. */
export function workBeingDragged(event: DragEvent): string | null {
  return (event.dataTransfer?.types.includes(WORK_DRAG) ? event.dataTransfer.getData(WORK_DRAG) : "") || draggedWork;
}

/** The drag ended (dropped, or let go elsewhere). */
export function endWorkDrag() {
  window.clearTimeout(forgetDrag);
  forgetDrag = window.setTimeout(() => (draggedWork = null), 1000);
}

/**
 * A Work dragged from the Vault browser and dropped in the text: `dropped` is told which Work
 * (its ID) and where in the text it was dropped, to include it there. Other drops are left alone.
 */
export function workDrops(dropped: (work: string, pos: number) => void): Extension {
  const workOf = (e: DragEvent) => (e.dataTransfer?.types.includes(WORK_DRAG) ? e.dataTransfer.getData(WORK_DRAG) : "") || draggedWork;
  return EditorView.domEventHandlers({
    dragstart() {
      draggedWork = null; // text dragged in the editor, not a Work
      return false;
    },
    dragover(event) {
      if (!draggedWork &&!event.dataTransfer?.types.includes(WORK_DRAG)) return false;
      event.preventDefault(); // it may be dropped here
      if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
      return false; // the drop cursor still follows the mouse
    },
    drop(event, view) {
      const work = workOf(event);
      if (!work) return false;
      event.preventDefault();
      draggedWork = null;
      dropped(work, view.posAtCoords({ x: event.clientX, y: event.clientY }, false));
      return true;
    },
  });
}

/** The Inclusions the editor shows now. */
export function inclusionsOf(state: EditorState): InclusionView[] {
  return state.field(shownField, false)?.views ?? [];
}

/**
 * Some of the editor's text with each Inclusion's character replaced by its text, as a reader
 * sees it (for counting words), and the included part's text on its own. The Annotations printed
 * on it are in it too, as the Footnotes they print as: the Work's own count as its own words,
 * another Work's as included ones.
 */
export function readerText(state: EditorState, from = 0, to = state.doc.length): { text: string; included: string } {
  const views = inclusionsOf(state).filter((v) => v.at >= from && v.at < to && !v.missing).sort((a, b) => a.at - b.at);
  let text = "";
  let included = "";
  let at = from;
  for (const v of views) {
    text += state.doc.sliceString(at, v.at) + v.text;
    included += v.text + "\n";
    const walk = (list: AnnotationShown[]) => {
      for (const n of list) {
        const printed = `\n#footnote[${n.body}]`;
        text += printed;
        if (!n.mine) included += printed + "\n";
        walk(n.notes);
      }
    };
    walk(v.annotations ?? []);
    at = v.at + 1;
  }
  text += state.doc.sliceString(at, to);
  return { text: text.split(INCLUSION).join(""), included };
}

/**
 * Where to put a new Inclusion so it's on a line of its own, near `pos`: the text to insert, where
 * to insert it, and where its character ends up.
 */
export function placeInclusion(doc: Text, pos: number): { from: number; insert: string; at: number } {
  const line = doc.lineAt(pos);
  if (line.from === line.to) return { from: pos, insert: INCLUSION, at: pos };
  if (pos === line.from) return { from: pos, insert: INCLUSION + "\n", at: pos };
  return { from: line.to, insert: "\n" + INCLUSION, at: line.to + 1 };
}
