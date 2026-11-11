// Inclusions (ADR 0015): another Work, or one Chapter of it, shown live where its one
// character stands in the text. The editor shows the included text read-only under a bar that
// names it, with "Go to" to open that Work; each one folds to just its bar. The character is
// one unit: the cursor goes before or after it, never inside, and deleting it removes it whole.

import { StateEffect, StateField, type EditorState, type Extension, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { countsOf } from "./counts";

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
    return this.folded === other.folded && a.label === b.label && a.text === b.text && a.missing === b.missing && a.work === b.work && a.at === b.at;
  }

  toDOM(view: EditorView) {
    const v = this.v;
    const box = document.createElement("div");
    box.className = "cm-inclusion" + (v.missing ? " missing" : "") + (this.folded ? " folded" : "");
    box.contentEditable = "false";
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
    if (v.missing) {
      const why = document.createElement("p");
      why.className = "cm-inclusion-missing";
      why.textContent = v.missing;
      box.append(why);
    } else if (!this.folded) {
      const text = document.createElement("div");
      text.className = "cm-inclusion-text";
      if (v.language) text.lang = v.language;
      text.append(...readable(v.text));
      box.append(text);
    }
    return box;
  }

  // Clicks inside it are its own (buttons, selecting text to copy), not the editor's.
  ignoreEvent() {
    return true;
  }
}

/**
 * Included text as it reads: its headings as headings (their `=` marks off, one size per
 * level), the rest as written. Paragraphs are runs of lines between blank lines.
 */
export function readable(text: string): HTMLElement[] {
  const out: HTMLElement[] = [];
  let paragraph: string[] = [];
  const flush = () => {
    if (paragraph.length === 0) return;
    const p = document.createElement("p");
    p.textContent = paragraph.join("\n");
    out.push(p);
    paragraph = [];
  };
  for (const line of text.split("\n")) {
    const heading = /^(=+)\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      const h = document.createElement("div");
      h.className = `cm-inclusion-heading level-${Math.min(heading[1].length, 3)}`;
      h.textContent = heading[2];
      out.push(h);
    } else if (line.trim() === "") {
      flush();
    } else {
      paragraph.push(line);
    }
  }
  flush();
  return out;
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
