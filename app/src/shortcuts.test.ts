import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState } from "@codemirror/state";
import { markupChange } from "./shortcuts";

/** `text` with the selection marked by [ and ] (or a | for a cursor), after Ctrl+B or Ctrl+I. */
function press(marked: string, kind: "strong" | "emph" = "strong") {
  const cursor = marked.indexOf("|");
  const [a, b] = cursor !== -1 ? [cursor, cursor] : [marked.indexOf("["), marked.indexOf("]") - 1];
  const doc = marked.replace(/[[\]|]/g, "");
  let state = EditorState.create({ doc, selection: EditorSelection.single(a, b) });
  state = state.update(markupChange(state, kind)).state;
  const sel = state.selection.main;
  const text = state.doc.toString();
  return sel.empty ? text.slice(0, sel.head) + "|" + text.slice(sel.head) : text.slice(0, sel.from) + "[" + text.slice(sel.from, sel.to) + "]" + text.slice(sel.to);
}

describe("Ctrl+B and Ctrl+I", () => {
  it("wrap the selection", () => {
    expect(press("It was [very] dark.")).toBe("It was *[very]* dark.");
    expect(press("It was [very] dark.", "emph")).toBe("It was _[very]_ dark.");
  });
  it("leave spaces at the selection's edges outside", () => {
    expect(press("It was[ very ]dark.")).toBe("It was *[very]* dark.");
  });
  it("take whole words when the selection starts or ends inside one", () => {
    expect(press("It was ve[ry da]rk.")).toBe("It was *[very dark]*.");
  });
  it("take the markers off text that already is strong", () => {
    expect(press("It was *[very]* dark.")).toBe("It was [very] dark.");
    expect(press("It was *ve|ry* dark.")).toBe("It was ve|ry dark.");
    expect(press("It was *[very]* dark.", "emph")).toBe("It was *_[very]_* dark.");
  });
  it("type a pair to type into, with nothing selected", () => {
    expect(press("It was | dark.")).toBe("It was *|* dark.");
  });
});
