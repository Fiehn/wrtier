import { describe, expect, it } from "vitest";
import { Compartment, EditorState } from "@codemirror/state";
import { bracketsForTests, spansForTests } from "./formatted";
import { parseMarkup } from "./markup";
import { elementsFacet, type ElementInfo } from "./elements";
import { fromHistory } from "./sync";

const elements: ElementInfo[] = [
  { name: "block-text", title: "Block text", shownAs: "block", inWork: true },
  { name: "epigraph", title: "Epigraph", shownAs: "block", inWork: true },
];

function editor(doc: string) {
  return EditorState.create({ doc, extensions: [elementsFacet.of(elements), ...bracketsForTests] });
}

/** Deletes `from`..`to` as the Author would (Backspace, Delete, or a selection). */
function del(state: EditorState, from: number, to: number, userEvent = "delete.backward") {
  return state.update({ changes: { from, to }, selection: { anchor: from }, userEvent }).state;
}

describe("a Story element's brackets in the Formatted view", () => {
  it("can't be deleted on their own", () => {
    let s = editor("#block-text[Dear Anna,]");
    s = del(s, 11, 12); // Backspace at the start of its text
    expect(s.doc.toString()).toBe("#block-text[Dear Anna,]");
    s = del(s, 21, 23, "delete.selection"); // ",]" selected
    expect(s.doc.toString()).toBe("#block-text[Dear Anna]");
  });

  it("go with the command when it's deleted, at once or letter by letter", () => {
    expect(del(editor("A #block-text[Dear Anna] B"), 2, 13, "delete.selection").doc.toString()).toBe("A Dear Anna B");
    let s = editor("#epigraph[Long.]");
    for (let at = 9; at > 0; at--) s = del(s, at - 1, at); // backspacing the command away
    expect(s.doc.toString()).toBe("Long.");
  });

  it("stay while the command is only renamed", () => {
    let s = editor("#epigraph[Long.]");
    s = s.update({ changes: { from: 1, to: 9, insert: "block-text" }, userEvent: "input.type" }).state;
    expect(s.doc.toString()).toBe("#block-text[Long.]");
  });

  it("leave typing over a selection alone, apart from the bracket", () => {
    let s = editor("#epigraph[Long.] After.");
    // ". Aft" and the hidden "]" between them, typed over with "!":
    s = s.update({ changes: { from: 14, to: 20, insert: "!" }, userEvent: "input.type" }).state;
    expect(s.doc.toString()).toBe("#epigraph[Long!]er.");
  });

  it("survive switching to the Raw view and back, again and again", () => {
    const view = new Compartment();
    let s = EditorState.create({ doc: "#epigraph[Long.]", extensions: [elementsFacet.of(elements), view.of(bracketsForTests)] });
    for (let i = 0; i < 3; i++) {
      s = s.update({ effects: view.reconfigure([]) }).state; // the Raw view
      s = s.update({ effects: view.reconfigure(bracketsForTests) }).state; // the Formatted view again
    }
    s = del(s, 9, 10); // and its brackets are still kept
    expect(s.doc.toString()).toBe("#epigraph[Long.]");
  });

  it("never change what comes from the History", () => {
    let s = editor("#epigraph[Long.]");
    s = s.update({ changes: { from: 15, to: 16 }, annotations: fromHistory.of(true) }).state;
    expect(s.doc.toString()).toBe("#epigraph[Long.");
  });
});

describe("the Formatted view's spans", () => {
  it("parsing only the edited paragraph always gives what parsing everything gives", () => {
    // A fixed-seed run of random edits: typing markup, deleting, joining and splitting paragraphs.
    const pieces = ["*", "_", "`", "= ", "\n", "\n\n", "#quote[", "]", "--", "word ", "\\", "// note", "😊", "$", "<a>", "@a "];
    let state = EditorState.create({ doc: "= One\nIt was *dark*.\n\n_Two_ -- #quote[three]\n\nfour `raw`", extensions: [spansForTests] });
    let seed = 7;
    const next = (n: number) => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed % Math.max(n, 1);
    };
    for (let step = 0; step < 400; step++) {
      const len = state.doc.length;
      const from = next(len + 1);
      const to = Math.min(len, from + (next(3) === 0 ? next(6) : 0));
      const insert = next(4) === 0 ? "" : pieces[next(pieces.length)];
      state = state.update({ changes: { from, to, insert } }).state;
      expect(state.field(spansForTests), `after step ${step}: ${JSON.stringify(state.doc.toString())}`)
        .toEqual(parseMarkup(state.doc.toString()));
    }
  });
});
