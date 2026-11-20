import { describe, expect, it } from "vitest";
import { contextOf, groupMarks } from "./markspanel";
import type { Mark } from "./marks";

describe("the text around a Mark in the Marks panel", () => {
  it("is the whole line when it's short", () => {
    const text = "She said teh word.\nNext line.";
    expect(contextOf(text, 9, 12)).toEqual({ before: "She said ", marked: "teh", after: " word." });
  });

  it("is cut at a whole word each side, with … for what's left out", () => {
    const text = "One two three four five six seven teh eight nine ten eleven twelve";
    const from = text.indexOf("teh");
    const { before, marked, after } = contextOf(text, from, from + 3, 12);
    expect(marked).toBe("teh");
    expect(before).toBe("…six seven ");
    expect(after).toBe(" eight nine…");
  });

  it("stays within the Mark's line", () => {
    const text = "Above.\nteh start\nBelow.";
    const from = text.indexOf("teh");
    expect(contextOf(text, from, from + 3)).toEqual({ before: "", marked: "teh", after: " start" });
  });
});

describe("Marks grouped in the Marks panel", () => {
  const mark = (rule: string, from: number, to: number): Mark => ({ mod: "spellcheck", rule, level: "mistake", from, to, why: "", suggestions: [] });
  const text = "Kvothe met teh Kvothe";
  const textOf = (m: Mark) => text.slice(m.from, m.to);

  it("puts the same rule on the same text together, at its first place", () => {
    const marks = [mark("unknown", 0, 6), mark("unknown", 11, 14), mark("unknown", 15, 21)];
    expect(groupMarks(marks, textOf).map((g) => g.marks.map((m) => m.from))).toEqual([[0, 15], [11]]);
  });

  it("keeps different rules on the same text apart", () => {
    const marks = [mark("unknown", 0, 6), mark("name", 15, 21)];
    expect(groupMarks(marks, textOf)).toHaveLength(2);
  });
});
