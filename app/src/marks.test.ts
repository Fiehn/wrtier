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

  describe("dismissed exactly (the Spellchecker's)", () => {
    const spelling = (from: number, to: number, rule = "unknown"): Mark =>
      ({ mod: "spellcheck", rule, level: rule === "name" ? "query" : "mistake", from, to, why: "", suggestions: [], dismissAs: "word", dismissExact: true });
    // "olde" is 4..8, "inn" 9..12.
    const doc = "The olde inn.";
    const dismissed = () => setDismissed.of([{ key: "spellcheck/word", ranges: [[4, 8]], whole: true }]);

    it("stay dismissed on exactly those characters, whichever rule they show as, as text moves them along", () => {
      let s = start(doc).update({ effects: [setFound.of([spelling(4, 8), spelling(9, 12)]), dismissed()] }).state;
      expect(marksOf(s).map((m) => m.from)).toEqual([9]);
      // Only some of the same characters: not this dismissal.
      s = s.update({ effects: setFound.of([spelling(4, 7)]) }).state;
      expect(marksOf(s).map((m) => m.from)).toEqual([4]);
      s = s.update({ effects: setFound.of([spelling(4, 8, "name")]) }).state;
      expect(marksOf(s)).toEqual([]);
      s = s.update({ changes: { from: 0, insert: "So: " } }).state;
      s = s.update({ effects: setFound.of([spelling(8, 12)]) }).state;
      expect(marksOf(s)).toEqual([]);
    });

    it("come back once a letter is typed into the word or deleted from it", () => {
      let s = start(doc).update({ effects: dismissed() }).state;
      s = s.update({ changes: { from: 6, insert: "l" } }).state; // "ollde"
      s = s.update({ effects: setFound.of([spelling(4, 9)]) }).state;
      expect(marksOf(s).map((m) => [m.from, m.to])).toEqual([[4, 9]]);
      let t = start(doc).update({ effects: dismissed() }).state;
      t = t.update({ changes: { from: 6, to: 7 } }).state; // "ole"
      t = t.update({ effects: setFound.of([spelling(4, 7)]) }).state;
      expect(marksOf(t).map((m) => [m.from, m.to])).toEqual([[4, 7]]);
    });
  });
});
