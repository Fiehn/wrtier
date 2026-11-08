// Story elements in the editor: which ones a Work has, how the Formatted view shows them (a
// chip, or a block with its own lines), the "Add it?" chip for one from the Style library the
// Work hasn't added, and `***` on an empty line becoming a scene break.

import { type Extension, Facet, type Text } from "@codemirror/state";
import { EditorView, WidgetType } from "@codemirror/view";

/** What the editor knows about one Story element. */
export type ElementInfo = {
  /** How it's written after `#`. */
  name: string;
  title: string;
  /** "chip" or "block". */
  shownAs: string;
  /** Does this Work have it (built in, or added in its Preamble)? */
  inWork: boolean;
};

/** The Story elements of the Work in this editor (the last one given wins). */
export const elementsFacet = Facet.define<ElementInfo[], ElementInfo[]>({
  combine: (values) => values[values.length - 1] ?? [],
});

/** A call's head in the text: `#name[` or `#name()` (Story elements take only a body in v1). */
const HEAD = /^#([a-z][a-z0-9-]*)(\(\))?(\[)?$/;

/** A Story element written in the text: where its parts are. */
export type Call = {
  info: ElementInfo;
  from: number;
  /** The head: `#name[` or `#name()`. */
  head: [number, number];
  /** The `]` that ends its body, if it has one. */
  close: [number, number] | null;
};

/** The `]` matching a `[` just before `from` (skipping escaped and nested brackets), or -1. */
export function matchingBracket(doc: Text, from: number, limit = 50_000): number {
  const text = doc.sliceString(from, Math.min(doc.length, from + limit));
  let depth = 1;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "\\") { i++; continue; }
    if (c === "[") depth++;
    else if (c === "]" && --depth === 0) return from + i;
  }
  return -1;
}

/** If the code at `from`..`to` is a Story element's head, the call it starts. */
export function callAt(doc: Text, from: number, to: number, elements: ElementInfo[]): Call | null {
  const m = HEAD.exec(doc.sliceString(from, to));
  if (!m) return null;
  const info = elements.find((e) => e.name === m[1]);
  if (!info) return null;
  let close: [number, number] | null = null;
  if (m[3]) {
    const at = matchingBracket(doc, to);
    if (at === -1) return null; // not closed yet: shown as typed
    close = [at, at + 1];
  }
  return { info, from, head: [from, to], close };
}

/** A Story element's head, shown as a small label; or, for one the Work lacks, "Add it?". */
export class ElementChip extends WidgetType {
  constructor(readonly info: ElementInfo, readonly whole: boolean) {
    super();
  }
  eq(other: ElementChip) {
    return other.info.name === this.info.name && other.info.title === this.info.title
      && other.info.inWork === this.info.inWork && other.whole === this.whole;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = `cm-tp-element cm-tp-element-${this.info.shownAs}${this.whole ? " cm-tp-element-whole" : ""}`;
    if (this.info.inWork) {
      el.textContent = this.info.name === "scene-break" ? "⁂ Scene break" : this.info.title;
      el.title = `#${this.info.name}: a Story element. Click next to it, or move the cursor into it, to see it as written.`;
    } else {
      el.classList.add("cm-tp-element-missing");
      el.textContent = `${this.info.title} isn't in this Work. Add it?`;
      el.title = `#${this.info.name} is in the Style library but not in this Work's Preamble, so it won't typeset. Click to add it.`;
      el.addEventListener("mousedown", (e) => {
        e.preventDefault();
        el.dispatchEvent(new CustomEvent("wrtier-add-element", { detail: this.info.name, bubbles: true }));
      });
    }
    return el;
  }
  ignoreEvent() {
    return !this.info.inWork; // the "Add it?" chip takes its own clicks
  }
}

/** Typing the third `*` of `***` on an otherwise empty line makes it a scene break. Two steps,
 *  so Ctrl+Z gives back the `***` if that's what was meant. */
export function makesSceneBreak(doc: Text, from: number, to: number, text: string): boolean {
  if (text !== "*" || from !== to) return false;
  const line = doc.lineAt(from);
  return line.text === "**" && from === line.to;
}

const sceneBreaks = EditorView.inputHandler.of((view, from, to, text) => {
  if (!makesSceneBreak(view.state.doc, from, to, text)) return false;
  const line = view.state.doc.lineAt(from);
  view.dispatch({ changes: { from, to, insert: "*" }, selection: { anchor: from + 1 }, userEvent: "input.type" });
  view.dispatch({
    changes: { from: line.from, to: line.from + 3, insert: "#scene-break()" },
    selection: { anchor: line.from + "#scene-break()".length },
    userEvent: "input.type",
  });
  return true;
});

/** For a Work's editor: what Story elements it has, and `***` becoming a scene break. */
export function storyElements(elements: ElementInfo[]): Extension {
  return [elementsFacet.of(elements), sceneBreaks];
}
