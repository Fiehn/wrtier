import { describe, expect, it } from "vitest";
import { chaptersIn } from "./chapters";
import { readingOfText, type Query } from "./find";
import { INCLUSION } from "./inclusions";
import { matchesInWork, snippetAt, summary } from "./findinvault";
import type { ElementInfo } from "./elements";

const elements: ElementInfo[] = [{ name: "scene-break", title: "Scene break", shownAs: "chip", inWork: true }];
const q = (text: string, more: Partial<Query> = {}): Query => ({ text, matchCase: false, wholeWords: false, ...more });

/** The matches in a text: what each covers as typed, and its Chapter. */
function found(text: string, query: Query) {
  const reading = readingOfText(text, elements);
  return matchesInWork(reading, chaptersIn(text), query).map((m) => [text.slice(m.from, m.to), m.chapter]);
}

describe("finding in a Work's text", () => {
  it("reads it as the Formatted view does, past hidden markup", () => {
    const text = "= One\n\nThe *lighthouse* keeper -- old by then.\n";
    expect(found(text, q("lighthouse keeper"))).toEqual([["lighthouse* keeper", "One"]]);
    expect(found(text, q("keeper – old"))).toEqual([["keeper -- old", "One"]]);
  });

  it("names each match's Chapter, and none before the first", () => {
    const text = "A lighthouse first.\n\n= One\n\nThe lighthouse.\n\n= _Two_\n\nAnother lighthouse.\n";
    expect(found(text, q("lighthouse"))).toEqual([["lighthouse", null], ["lighthouse", "One"], ["lighthouse", "Two"]]);
  });

  it("has Find's options", () => {
    const text = "Lighthouses, a lighthouse, the LIGHTHOUSE.";
    expect(found(text, q("lighthouse"))).toHaveLength(3);
    expect(found(text, q("lighthouse", { matchCase: true }))).toEqual([["lighthouse", null]]);
    expect(found(text, q("lighthouse", { wholeWords: true }))).toHaveLength(2);
  });

  it("finds only the Work's own text, not what an Inclusion shows", () => {
    // An Inclusion is its one character here; its text is found in its own Work.
    const text = `Before the lighthouse.\n\n${INCLUSION}\n\nAfter.\n`;
    expect(found(text, q("lighthouse"))).toEqual([["lighthouse", null]]);
    expect(found(text, q("before the lighthouse. after"))).toEqual([]);
  });

  it("never matches across a Story element", () => {
    expect(found("dark #scene-break() night", q("dark night"))).toEqual([]);
  });
});

describe("a match's snippet", () => {
  it("is the words around it, in its paragraph", () => {
    const reading = readingOfText("= One\n\nShe walked out to the lighthouse before dawn.\n\nNext.", elements);
    const at = reading.text.indexOf("lighthouse");
    expect(snippetAt(reading, at, at + 10)).toEqual({ before: "She walked out to the ", match: "lighthouse", after: " before dawn." });
  });

  it("is cut at a space, with … where it's cut", () => {
    const long = `${"word ".repeat(30)}lighthouse${" more".repeat(40)}`;
    const reading = readingOfText(long, elements);
    const at = reading.text.indexOf("lighthouse");
    const s = snippetAt(reading, at, at + 10, 20);
    expect(s.before.startsWith("…")).toBe(true);
    expect(s.before.endsWith("word ")).toBe(true);
    expect(s.after.endsWith("…")).toBe(true);
    expect(s.after.startsWith(" more")).toBe(true);
  });

  it("reads as the text does", () => {
    const reading = readingOfText("The *old* lighthouse", elements);
    const at = reading.text.indexOf("lighthouse");
    expect(snippetAt(reading, at, at + 10).before).toBe("The old ");
  });
});

describe("the summary line", () => {
  it("counts matches and Works", () => {
    expect(summary(0, 0)).toBe("No matches");
    expect(summary(1, 1)).toBe("1 match in 1 Work");
    expect(summary(6410, 4)).toBe("6,410 matches in 4 Works");
  });
});
