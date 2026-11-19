// The Footnotes tab (CONTEXT.md): every Footnote and Annotation that prints in the Work in front,
// in the order they print, with the editor's numbers, and its Detached annotations last. The
// Work's own Footnotes go to their place in the text, and its own Annotations open to change;
// what its Inclusions print belongs to the Works it includes, and is listed to read. The tab is
// offered only while there's something to list.

import type { EditorView } from "@codemirror/view";
import { footnoteSpans, footnoteText, includedNotes, numberFootnotes } from "./footnotes";
import { markupSpans } from "./formatted";
import { parseMarkup } from "./markup";
import { inclusionsOf } from "./inclusions";

/** One row of the tab. */
export type NoteRow = {
  number: number;
  text: string;
  /** Where to go in the Work's text: inside its own Footnote, or at the Inclusion's character. */
  at: number;
  /** Set for one in an Inclusion's text: what the Inclusion shows. */
  from?: string;
  /** Set for an Annotation: which, whose, and whether it's this Work's own. */
  annotation?: { id: string; owner: string; mine: boolean; quote: string; ownerLabel: string; onNote: boolean };
};

/** A Detached annotation of the Work (vault/annotations.rs `DetachedView`). */
export type DetachedRow = { id: string; label: string; quote: string; body: string };

/** The Footnotes and Annotations that print in the Work in an editor, in order, numbered as the editor numbers them. */
export function noteRows(state: EditorView["state"]): NoteRow[] {
  const source = state.doc.toString();
  const own = footnoteSpans(markupSpans(state) ?? parseMarkup(source));
  const included = inclusionsOf(state)
    .filter((v) => !v.missing)
    .map((v) => ({ v, notes: includedNotes(v.text, v.annotations) }))
    .filter(({ notes }) => notes.length > 0);
  const numbers = numberFootnotes(own, included.map(({ v, notes }) => ({ at: v.at, count: notes.length })));
  const rows: NoteRow[] = own.map((f) => ({ number: numbers.get(f.from) ?? 0, text: footnoteText(source, f), at: f.open[1] }));
  for (const { v, notes } of included) {
    // Numbered after everything of the Work's own before the Inclusion, and earlier Inclusions'.
    const before = own.filter((f) => f.from < v.at).length + included.filter((o) => o.v.at < v.at).reduce((n, o) => n + o.notes.length, 0);
    const name = v.label.split(": ")[0] || v.label;
    notes.forEach((n, i) => {
      const number = before + i + 1;
      if (n.kind === "footnote") rows.push({ number, text: n.text, at: v.at, from: name });
      else {
        const a = n.note;
        rows.push({
          number, text: a.body, at: v.at, from: a.mine ? undefined : name,
          annotation: { id: a.id, owner: a.owner, mine: a.mine, quote: n.on ? n.on.body : a.quote, ownerLabel: a.ownerLabel, onNote: n.on !== null },
        });
      }
    });
  }
  return rows.sort((a, b) => a.number - b.number);
}

export type FootnotesHooks = {
  /** The Work in front, if it's a Work. */
  work(): string | null;
  view(work: string): EditorView | null;
  /** Puts the cursor at `at` in the Work's text and brings it into view. */
  go(work: string, at: number): void;
  /** Opens one of the Annotations shown (to change, if it's the Work's own; to read otherwise). */
  openAnnotation(work: string, id: string, owner: string): void;
  /** The Work's Detached annotations. */
  detached(work: string): Promise<DetachedRow[]>;
  /** Deletes some of the Work's Annotations (the Detached ones from one Work). */
  deleteAnnotations(work: string, ids: string[]): void;
  /** Whether the Work in front has anything to list (`now`: another Work just came to the front). */
  available(has: boolean, now: boolean): void;
};

const SETTLE_MS = 400;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
}

export class FootnotesPanel {
  private visible = false;
  private timer: number | undefined;
  private shownFor: string | null = null;
  /** Asked of Rust last, so an older answer arriving late is dropped. */
  private asking = 0;

  constructor(private readonly list: HTMLElement, private readonly hooks: FootnotesHooks) {}

  setVisible(visible: boolean) {
    this.visible = visible;
    if (visible) this.render();
  }

  /** The Work in front changed (or another came to the front). */
  workChanged() {
    clearTimeout(this.timer);
    this.render();
  }

  /** A Work's text (or its Inclusions, or its Annotations) changed. */
  edited(id: string) {
    if (id !== this.hooks.work()) return;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.render(), SETTLE_MS);
  }

  render() {
    const work = this.hooks.work();
    const view = work ? this.hooks.view(work) : null;
    const rows = view ? noteRows(view.state) : [];
    const now = work !== this.shownFor;
    this.shownFor = work;
    // What prints is known at once; the Detached annotations are asked of Rust (only a Work
    // with Inclusions can have any).
    const mayHaveDetached = view !== null && inclusionsOf(view.state).length > 0 || rows.some((r) => r.annotation);
    this.hooks.available(rows.length > 0, now);
    this.draw(work, rows, []);
    if (!work || !mayHaveDetached) return;
    const asked = ++this.asking;
    void this.hooks.detached(work).then((detached) => {
      if (asked !== this.asking || work !== this.hooks.work()) return;
      if (detached.length > 0) this.hooks.available(true, false);
      this.draw(work, rows, detached);
    }).catch(() => {
      // Not this time: what prints is listed anyway.
    });
  }

  private draw(work: string | null, rows: NoteRow[], detached: DetachedRow[]) {
    if (!this.visible) return;
    this.list.replaceChildren();
    if (!work || (rows.length === 0 && detached.length === 0)) {
      this.list.append(el("p", "muted", "No Footnotes. Write one with “Insert footnote”, or #footnote[its text]."));
      return;
    }
    const ol = el("ol", "footnote-list");
    for (const row of rows) ol.append(this.rowOf(work, row));
    this.list.append(ol);
    if (detached.length > 0) this.list.append(this.detachedOf(work, detached));
  }

  private rowOf(work: string, row: NoteRow): HTMLElement {
    const a = row.annotation;
    const li = el("li", `footnote-row${row.from ? " included" : ""}${a ? " annotation" : ""}${a?.mine ? " mine" : ""}`);
    li.tabIndex = 0;
    li.append(el("span", "footnote-number", String(row.number)), el("span", "footnote-text", row.text || "(empty)"));
    if (a) {
      const about = a.onNote ? "on a note" : `on “${a.quote.replace(/\s+/g, " ").slice(0, 80)}”`;
      const whose = a.mine ? "Annotation" : `${a.ownerLabel || "Another Work"}'s Annotation`;
      li.append(el("span", "footnote-from", `${whose} ${about}${row.from ? `, in ${row.from}` : ""}`));
      li.title = a.mine ? "This Work's Annotation: click to change it." : "Another Work's Annotation: click to read it, or annotate it.";
    } else if (row.from) {
      li.append(el("span", "footnote-from", `In ${row.from}`));
      li.title = "This Footnote belongs to the included Work: change it there.";
    }
    const go = () => (a ? this.hooks.openAnnotation(work, a.id, a.owner) : this.hooks.go(work, row.at));
    li.addEventListener("click", go);
    li.addEventListener("keydown", (e) => {
      if (e.key === "Enter") go();
    });
    return li;
  }

  /** The Detached annotations, grouped by the Work they were on, each group with "Delete all". */
  private detachedOf(work: string, detached: DetachedRow[]): HTMLElement {
    const section = el("section", "footnote-detached");
    section.append(el("h3", "", "Detached annotations"));
    section.append(el("p", "muted", "Their text isn't shown in any Inclusion here any more, so they aren't printed. They come back by themselves when it is."));
    const groups = new Map<string, DetachedRow[]>();
    for (const d of detached) groups.set(d.label, [...(groups.get(d.label) ?? []), d]);
    for (const [label, list] of groups) {
      const header = el("div", "footnote-detached-group");
      header.append(el("span", "", `${label} · ${list.length} ${list.length === 1 ? "Annotation" : "Annotations"}`));
      const remove = el("button", "link", list.length === 1 ? "Delete" : "Delete all");
      remove.type = "button";
      remove.addEventListener("click", () => this.hooks.deleteAnnotations(work, list.map((d) => d.id)));
      header.append(remove);
      section.append(header);
      const ul = el("ul", "footnote-list");
      for (const d of list) {
        const li = el("li", "footnote-row detached");
        li.append(el("span", "footnote-number", "–"), el("span", "footnote-text", d.body), el("span", "footnote-from", `Was on “${d.quote.replace(/\s+/g, " ").slice(0, 80)}”`));
        ul.append(li);
      }
      section.append(ul);
    }
    return section;
  }
}
