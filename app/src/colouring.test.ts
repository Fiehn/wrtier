import { describe, expect, it, vi } from "vitest";
import { EditorState, Text } from "@codemirror/state";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { coloursForTests, decorationsFor, pieceToColour, setColoursForTests } from "./colouring";

/** The coloured stretches in `state`, as [text, class]. */
function shown(state: EditorState): [string, string][] {
  const found: [string, string][] = [];
  state.field(coloursForTests).between(0, state.doc.length, (from, to, value) => {
    found.push([state.doc.sliceString(from, to), value.spec.class as string]);
  });
  return found;
}

describe("Typst colouring in the editor", () => {
  it("places a piece's colours where the piece sits in the text", () => {
    const doc = "First paragraph.\n\n#set page(width: 148mm)\n";
    const offset = doc.indexOf("#set");
    let state = EditorState.create({ doc, extensions: [coloursForTests] });
    // As Rust answers for "#set page(width: 148mm)\n": ranges counted from the piece's start.
    const deco = decorationsFor([{ from: 1, to: 4, class: "typ-key" }, { from: 5, to: 9, class: "typ-func" }], offset, doc.length);
    state = state.update({ effects: setColoursForTests.of(deco) }).state;
    expect(shown(state)).toEqual([["set", "typ-key"], ["page", "typ-func"]]);
  });

  it("moves the colours along with typing until the next answer", () => {
    let state = EditorState.create({ doc: "#set page()", extensions: [coloursForTests] });
    state = state.update({ effects: setColoursForTests.of(decorationsFor([{ from: 5, to: 9, class: "typ-func" }], 0, 11)) }).state;
    state = state.update({ changes: { from: 0, insert: "// x\n" } }).state;
    expect(shown(state)).toEqual([["page", "typ-func"]]);
  });

  it("drops an answer's ranges that no longer fit the text, and empty ones", () => {
    const deco = decorationsFor([{ from: 0, to: 0, class: "typ-key" }, { from: 2, to: 50, class: "typ-str" }, { from: 0, to: 2, class: "typ-num" }], 0, 10);
    expect(deco.size).toBe(1);
  });

  it("keeps nested colours (a string inside a heading)", () => {
    const doc = 'Hi = "x"';
    let state = EditorState.create({ doc, extensions: [coloursForTests] });
    const deco = decorationsFor([{ from: 0, to: 8, class: "typ-heading" }, { from: 5, to: 8, class: "typ-str" }], 0, doc.length);
    state = state.update({ effects: setColoursForTests.of(deco) }).state;
    expect(shown(state)).toEqual([[doc, "typ-heading"], ['"x"', "typ-str"]]);
  });

  it("colours a library file whole, and only the shown paragraphs of a Work", () => {
    const doc = Text.of(["One.", "", "Two a,", "two b.", "", "Three."]);
    expect(pieceToColour(doc, { from: 8, to: 9 }, true)).toEqual([0, doc.length]);
    const [from, to] = pieceToColour(doc, { from: 8, to: 9 }, false);
    expect(doc.sliceString(from, to)).toBe("Two a,\ntwo b.");
    expect(pieceToColour(Text.of([""]), { from: 0, to: 0 }, false)).toEqual([0, 0]);
  });
});
