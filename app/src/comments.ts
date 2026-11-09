// Comments in the editor: each Comment's text highlighted in its sender's colour, in the
// Formatted and the Raw view alike.
//
// The highlights come from Rust (sync.ts asks for them whenever the History changed) and, in
// between, move along with the typing: one grows when text is typed strictly inside it, never at
// its edges, the same rule the History follows. Overlapping highlights stack, since each is a
// see-through colour. Hidden ones (resolved, or all of them by the setting) stay in the editor's
// state, so they're right the moment they're shown again.

import { StateEffect, StateField, RangeSetBuilder, type Extension } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, hoverTooltip } from "@codemirror/view";
import type { Mark } from "./sync";

/** New highlights from Rust, fitted to the editor's text. */
export const setMarks = StateEffect.define<Mark[]>();

/** What's shown: the highlights at all (a Vault setting), and resolved Comments' (Device state). */
export type CommentOptions = { highlights: boolean; resolved: boolean };

type Marks = {
  /** Every Comment's details, by ID (their ranges are in `deco`, which follows the typing). */
  byId: Map<string, Mark>;
  deco: DecorationSet;
};

function build(marks: Mark[]): Marks {
  const all: { from: number; to: number; deco: Decoration }[] = [];
  for (const mark of marks) {
    const deco = Decoration.mark({
      class: `cm-comment cm-comment-${mark.colour}${mark.resolved ? " cm-comment-resolved" : ""}`,
      attributes: { "data-comment": mark.id },
      comment: mark.id,
    });
    for (const [from, to] of mark.ranges) all.push({ from, to, deco });
  }
  all.sort((a, b) => a.from - b.from || a.to - b.to);
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to, deco } of all) builder.add(from, to, deco);
  return { byId: new Map(marks.map((m) => [m.id, m])), deco: builder.finish() };
}

const marksField = StateField.define<Marks>({
  create: () => ({ byId: new Map(), deco: Decoration.none }),
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(setMarks)) return build(effect.value);
    return tr.docChanged ? { byId: value.byId, deco: value.deco.map(tr.changes) } : value;
  },
  provide: (field) => EditorView.decorations.from(field, (value) => value.deco),
});

/** Where a Comment's text is now (it follows the typing between answers from Rust). */
export function rangesOf(view: EditorView, id: string): [number, number][] {
  const ranges: [number, number][] = [];
  view.state.field(marksField).deco.between(0, view.state.doc.length, (from, to, value) => {
    if (value.spec.comment === id) ranges.push([from, to]);
  });
  return ranges;
}

/** The text a new Comment will be on, outlined while it's being written (null: none). */
export const setPending = StateEffect.define<{ from: number; to: number } | null>();

const pendingMark = Decoration.mark({ class: "cm-comment-pending" });

const pendingField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setPending)) {
        const range = effect.value;
        return range && range.from < range.to ? Decoration.set([pendingMark.range(range.from, range.to)]) : Decoration.none;
      }
    }
    return value.map(tr.changes);
  },
  provide: (field) => EditorView.decorations.from(field),
});

/** Where the new Comment's text is now, if it's still there. */
export function pendingRange(view: EditorView): { from: number; to: number } | null {
  let found: { from: number; to: number } | null = null;
  view.state.field(pendingField).between(0, view.state.doc.length, (from, to) => {
    found = { from, to };
  });
  return found;
}

/** The Comments whose highlight is at `pos` and showing, in the order their text starts. */
export function commentsAt(view: EditorView, pos: number, options: CommentOptions): Mark[] {
  if (!options.highlights) return [];
  const { byId, deco } = view.state.field(marksField);
  const found: Mark[] = [];
  deco.between(pos, pos, (from, to, value) => {
    const mark = byId.get(value.spec.comment as string);
    // Strictly inside, or at the start (the character after `pos` is in it).
    if (mark && from <= pos && pos < to && (options.resolved || !mark.resolved) && !found.includes(mark)) found.push(mark);
  });
  return found;
}

/** The first line or so of a Comment's text, for a tooltip. */
function preview(body: string): string {
  const line = body.trim().split("\n")[0];
  return line.length > 140 ? `${line.slice(0, 139)}…` : line;
}

/**
 * The editor's part of Comments. `options` is read each time (it changes with the settings);
 * `clicked` gets the Comments under a plain click (the panel opens on them).
 */
export function commentHighlights(options: () => CommentOptions, clicked: (ids: string[]) => void): Extension {
  const desktop = window.matchMedia("(hover: hover)").matches;
  const tips = hoverTooltip((view, pos, side) => {
    // The character the mouse is over: after `pos` on its right half, before it on its left.
    const at = side < 0 ? pos - 1 : pos;
    const marks = commentsAt(view, at, options());
    if (marks.length === 0) return null;
    return {
      pos: at,
      above: true,
      create: () => {
        const dom = document.createElement("div");
        dom.className = "cm-comment-tooltip";
        for (const mark of marks) {
          const row = document.createElement("div");
          const who = document.createElement("strong");
          who.textContent = mark.name;
          who.className = `comment-sender comment-sender-${mark.colour}`;
          row.append(who, document.createTextNode(` ${preview(mark.body)}`));
          dom.append(row);
        }
        return { dom };
      },
    };
  }, { hoverTime: 400 });

  return [
    marksField,
    pendingField,
    desktop ? tips : [],
    EditorView.domEventHandlers({
      // A plain click (not the end of a selection) on highlighted text.
      click(event, view) {
        if (event.button !== 0 || event.shiftKey || !view.state.selection.main.empty) return false;
        const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
        if (pos === null) return false;
        const ids = commentsAt(view, pos, options()).map((m) => m.id);
        if (ids.length > 0) clicked(ids);
        return false; // the cursor still goes where the click was
      },
    }),
  ];
}

/** Classes on the editor that hide highlights (they stay in its state, only unpainted). */
export function commentClasses(options: CommentOptions): Extension {
  const classes = [options.highlights ? "" : "comments-hidden", options.resolved ? "resolved-shown" : ""].filter(Boolean).join(" ");
  return EditorView.editorAttributes.of(classes ? { class: classes } : {});
}

/** For tests: the field that holds the highlights. */
export const marksForTests = marksField;
