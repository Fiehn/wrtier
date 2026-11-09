// "Colour text by Session": the text the newest Sessions wrote gets a faint tint in the
// editor, the newest strongest, with "Laptop · today 14:02" on hover on desktop.
//
// The tints come from Rust (sync.ts asks for them whenever the History grows, but only while
// the switch is on) and, in between, move along with the typing like the Comment highlights.
// Text typed since shows untinted until Rust's next answer (when the burst is recorded).

import { StateEffect, StateField, RangeSetBuilder, type Extension } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, hoverTooltip } from "@codemirror/view";
import { dayOf } from "./historypanel";
import type { Tint } from "./sync";

/** New tints from Rust, fitted to the editor's text; an empty list clears them. */
export const setTints = StateEffect.define<Tint[]>();

/** "Laptop · today 14:02", or "Laptop · Mon 2 Nov 14:02" for an older one. */
export function tintLabel(device: string, timeMs: number, now = Date.now()): string {
  const day = dayOf(timeMs, now);
  const shown = day === "Today" || day === "Yesterday" ? day.toLowerCase() : day;
  const time = new Date(timeMs).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `${device} · ${shown} ${time}`;
}

function build(tints: Tint[]): DecorationSet {
  const all: { from: number; to: number; deco: Decoration }[] = [];
  for (const tint of tints) {
    const deco = Decoration.mark({ class: `cm-tint cm-tint-${Math.min(tint.rank, 4)}`, label: tintLabel(tint.device, tint.timeMs) });
    for (const [from, to] of tint.ranges) all.push({ from, to, deco });
  }
  all.sort((a, b) => a.from - b.from || a.to - b.to);
  const builder = new RangeSetBuilder<Decoration>();
  for (const { from, to, deco } of all) builder.add(from, to, deco);
  return builder.finish();
}

const tintsField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(setTints)) return build(effect.value);
    return tr.docChanged ? value.map(tr.changes) : value;
  },
  provide: (field) => EditorView.decorations.from(field),
});

/** Which Session wrote the character at `pos`, as its label (null: none of the newest). */
export function tintAt(state: EditorView["state"], pos: number): string | null {
  let found: string | null = null;
  state.field(tintsField).between(pos, pos + 1, (from, to, value) => {
    if (from <= pos && pos < to) found = value.spec.label as string;
  });
  return found;
}

/** The editor's part: the tints, and on desktop who wrote the text under the mouse. */
export function sessionTints(): Extension {
  const desktop = window.matchMedia("(hover: hover)").matches;
  const tips = hoverTooltip((view, pos, side) => {
    const at = side < 0 ? pos - 1 : pos;
    const label = tintAt(view.state, at);
    if (label === null) return null;
    return {
      pos: at,
      create: () => {
        const dom = document.createElement("div");
        dom.className = "cm-tint-tooltip";
        dom.textContent = label;
        return { dom };
      },
    };
  }, { hoverTime: 600 });
  return [tintsField, desktop ? tips : []];
}

/** For tests: the field that holds the tints. */
export const tintsForTests = tintsField;
