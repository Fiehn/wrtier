// The Marks panel: every Mark in the Work in front, by kind (Mistakes, Rule breaks, Queries), each
// as the card the editor shows, so a Work can be gone through without moving in the editor.
// Accepting a Suggestion here edits the text where it is, one undo step, as on the editor's card;
// the sentence over a card goes there in the editor.
//
// The same word marked in several places (a name the Dictionary doesn't know) is one entry, with
// the Mod's own actions, which act on every place ("Allow in Work"); its places open under it,
// each with its own Suggestions and Dismiss. Suggestions a Mod works out only when asked (the
// Spellchecker's) are asked for as their card comes into view, not for every Mark at once.
//
// Drawn again a moment after the Marks change, and only while the panel shows them. A card drawn
// before the text changed acts on nothing: the panel is drawn again instead.

import type { EditorView } from "@codemirror/view";
import type { Text } from "@codemirror/state";
import { accept, dismiss, markHooks, marksOf, runAction, showMark, stillShown, withSuggestions, type Mark, type MarkLevel } from "./marks";

const LEVELS: MarkLevel[] = ["mistake", "rule", "query"];
const PLURAL: Record<MarkLevel, string> = { mistake: "Mistakes", rule: "Rule breaks", query: "Queries" };

/** How long after the Marks change the panel is drawn again (they change with every key typed). */
const REDRAW_MS = 300;

/** The text around a Mark: up to `reach` characters each side, within its line, cut at a word. */
export function contextOf(text: string, from: number, to: number, reach = 50): { before: string; marked: string; after: string } {
  let start = Math.max(0, from - reach);
  let end = Math.min(text.length, to + reach);
  const lineStart = text.lastIndexOf("\n", from - 1) + 1;
  const lineEnd = text.indexOf("\n", to);
  start = Math.max(start, lineStart);
  if (lineEnd !== -1) end = Math.min(end, lineEnd);
  let before = text.slice(start, from);
  let after = text.slice(to, end);
  // Cut short: start and end at a whole word, with "…" for what's left out.
  if (start > lineStart) before = `…${before.replace(/^\S*\s/, "")}`;
  if (lineEnd === -1 ? end < text.length : end < lineEnd) after = `${after.replace(/\s\S*$/, "")}…`;
  return { before, marked: text.slice(from, to), after };
}

/** Marks of one kind, in reading order, the same Mod's rule on the same text together (at its first place). */
export function groupMarks(marks: Mark[], textOf: (m: Mark) => string): { key: string; marks: Mark[] }[] {
  const groups = new Map<string, Mark[]>();
  for (const m of marks) {
    const key = `${m.mod}/${m.rule}/${textOf(m)}`;
    const group = groups.get(key);
    if (group) group.push(m);
    else groups.set(key, [m]);
  }
  return [...groups].map(([key, marks]) => ({ key, marks }));
}

export type MarksPanelHooks = {
  /** The Work in front's editor, if a Work is in front. */
  view(): EditorView | null;
  /** Does the Work in front have a Mod that makes Marks (on this Device)? */
  marking(): boolean;
};

export class MarksPanel {
  private visible = false;
  private timer: number | undefined;
  /** The kinds whose section is closed, and the repeated Marks whose places are open (this session). */
  private closed = new Set<MarkLevel>();
  private openGroups = new Set<string>();
  /** A kind to bring into view once the panel is next drawn (asked for while it was out of view). */
  private toShow: MarkLevel | null = null;
  /** Suggestions asked for as their place comes into view. */
  private readonly seen = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      this.seen.unobserve(entry.target);
      (entry.target as HTMLElement & { load?: () => void }).load?.();
    }
  });

  constructor(private readonly list: HTMLElement, private readonly hooks: MarksPanelHooks) {}

  setVisible(visible: boolean) {
    this.visible = visible;
    if (visible) this.draw();
  }

  /** Another Work came to the front. */
  workChanged() {
    this.draw();
  }

  /** The Marks changed in the Work in front: drawn again once they've settled. */
  changed() {
    if (!this.visible) return;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.draw(), REDRAW_MS);
  }

  /** Opens the section of one kind (the counts under the editor ask for it) and brings it into view, now or once shown. */
  showKind(level: MarkLevel) {
    this.closed.delete(level);
    this.toShow = level;
    this.draw();
  }

  private draw() {
    clearTimeout(this.timer);
    if (!this.visible) return;
    this.seen.disconnect();
    const view = this.hooks.view();
    if (!view) return this.list.replaceChildren(note("Open a Work to see its Marks."));
    if (!this.hooks.marking()) return this.list.replaceChildren(note("None of this Work's Mods makes Marks. Add one, such as the Spellchecker, under Mods… in the tab's ⌄ menu."));
    const marks = marksOf(view.state);
    if (marks.length === 0) return this.list.replaceChildren(note("No Marks."));
    const doc = view.state.doc;
    const textOf = (m: Mark) => doc.sliceString(m.from, m.to);
    const sections: HTMLElement[] = [];
    for (const level of LEVELS) {
      const ofKind = marks.filter((m) => m.level === level);
      if (ofKind.length === 0) continue;
      const section = document.createElement("details");
      section.className = "marks-kind";
      section.dataset.level = level;
      section.open = !this.closed.has(level);
      section.addEventListener("toggle", () => {
        if (section.open) this.closed.delete(level);
        else this.closed.add(level);
      });
      const summary = document.createElement("summary");
      summary.append(PLURAL[level], " ", el("span", "marks-count", String(ofKind.length)));
      section.append(summary);
      for (const group of groupMarks(ofKind, textOf)) {
        section.append(group.marks.length === 1 ? this.single(view, doc, group.marks[0]) : this.repeated(view, doc, group.key, group.marks, textOf(group.marks[0])));
      }
      sections.push(section);
    }
    this.list.replaceChildren(...sections);
    if (this.toShow) {
      this.list.querySelector(`[data-level="${this.toShow}"]`)?.scrollIntoView({ block: "start" });
      this.toShow = null;
    }
  }

  /** One Mark: the text around it, why, its Suggestions, the Mod's actions and Dismiss. */
  private single(view: EditorView, doc: Text, mark: Mark): HTMLElement {
    const entry = el("article", `mark-entry mark-entry-${mark.level}`);
    entry.append(this.context(view, doc, mark), el("p", "mark-why", mark.why), this.suggestions(view, doc, mark));
    const row = el("div", "mark-row");
    for (const action of mark.actions ?? []) row.append(this.button(action.label, "mark-action", view, doc, mark, () => runAction(view, action)));
    row.append(this.button(mark.dismissLabel ?? "Dismiss", "mark-dismiss", view, doc, mark, () => dismiss(view, mark, false)));
    entry.append(row, this.stopRule(view, mark));
    return entry;
  }

  /** The same Mark in several places: once, with the Mod's actions (for every place), its places under it. */
  private repeated(view: EditorView, doc: Text, key: string, marks: Mark[], text: string): HTMLElement {
    const first = marks[0];
    const entry = el("article", `mark-entry mark-entry-${first.level} mark-repeated`);
    const head = el("p", "mark-repeated-head");
    head.append(el("span", `mark-hit mark-hit-${first.level}`, text), el("span", "mark-times", `×${marks.length}`));
    entry.append(head, el("p", "mark-why", first.why));
    if (first.actions?.length) {
      const row = el("div", "mark-row");
      for (const action of first.actions) row.append(this.button(action.label, "mark-action", view, doc, first, () => runAction(view, action)));
      entry.append(row);
    }
    const places = document.createElement("details");
    places.className = "mark-places";
    places.open = this.openGroups.has(key);
    places.addEventListener("toggle", () => {
      if (places.open) this.openGroups.add(key);
      else this.openGroups.delete(key);
    });
    const summary = document.createElement("summary");
    summary.textContent = `${marks.length} places`;
    places.append(summary);
    for (const mark of marks) {
      const place = el("div", "mark-place");
      place.append(this.context(view, doc, mark), this.suggestions(view, doc, mark));
      const row = el("div", "mark-row");
      row.append(this.button(mark.dismissLabel ?? "Dismiss", "mark-dismiss", view, doc, mark, () => dismiss(view, mark, false)));
      place.append(row);
      places.append(place);
    }
    entry.append(places, this.stopRule(view, first));
    return entry;
  }

  /** The text around a Mark, the Mark underlined: clicked, the editor goes there and opens its card. */
  private context(view: EditorView, doc: Text, mark: Mark): HTMLElement {
    const reach = 50;
    const from = Math.max(0, mark.from - reach - 1);
    const to = Math.min(doc.length, mark.to + reach + 1);
    // Only the text near the Mark is read; positions are made relative to it.
    const near = doc.sliceString(from, to);
    const { before, marked, after } = contextOf(near, mark.from - from, mark.to - from, reach);
    const b = el("button", "mark-context");
    b.type = "button";
    b.title = "Show it in the text";
    b.append(before, el("span", `mark-hit mark-hit-${mark.level}`, marked), after);
    b.addEventListener("click", () => {
      if (this.stale(view, doc, mark)) return;
      showMark(view, mark);
    });
    return b;
  }

  /** A Mark's Suggestions: at once if it has them, else asked of its Mod when they come into view. */
  private suggestions(view: EditorView, doc: Text, mark: Mark): HTMLElement {
    const box = el("div", "mark-suggestions") as HTMLElement & { load?: () => void };
    const fill = (m: Mark) => {
      box.replaceChildren(...m.suggestions.map((s, i) => this.button(s.label, "mark-suggestion", view, doc, mark, () => accept(view, m, i, false))));
      if (m.suggestions.length === 0 && mark.suggest) box.append(el("p", "mark-looking", "No suggestions."));
    };
    if (!mark.suggest) {
      fill(mark);
      return box;
    }
    box.append(el("p", "mark-looking", "Looking for suggestions…"));
    box.load = () => {
      // Typed since this was drawn: it's drawn again (and asked again) in a moment.
      if (view.state.doc !== doc) return;
      void withSuggestions(view, mark).then((m) => {
        if (box.isConnected && view.state.doc === doc) fill(m);
      });
    };
    this.seen.observe(box);
    return box;
  }

  /** "Stop checking this rule in this Work", as on the editor's card. */
  private stopRule(view: EditorView, mark: Mark): HTMLElement | string {
    const hooks = view.state.facet(markHooks);
    if (!hooks?.stopRule) return "";
    const b = el("button", "mark-stop", "Stop checking this rule in this Work");
    b.type = "button";
    b.addEventListener("click", () => hooks.stopRule!(mark));
    return b;
  }

  /** A button on a card: acts only if the text is as it was when the card was drawn. */
  private button(label: string, cls: string, view: EditorView, doc: Text, mark: Mark, run: () => void): HTMLButtonElement {
    const b = el("button", cls, label);
    b.type = "button";
    b.addEventListener("click", () => {
      if (this.stale(view, doc, mark)) return;
      run();
      this.changed();
    });
    return b;
  }

  /** The text changed since this card was drawn, or its Mark is gone: drawn again instead of acting. */
  private stale(view: EditorView, doc: Text, mark: Mark): boolean {
    if (view.state.doc === doc && stillShown(view.state, mark)) return false;
    this.draw();
    return true;
  }
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
}

function note(text: string): HTMLElement {
  return el("p", "muted", text);
}

