// Ctrl+B and Ctrl+I: strong and emphasis, the way a word processor does them. They only type
// Typst's markup (`*…*`, `_…_`), so they work the same in the Formatted and the Raw view, and
// the words keep their History (the markers are inserted around them, not retyped).

import { type EditorState, type TransactionSpec } from "@codemirror/state";
import { type EditorView } from "@codemirror/view";
import { parseMarkup } from "./markup";
import { paragraphAround } from "./formatted";

const isWordChar = (c: string) => /[\p{L}\p{N}]/u.test(c);

/**
 * What Ctrl+B (`strong`) or Ctrl+I (`emph`) does here:
 * - inside text that already is that, the markers come off;
 * - with a selection, it's wrapped (spaces at its edges stay outside; a selection starting or
 *   ending inside a word takes the whole word, since Typst ignores `*` and `_` inside words);
 * - with nothing selected, a pair is typed with the cursor between, to type into.
 */
export function markupChange(state: EditorState, kind: "strong" | "emph"): TransactionSpec {
  const marker = kind === "strong" ? "*" : "_";
  const doc = state.doc;
  const range = state.selection.main;
  const [start, end] = paragraphAround(doc, range.from, range.to);
  const at = (pos: number) => doc.sliceString(pos, pos + 1);

  // Already that: the innermost such stretch around the cursor or selection loses its markers.
  const around = parseMarkup(doc.sliceString(start, end), start)
    .filter((s): s is Extract<typeof s, { open: [number, number] }> => s.kind === kind)
    .filter((s) => s.from <= range.from && range.to <= s.to)
    .sort((a, b) => a.to - a.from - (b.to - b.from))[0];
  if (around) {
    return {
      changes: [{ from: around.open[0], to: around.open[1] }, { from: around.close[0], to: around.close[1] }],
      userEvent: "input.format",
    };
  }

  let { from, to } = range;
  while (from < to && /\s/.test(at(from))) from++;
  while (to > from && /\s/.test(at(to - 1))) to--;
  if (from === to) {
    return { changes: { from: range.head, insert: marker + marker }, selection: { anchor: range.head + 1 }, userEvent: "input.format" };
  }
  while (from > start && isWordChar(at(from - 1)) && isWordChar(at(from))) from--;
  while (to < end && isWordChar(at(to - 1)) && isWordChar(at(to))) to++;
  return {
    changes: [{ from, insert: marker }, { from: to, insert: marker }],
    selection: { anchor: from + 1, head: to + 1 },
    userEvent: "input.format",
  };
}

/** The key's command for the editor. */
export const toggleMarkup = (kind: "strong" | "emph") => (view: EditorView) => {
  view.dispatch({ ...markupChange(view.state, kind), scrollIntoView: true });
  return true;
};
