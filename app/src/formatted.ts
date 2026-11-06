// The Formatted view: Typst markup hidden and shown as formatting, except where the cursor is.
// `*` and `_` show again while the cursor is inside that stretch of text; a heading's `=` signs
// while the cursor is on its line. Only how the text looks changes: the text, its positions and
// everything sent to the History stay exactly as in the Raw view.

import { type Extension, type Text, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, hoverTooltip, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import { parseMarkup, type Span } from "./markup";

/** Where the paragraph around `from`..`to` starts and ends (blank lines separate paragraphs). */
export function paragraphAround(doc: Text, from: number, to: number): [number, number] {
  let start = doc.lineAt(from);
  while (start.number > 1) {
    const before = doc.line(start.number - 1);
    if (before.text.trim() === "") break;
    start = before;
  }
  let end = doc.lineAt(to);
  while (end.number < doc.lines) {
    const after = doc.line(end.number + 1);
    if (after.text.trim() === "") break;
    end = after;
  }
  return [start.from, end.to];
}

/** A span's positions moved along with an edit elsewhere in the text. */
function mapSpan(span: Span, map: (pos: number) => number): Span {
  const moved = { ...span, from: map(span.from), to: map(span.to) } as Span;
  if ("mark" in moved) moved.mark = [map(moved.mark[0]), map(moved.mark[1])];
  if ("open" in moved) {
    moved.open = [map(moved.open[0]), map(moved.open[1])];
    moved.close = [map(moved.close[0]), map(moved.close[1])];
  }
  return moved;
}

/** The markup in the whole text, kept up to date by parsing again only the paragraphs an edit touched. */
const spansField = StateField.define<Span[]>({
  create: (state) => parseMarkup(state.doc.toString()),
  update(spans, tr) {
    if (!tr.docChanged) return spans;
    let from = Infinity;
    let to = -Infinity;
    tr.changes.iterChangedRanges((_fa, _ta, fromB, toB) => {
      from = Math.min(from, fromB);
      to = Math.max(to, toB);
    });
    const doc = tr.state.doc;
    const [start, end] = paragraphAround(doc, from, to);
    const kept = spans
      .map((s) => mapSpan(s, (pos) => tr.changes.mapPos(pos)))
      .filter((s) => s.to < start || s.from > end);
    const fresh = parseMarkup(doc.sliceString(start, end), start);
    return [...kept, ...fresh].sort((a, b) => a.from - b.from || b.to - a.to);
  },
});

/** Shows a dash or an ellipsis in place of `--`, `---` or `...`. */
class Glyph extends WidgetType {
  constructor(readonly glyph: string) {
    super();
  }
  eq(other: Glyph) {
    return other.glyph === this.glyph;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "cm-tp-glyph";
    el.textContent = this.glyph;
    return el;
  }
}

const hide = Decoration.replace({});
const mark = (cls: string) => Decoration.mark({ class: cls });
const styles = {
  strong: mark("cm-tp-strong"),
  emph: mark("cm-tp-emph"),
  raw: mark("cm-tp-raw"),
  comment: mark("cm-tp-comment"),
  code: mark("cm-tp-code"),
  label: mark("cm-tp-label"),
  math: mark("cm-tp-math"),
  link: mark("cm-tp-link"),
  list: mark("cm-tp-list"),
  linebreak: mark("cm-tp-linebreak"),
  delimiter: mark("cm-tp-delimiter"),
};
const headingLines = [1, 2, 3, 4, 5, 6].map((level) => Decoration.line({ class: `cm-tp-h cm-tp-h${level}` }));

/** The decorations for what's on screen, given where the cursor and selections are. */
function decorate(view: EditorView): DecorationSet {
  const state = view.state;
  const spans = state.field(spansField);
  const ranges = state.selection.ranges;
  const touched = (from: number, to: number) => ranges.some((r) => r.from <= to && r.to >= from);
  const onLine = (pos: number) => {
    const line = state.doc.lineAt(pos);
    return ranges.some((r) => r.from <= line.to && r.to >= line.from);
  };
  const decos = [];
  for (const { from: top, to: bottom } of view.visibleRanges) {
    for (const span of spans) {
      if (span.to < top || span.from > bottom) continue;
      const near = touched(span.from, span.to);
      switch (span.kind) {
        case "heading": {
          decos.push(headingLines[Math.min(span.level, 6) - 1].range(state.doc.lineAt(span.from).from));
          if (span.mark[1] > span.mark[0]) {
            decos.push((onLine(span.from) ? styles.delimiter : hide).range(span.mark[0], span.mark[1]));
          }
          break;
        }
        case "strong":
        case "emph":
        case "raw": {
          if (span.close[0] > span.open[1]) decos.push(styles[span.kind].range(span.open[1], span.close[0]));
          for (const [a, b] of [span.open, span.close]) {
            if (b > a) decos.push((near ? styles.delimiter : hide).range(a, b));
          }
          break;
        }
        case "escape":
          if (!near) decos.push(hide.range(span.from, span.from + 1));
          break;
        case "glyph":
          if (!near) decos.push(Decoration.replace({ widget: new Glyph(span.glyph) }).range(span.from, span.to));
          break;
        default:
          if (span.to > span.from) decos.push(styles[span.kind].range(span.from, span.to));
      }
    }
  }
  return Decoration.set(decos, true);
}

/** Asks for the decorations to be worked out again (when the pointer is let go). */
const refresh = StateEffect.define<null>();

const formatting = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    /**
     * Is a mouse button (or a finger) down in the text? Then what's shown and hidden stays as
     * it is: markers appearing next to a selection that's being dragged would move the text
     * under the pointer, and the selection would end up somewhere other than meant.
     */
    pressed = false;
    private readonly down = (e: PointerEvent) => {
      if (e.isPrimary && e.button === 0) this.pressed = true;
    };
    private readonly up = () => {
      if (!this.pressed) return;
      this.pressed = false;
      this.view.dispatch({ effects: refresh.of(null) });
    };

    constructor(private readonly view: EditorView) {
      this.decorations = decorate(view);
      view.contentDOM.addEventListener("pointerdown", this.down);
      window.addEventListener("pointerup", this.up);
      window.addEventListener("pointercancel", this.up);
    }

    update(update: ViewUpdate) {
      const refreshed = update.transactions.some((tr) => tr.effects.some((e) => e.is(refresh)));
      if (update.docChanged || update.viewportChanged || refreshed || (update.selectionSet && !this.pressed)) {
        this.decorations = decorate(update.view);
      }
    }

    destroy() {
      this.view.contentDOM.removeEventListener("pointerdown", this.down);
      window.removeEventListener("pointerup", this.up);
      window.removeEventListener("pointercancel", this.up);
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

const explained: Partial<Record<Span["kind"], string>> = {
  heading: "Heading: written with = at the start of the line (== for a level 2 heading, and so on).",
  strong: "Strong: written between *stars* (Ctrl+B).",
  emph: "Emphasis: written between _underscores_ (Ctrl+I).",
  raw: "Raw text, shown as typed: written between `backticks`.",
  escape: "A character written with a backslash before it, so Typst doesn't read it as markup.",
  glyph: "Typed as -- (en dash), --- (em dash) or ... (ellipsis).",
  linebreak: "A line break: a backslash at the end of a line.",
  code: "Typst code, used as typed.",
  label: "A label or a reference to one.",
  math: "Math, written between $dollar signs$.",
};

/** On desktop, hovering over formatting says how it's written. */
const tooltips = hoverTooltip((view, pos) => {
  const spans = view.state.field(spansField);
  const span = [...spans].reverse().find((s) => s.from <= pos && pos <= s.to && explained[s.kind]);
  if (!span) return null;
  return {
    pos: span.from,
    end: span.to,
    above: true,
    create: () => {
      const dom = document.createElement("div");
      dom.className = "cm-tp-tooltip";
      dom.textContent = explained[span.kind]!;
      return { dom };
    },
  };
}, { hoverTime: 600 });

/** The Formatted view, as an editor extension (the Raw view is simply without it). */
export function formattedView(): Extension {
  const desktop = window.matchMedia("(hover: hover)").matches;
  return [spansField, formatting, desktop ? tooltips : []];
}

/** For tests: the field that holds the spans. */
export const spansForTests = spansField;
