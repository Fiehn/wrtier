import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { countsOf, markAt, marks, marksOf, setDismissed, setFound, type Mark } from "./marks";

const mark = (from: number, to: number, level: Mark["level"] = "rule", rule = "comma-before-tag"): Mark =>
  ({ mod: "dialogue", rule, level, from, to, why: "", suggestions: [{ label: "", edits: [{ from, to, insert: "," }] }] });

const start = (doc = '"Hi." she said. "Yes." he said.') =>
  EditorState.create({ doc, extensions: marks({ dismiss: () => {} }) });

describe("Marks in an editor", () => {
  it("are what the Mods found, in reading order, counted by kind", () => {
    const s = start().update({ effects: setFound.of([mark(20, 21, "query"), mark(3, 4)]) }).state;
    expect(marksOf(s).map((m) => m.from)).toEqual([3, 20]);
    expect(countsOf(s)).toEqual({ mistake: 0, rule: 1, query: 1 });
    expect(markAt(s, 3)?.from).toBe(3);
    expect(markAt(s, 10)).toBeNull();
  });

  it("move with text typed before them, and an edit touching one drops it until the Mods look again", () => {
    let s = start().update({ effects: setFound.of([mark(3, 4), mark(20, 21)]) }).state;
    s = s.update({ changes: { from: 0, insert: "So: " } }).state;
    expect(marksOf(s).map((m) => [m.from, m.to])).toEqual([[7, 8], [24, 25]]);
    expect(marksOf(s)[0].suggestions[0].edits[0]).toEqual({ from: 7, to: 8, insert: "," });
    s = s.update({ changes: { from: 7, to: 8, insert: "," } }).state;
    expect(marksOf(s).map((m) => m.from)).toEqual([24]);
  });

  it("leave out those dismissed, on the same characters for the same rule", () => {
    let s = start().update({ effects: setFound.of([mark(3, 4), mark(20, 21), mark(3, 4, "query", "speaker-change")]) }).state;
    s = s.update({ effects: setDismissed.of([{ key: "dialogue/comma-before-tag", ranges: [[3, 4]] }]) }).state;
    expect(marksOf(s).map((m) => `${m.rule}@${m.from}`)).toEqual(["speaker-change@3", "comma-before-tag@20"]);
    // The Mods look again: still dismissed.
    s = s.update({ effects: setFound.of([mark(3, 4), mark(20, 21)]) }).state;
    expect(marksOf(s).map((m) => m.from)).toEqual([20]);
    // Text typed before: the dismissal moves with its characters.
    s = s.update({ changes: { from: 0, insert: "So: " } }).state;
    s = s.update({ effects: setFound.of([mark(7, 8), mark(24, 25)]) }).state;
    expect(marksOf(s).map((m) => m.from)).toEqual([24]);
  });
});
