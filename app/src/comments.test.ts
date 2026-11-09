import { describe, expect, it } from "vitest";
import { ChangeSet, EditorState } from "@codemirror/state";
import { marksForTests, setMarks } from "./comments";
import { fitMarks, type Mark } from "./sync";

const mark = (id: string, ranges: [number, number][], resolved = false): Mark => ({
  id, ranges, sender: "author", name: "Author", colour: "author", body: "too abrupt?", resolved,
  quote: "bcd", timeMs: 0, replies: [], otherBody: null,
});

/** Where each highlight is now, by Comment. */
function highlighted(state: EditorState): Record<string, [number, number][]> {
  const { byId, deco } = state.field(marksForTests);
  const out: Record<string, [number, number][]> = {};
  for (const id of byId.keys()) out[id] = [];
  deco.between(0, state.doc.length, (from, to, value) => {
    out[value.spec.comment as string].push([from, to]);
  });
  return out;
}

function typeAt(state: EditorState, at: number, text: string) {
  return state.update({ changes: { from: at, insert: text } }).state;
}

describe("Comment highlights in the editor", () => {
  // "abcdef", with a Comment on "bcd".
  const start = () => EditorState.create({ doc: "abcdef", extensions: [marksForTests] })
    .update({ effects: setMarks.of([mark("c1", [[1, 4]])]) }).state;

  it("grow when text is typed strictly inside", () => {
    expect(highlighted(typeAt(start(), 2, "XY"))).toEqual({ c1: [[1, 6]] });
  });

  it("don't grow at their edges", () => {
    let s = typeAt(start(), 4, "!"); // right after "d"
    s = typeAt(s, 1, "<"); // right before "b"
    expect(s.doc.toString()).toBe("a<bcd!ef");
    expect(highlighted(s)).toEqual({ c1: [[2, 5]] });
  });

  it("go when all their text is deleted, and come back with Rust's next word", () => {
    let s = start().update({ changes: { from: 1, to: 4 } }).state;
    expect(highlighted(s)).toEqual({ c1: [] });
    s = s.update({ effects: setMarks.of([mark("c1", [])]) }).state; // detached
    s = s.update({ changes: { from: 1, insert: "bcd" } }).state; // a Revert brings the text back...
    s = s.update({ effects: setMarks.of([mark("c1", [[1, 4]])]) }).state; // ...and Rust says it's attached again
    expect(highlighted(s)).toEqual({ c1: [[1, 4]] });
  });

  it("overlap and split, as Rust gives them", () => {
    const s = start().update({ effects: setMarks.of([mark("c1", [[1, 2], [4, 6]]), mark("c2", [[0, 5]])]) }).state;
    expect(highlighted(s)).toEqual({ c1: [[1, 2], [4, 6]], c2: [[0, 5]] });
  });
});

describe("fitting Rust's highlights around what was typed since", () => {
  it("moves them along with text typed before them", () => {
    const typed = ChangeSet.of({ from: 0, insert: "XY" }, 6);
    expect(fitMarks([mark("c1", [[1, 4]])], [typed])[0].ranges).toEqual([[3, 6]]);
  });

  it("grows them only for text typed strictly inside", () => {
    const inside = ChangeSet.of({ from: 2, insert: "X" }, 6);
    const atEnd = ChangeSet.of({ from: 4, insert: "!" }, 6);
    const atStart = ChangeSet.of({ from: 1, insert: "<" }, 6);
    expect(fitMarks([mark("c1", [[1, 4]])], [inside])[0].ranges).toEqual([[1, 5]]);
    expect(fitMarks([mark("c1", [[1, 4]])], [atEnd])[0].ranges).toEqual([[1, 4]]);
    expect(fitMarks([mark("c1", [[1, 4]])], [atStart])[0].ranges).toEqual([[2, 5]]);
  });

  it("drops a range whose text was deleted since, and goes through several edits in order", () => {
    const deleted = ChangeSet.of({ from: 1, to: 4 }, 6);
    expect(fitMarks([mark("c1", [[1, 4]])], [deleted])[0].ranges).toEqual([]);
    const first = ChangeSet.of({ from: 0, insert: "X" }, 6); // "Xabcdef"
    const second = ChangeSet.of({ from: 3, insert: "Y" }, 7); // "XabYcdef": inside
    expect(fitMarks([mark("c1", [[1, 4]])], [first, second])[0].ranges).toEqual([[2, 6]]);
  });
});
