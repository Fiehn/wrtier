// Inclusions (ADR 0015): another Work, or one Chapter of it, shown live where its one
// character stands in the text. The editor shows the included text read-only under a bar that
// names it, with "Go to" to open that Work; each one folds to just its bar. The character is
// one unit: the cursor goes before or after it, never inside, and deleting it removes it whole.

import { StateEffect, StateField, type EditorState, type Extension, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { countsOf } from "./counts";
import type { Mark } from "./sync";
import { isCommand } from "./keys";

/** The character that stands for an Inclusion (change.rs `INCLUSION`). */
export const INCLUSION = "￼";

/** An Inclusion as Rust shows it (vault/inclusions.rs `InclusionView`). */
export type InclusionView = {
  /** Where its character is in the editor's text (UTF-16). */
  at: number;
  /** The included Work's ID in the Vault, to open it; null if it wasn't found. */
  work: string | null;
  label: string;
  /** The included text, read-only. */
  text: string;
  /** Why it can't be shown, if it can't. */
  missing: string | null;
  language: string;
  whole: boolean;
  /** Where the included text starts in its own Work (UTF-16): "Go to" opens it there. */
  starts: number;
  /** The included Work's Comments on this text (they belong to it, and show in both); ranges in `text`. */
  comments: Mark[];
  /** A new Chapter right after the included one, not included (it was split, or one was put
   *  after it), until the Author includes it too or dismisses this. */
  split?: { title: string; at: number } | null;
  /** Styles given to this Inclusion only. */
  styles?: string[];
};

/** A Chapter that can be included (vault/inclusions.rs `ChapterChoice`). */
export type ChapterChoice = { title: string; at: number };

/** The Inclusions as Rust last gave them, for the text as it is now. */
export const setInclusions = StateEffect.define<InclusionView[]>();
/** Folds or unfolds the Inclusion whose character is here. */
const toggleFold = StateEffect.define<number>();

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
    }
    return views === value.views && folded === value.folded ? value : { views, folded };
  },
});

class IncludedText extends WidgetType {
  constructor(readonly v: InclusionView, readonly folded: boolean) {
    super();
  }

  eq(other: IncludedText) {
    const a = this.v;
    const b = other.v;
    return this.folded === other.folded && a.label === b.label && a.text === b.text && a.missing === b.missing && a.work === b.work && a.at === b.at
      && JSON.stringify(a.comments) === JSON.stringify(b.comments) && JSON.stringify(a.split) === JSON.stringify(b.split) && JSON.stringify(a.styles) === JSON.stringify(b.styles);
  }

  toDOM(view: EditorView) {
    const v = this.v;
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
      if (!isCommand(e, "addComment")) return;
      e.preventDefault();
      box.dispatchEvent(new CustomEvent("wrtier-add-comment", { bubbles: true }));
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
    bar.append(styles);
    if (v.work) {
      const go = document.createElement("button");
      go.type = "button";
      go.className = "cm-inclusion-go";
      go.textContent = "Go to";
      go.title = `Open ${v.work} to change this text`;
      go.addEventListener("mousedown", (e) => e.preventDefault());
      go.addEventListener("click", () => {
        box.dispatchEvent(new CustomEvent("wrtier-open-work", { bubbles: true, detail: { work: v.work, at: v.starts } }));
      });
      bar.append(go);
    }
    box.append(bar);
    if (v.split) box.append(this.splitNote(v, v.split));
    if (v.missing) {
      const why = document.createElement("p");
      why.className = "cm-inclusion-missing";
      why.textContent = v.missing;
      box.append(why);
    } else if (!this.folded) {
      const text = document.createElement("div");
      text.className = "cm-inclusion-text";
      if (v.language) text.lang = v.language;
      text.append(...readable(v.text, v.comments));
      // A highlight clicked: its Comments, in the Comments panel.
      text.addEventListener("click", (e) => {
        const ids: string[] = [];
        for (let el = e.target as Element | null; el && el !== text; el = el.parentElement) {
          if (el instanceof HTMLElement && el.dataset.comment) ids.push(el.dataset.comment);
        }
        if (ids.length > 0) box.dispatchEvent(new CustomEvent("wrtier-open-comments", { bubbles: true, detail: ids }));
      });
      box.append(text);
    }
    return box;
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

/** A Comment's highlight in included text: where (UTF-16, in the included text) and whose. */
type Highlight = { from: number; to: number; id: string; colour: string; resolved: boolean };

/** `text` (which starts at `start` in the included text) with the highlights over it. */
function marked(text: string, start: number, highlights: Highlight[]): (Node | string)[] {
  const end = start + text.length;
  const cuts = new Set([start, end]);
  for (const h of highlights) {
    if (h.to > start && h.from < end) {
      cuts.add(Math.max(h.from, start));
      cuts.add(Math.min(h.to, end));
    }
  }
  const points = [...cuts].sort((a, b) => a - b);
  const out: (Node | string)[] = [];
  for (let i = 0; i + 1 < points.length; i++) {
    const [a, b] = [points[i], points[i + 1]];
    const piece = text.slice(a - start, b - start);
    const over = highlights.filter((h) => h.from <= a && h.to >= b);
    if (over.length === 0) {
      out.push(piece);
      continue;
    }
    // Nested, one per Comment, so overlapping highlights darken as they do in the editor.
    let node: Node = document.createTextNode(piece);
    for (const h of over) {
      const span = document.createElement("span");
      span.className = `cm-comment cm-comment-${h.colour}${h.resolved ? " cm-comment-resolved" : ""}`;
      span.dataset.comment = h.id;
      span.append(node);
      node = span;
    }
    out.push(node);
  }
  return out;
}

/**
 * Included text as it reads: its headings as headings (their `=` marks off, one size per
 * level), the rest as written, with its Comments highlighted. Paragraphs are runs of lines
 * between blank lines. Each block says where its text starts in `text` (`data-from`), so a
 * selection in it can be traced back.
 */
export function readable(text: string, comments: Mark[] = []): HTMLElement[] {
  const highlights: Highlight[] = comments.flatMap((m) => m.ranges.map(([from, to]) => ({ from, to, id: m.id, colour: m.colour, resolved: m.resolved })));
  const out: HTMLElement[] = [];
  let paragraph: { from: number; lines: string[] } | null = null;
  const flush = () => {
    if (!paragraph) return;
    const p = document.createElement("p");
    p.dataset.from = String(paragraph.from);
    p.append(...marked(paragraph.lines.join("\n"), paragraph.from, highlights));
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
      h.append(...marked(heading[2], from, highlights));
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
  const ranges = [...views].sort((a, b) => a.at - b.at).map((v) => {
    const line = doc.lineAt(v.at);
    // On a line of its own (as Rust puts it): a block. Anywhere else (typed oddly in another
    // editor): inline, so the rest of the line still shows.
    const alone = line.from === v.at && line.to === v.at + 1;
    return Decoration.replace({ widget: new IncludedText(v, folded.includes(v.at)), block: alone }).range(v.at, v.at + 1);
  });
  return Decoration.set(ranges);
}

const decorationField = StateField.define<DecorationSet>({
  create: (state) => decorations(state),
  update(deco, tr) {
    return tr.startState.field(shownField) === tr.state.field(shownField) ? deco.map(tr.changes) : decorations(tr.state);
  },
  provide: (field) => [EditorView.decorations.from(field), EditorView.atomicRanges.of((view) => view.state.field(field))],
});

/** Shows a Work's Inclusions in its editor. */
export function inclusions(): Extension {
  return [shownField, decorationField];
}

/** The Inclusions the editor shows now. */
export function inclusionsOf(state: EditorState): InclusionView[] {
  return state.field(shownField, false)?.views ?? [];
}

/**
 * Some of the editor's text with each Inclusion's character replaced by its text, as a reader
 * sees it (for counting words), and the included part's text on its own.
 */
export function readerText(state: EditorState, from = 0, to = state.doc.length): { text: string; included: string } {
  const views = inclusionsOf(state).filter((v) => v.at >= from && v.at < to && !v.missing).sort((a, b) => a.at - b.at);
  let text = "";
  let included = "";
  let at = from;
  for (const v of views) {
    text += state.doc.sliceString(at, v.at) + v.text;
    included += v.text + "\n";
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
