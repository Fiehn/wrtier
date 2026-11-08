import { describe, expect, it } from "vitest";
import { Text } from "@codemirror/state";
import { callAt, makesSceneBreak, matchingBracket, type ElementInfo } from "./elements";

const doc = (s: string) => Text.of(s.split("\n"));
const elements: ElementInfo[] = [
  { name: "epigraph", title: "Epigraph", shownAs: "block", inWork: true },
  { name: "scene-break", title: "Scene break", shownAs: "chip", inWork: true },
  { name: "dream", title: "Dream", shownAs: "block", inWork: false },
];

describe("Story elements in the editor", () => {
  it("finds the bracket that ends a body, past nested and escaped ones", () => {
    const text = doc("#epigraph[A [nested] \\] one\n\non two lines.] after");
    const at = matchingBracket(text, "#epigraph[".length);
    expect(text.sliceString(at, at + 7)).toBe("] after");
    expect(matchingBracket(doc("#epigraph[not closed"), 10)).toBe(-1);
  });

  it("knows a call by its head", () => {
    const text = doc("#epigraph[Long.] and #scene-break() and #dream[x] and #text[y]");
    const head = (s: string) => {
      const from = text.toString().indexOf(s);
      return callAt(text, from, from + s.length, elements);
    };
    expect(head("#epigraph[")).toMatchObject({ info: { name: "epigraph" }, close: [15, 16] });
    expect(head("#scene-break()")).toMatchObject({ info: { name: "scene-break" }, close: null });
    expect(head("#dream[")?.info.inWork).toBe(false); // the "Add it?" chip
    expect(head("#text[")).toBeNull(); // Typst's own: not a Story element
  });

  it("turns *** on an empty line into a scene break, and nothing else", () => {
    expect(makesSceneBreak(doc("One.\n**"), 7, 7, "*")).toBe(true);
    expect(makesSceneBreak(doc("One.\n**"), 6, 6, "*")).toBe(false); // not at the end of the line
    expect(makesSceneBreak(doc("One **"), 6, 6, "*")).toBe(false); // not alone on its line
    expect(makesSceneBreak(doc("**"), 2, 2, "x")).toBe(false);
  });
});
