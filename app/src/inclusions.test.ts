import { describe, expect, it } from "vitest";
import { EditorState, Text } from "@codemirror/state";
import { INCLUSION, inclusions, inclusionsOf, placeInclusion, readerText, setInclusions, type InclusionView } from "./inclusions";
import { countsLine, countsOf } from "./counts";

const doc = (s: string) => Text.of(s.split("\n"));

const rain = (at: number): InclusionView => ({
  at, work: "Rain", label: "Rain", text: "The rain had stopped.", missing: null, language: "da", whole: true, starts: 0, comments: [],
});

function withInclusion(text: string, views: InclusionView[]) {
  const state = EditorState.create({ doc: text, extensions: [inclusions()] });
  return state.update({ effects: setInclusions.of(views) }).state;
}

describe("Inclusions in the editor", () => {
  it("go on a line of their own", () => {
    // An empty line: right there.
    expect(placeInclusion(doc("One.\n\nTwo."), 5)).toEqual({ from: 5, insert: INCLUSION, at: 5 });
    // The start of a line: before it, pushing the line down.
    expect(placeInclusion(doc("One.\nTwo."), 5)).toEqual({ from: 5, insert: INCLUSION + "\n", at: 5 });
    // Inside a line: after it.
    expect(placeInclusion(doc("One.\nTwo."), 2)).toEqual({ from: 4, insert: "\n" + INCLUSION, at: 5 });
  });

  it("count as the text a reader sees, with the included part apart", () => {
    const state = withInclusion(`My words.\n${INCLUSION}\nMore.`, [rain(10)]);
    const { text, included } = readerText(state);
    expect(text).toBe("My words.\nThe rain had stopped.\nMore.");
    expect(countsOf(text).words).toBe(7);
    expect(countsLine(countsOf(text), null, countsOf(included).words)).toBe("7 words (4 included) · 35 characters");
    // A selection that doesn't take the Inclusion counts only what it takes.
    expect(readerText(state, 0, 9).text).toBe("My words.");
  });

  it("follow typing elsewhere and go when their character is deleted", () => {
    let state = withInclusion(`A\n${INCLUSION}\nB`, [rain(2)]);
    state = state.update({ changes: { from: 0, insert: "Before. " } }).state;
    expect(inclusionsOf(state).map((v) => v.at)).toEqual([10]);
    state = state.update({ changes: { from: 10, to: 11 } }).state;
    expect(inclusionsOf(state)).toEqual([]);
  });

  it("ignore what Rust said about a place that isn't an Inclusion any more", () => {
    const state = withInclusion("No inclusion here.", [rain(3)]);
    expect(inclusionsOf(state)).toEqual([]);
  });
});
