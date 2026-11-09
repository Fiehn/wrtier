// Typst colouring for the Raw view and Style library tabs.
//
// The colours come from Typst's own parser in Rust (`colour_typst`), so they always agree with
// how Typst reads the text. A library file is small and is coloured whole. A Work can be a whole
// novel, so only the paragraphs on screen are coloured, and scrolling asks again. Colours are
// asked for after the same pause as compiling; in between they move along with the typing, and
// text typed since shows uncoloured until the next answer. The theme's `--syntax-…` variables
// set the colours (styles.css).

import { invoke } from "@tauri-apps/api/core";
import { StateEffect, StateField, type Extension, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { paragraphAround } from "./formatted";

/** One coloured stretch, counted from the start of the piece of text that was sent. */
export type Colour = { from: number; to: number; class: string };

/** How long after the last keystroke the colours are asked for (as compiling waits). */
const PAUSE_MS = 300;
/** How long after scrolling stops a Work's newly shown paragraphs are asked for. */
const SCROLL_MS = 80;

const setColours = StateEffect.define<DecorationSet>();

const coloursField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(setColours)) return effect.value;
    return tr.docChanged ? value.map(tr.changes) : value;
  },
  provide: (field) => EditorView.decorations.from(field),
});

const marks = new Map<string, Decoration>();
const markFor = (cls: string) => {
  let mark = marks.get(cls);
  if (!mark) marks.set(cls, (mark = Decoration.mark({ class: cls })));
  return mark;
};

/** The colours of a piece that starts at `offset`, as decorations on the whole text. */
export function decorationsFor(colours: Colour[], offset: number, docLength: number): DecorationSet {
  const ranges = colours
    .filter((c) => c.from < c.to && offset + c.to <= docLength)
    .map((c) => markFor(c.class).range(offset + c.from, offset + c.to));
  return Decoration.set(ranges, true);
}

/** Which part of the text to colour: all of it, or the whole paragraphs around what's shown. */
export function pieceToColour(doc: Text, shown: { from: number; to: number }, whole: boolean): [number, number] {
  if (whole || doc.length === 0) return [0, doc.length];
  return paragraphAround(doc, shown.from, shown.to);
}

/** Asks Rust for the colours of `text`. Replaced in tests. */
export type Colourer = (text: string) => Promise<Colour[]>;
const askRust: Colourer = (text) => invoke<Colour[]>("colour_typst", { text });

/**
 * Typst colouring. `whole`: colour the whole text (a library file); otherwise only the
 * paragraphs on screen (a Work's Raw view).
 */
export function typstColouring(whole: boolean, ask: Colourer = askRust): Extension {
  const plugin = ViewPlugin.fromClass(
    class {
      private timer: number | undefined;
      /** Counts requests, so an answer overtaken by a newer request is dropped. */
      private asked = 0;
      /** The part the colours shown were worked out for, or null once the text changed. */
      private covered: [number, number] | null = null;
      private gone = false;

      constructor(readonly view: EditorView) {
        this.schedule(0);
      }

      update(update: ViewUpdate) {
        if (update.docChanged) {
          this.covered = null;
          this.schedule(PAUSE_MS);
        } else if (!whole && update.viewportChanged && !this.covers(update.view.viewport)) {
          this.schedule(SCROLL_MS);
        }
      }

      private covers(shown: { from: number; to: number }) {
        return this.covered !== null && this.covered[0] <= shown.from && shown.to <= this.covered[1];
      }

      private schedule(ms: number) {
        // Typing keeps pushing the request back; scrolling doesn't cut short a pause in typing.
        if (ms < PAUSE_MS && this.covered === null && this.timer !== undefined) return;
        clearTimeout(this.timer);
        this.timer = window.setTimeout(() => {
          this.timer = undefined;
          void this.run();
        }, ms);
      }

      private async run() {
        const doc = this.view.state.doc;
        const [from, to] = pieceToColour(doc, this.view.viewport, whole);
        const request = ++this.asked;
        let colours: Colour[];
        try {
          colours = await ask(doc.sliceString(from, to));
        } catch {
          return; // no colours is never a problem worth reporting
        }
        // Typed or scrolled since: a newer request is on its way.
        if (this.gone || request !== this.asked || this.view.state.doc !== doc) return;
        this.covered = [from, to];
        this.view.dispatch({ effects: setColours.of(decorationsFor(colours, from, doc.length)) });
      }

      destroy() {
        this.gone = true;
        clearTimeout(this.timer);
      }
    },
  );
  return [coloursField, plugin];
}

/** For tests: the field that holds the colours, and the effect that replaces them. */
export const coloursForTests = coloursField;
export const setColoursForTests = setColours;
