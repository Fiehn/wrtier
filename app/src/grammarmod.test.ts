import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { levelOf, marksFrom, toSend, type Found } from "./grammar";
import { dialectOf, grammarChecker, OFF_BY_DEFAULT, ruleSetting, rulesOff, type Harper } from "./grammarmod";
import { markSources, type Mark } from "./marks";
import { noSettings, type ModContext, type ModSettings } from "./mods";
import { readPara } from "./prose";

/** A Harper that finds "a apple" (AnA) and "the the" (RepeatedWords), and counts what it's asked. */
function fakeHarper() {
  const asked: { paragraphs: string[]; off: string[] }[] = [];
  const harper: Harper = async (language, paragraphs, off) => {
    asked.push({ paragraphs, off });
    if (!["", "en"].includes(language.split("-")[0])) return null;
    return paragraphs.map((p) => {
      const found: Found[] = [];
      for (const m of p.matchAll(/\ba apple/g)) {
        if (!off.includes("AnA")) found.push({ from: m.index, to: m.index + 1, rule: "AnA", kind: "Miscellaneous", message: "Use “an” here.", suggestions: [{ kind: "replace", text: "an" }] });
      }
      for (const m of p.matchAll(/\bthe the\b/g)) {
        found.push({ from: m.index, to: m.index + 7, rule: "RepeatedWords", kind: "Repetition", message: "The word “the” is repeated.", suggestions: [{ kind: "replace", text: "the" }] });
      }
      return found;
    });
  };
  return { harper, asked };
}

async function marksOf(doc: string, harper: Harper, settings: Partial<ModSettings> = {}, language = "en-GB") {
  const ctx: ModContext = { language, settings: { ...noSettings(), ...settings }, setSetting: () => {} };
  const state = EditorState.create({ doc, extensions: grammarChecker(ctx, harper) });
  const sources = state.facet(markSources);
  const marks = sources.length ? await sources[0].find(state) : [];
  return { state, marks };
}

const brief = (marks: Mark[]) => marks.map((m) => `${m.level}:${m.rule}:${m.from}-${m.to}`);

/** The text after accepting a Mark's first Suggestion. */
const accepted = (state: EditorState, mark: Mark) => state.update({ changes: mark.suggestions[0].edits }).state.doc.toString();

describe("the Grammar checker's reading", () => {
  it("sends a passage on one line, with a space only where something left out sat between two words", () => {
    const src = "She saw *a* apple\nand a pear/* note */.\nThen `code` left.";
    const s = toSend(readPara(src, 0, src.length));
    expect(s.text).toBe("She saw a apple and a pear. Then left.");
    expect(s.at.filter((i) => i < 0)).toHaveLength(0);
    expect(s.at.every((i, n) => i < 0 || s.text[n] === (/\s/.test(s.para.text[i]) ? " " : s.para.text[i]))).toBe(true);
    const joined = "one#v(1em)two";
    expect(toSend(readPara(joined, 0, joined.length)).text).toBe("one two");
  });

  it("marks the text as typed, and a Suggestion changes only what differs, leaving markup be", () => {
    const src = "She saw *a* apple.";
    const s = toSend(readPara(src, 0, src.length));
    const [mark] = marksFrom(s, [{ from: 8, to: 9, rule: "AnA", kind: "Grammar", message: "m", suggestions: [{ kind: "replace", text: "an" }] }], "grammar");
    expect(src.slice(mark.from, mark.to)).toBe("a");
    const state = EditorState.create({ doc: src });
    expect(accepted(state, mark)).toBe("She saw *an* apple.");
    expect(mark.suggestions[0].label).toContain("an apple");
  });

  it("marks nothing Harper found across something left out, since accepting it would edit the code", () => {
    const src = "one#v(1em)two";
    const s = toSend(readPara(src, 0, src.length));
    expect(marksFrom(s, [{ from: 0, to: 7, rule: "R", kind: "Grammar", message: "m", suggestions: [] }], "grammar")).toEqual([]);
  });

  it("makes typos and repeated words Mistakes, grammar Rule breaks, and style Queries", () => {
    expect(["Typo", "Repetition", "Grammar", "Punctuation", "Style", "WordChoice", "Whatever"].map(levelOf))
      .toEqual(["mistake", "mistake", "rule", "rule", "query", "query", "query"]);
  });
});

describe("the Grammar checker", () => {
  it("marks what Harper finds, with Suggestions that put it right", async () => {
    const { harper } = fakeHarper();
    const { state, marks } = await marksOf("She bought a apple.\n\nI saw the the cat.", harper);
    expect(brief(marks)).toEqual(["query:AnA:11-12", "mistake:RepeatedWords:27-34"]);
    expect(accepted(state, marks[0])).toBe("She bought an apple.\n\nI saw the the cat.");
    expect(accepted(state, marks[1])).toBe("She bought a apple.\n\nI saw the cat.");
  });

  it("asks Harper only about passages it hasn't asked about before", async () => {
    const { harper, asked } = fakeHarper();
    const ctx: ModContext = { language: "en-GB", settings: noSettings(), setSetting: () => {} };
    const ext = grammarChecker(ctx, harper);
    const look = (doc: string) => {
      const state = EditorState.create({ doc, extensions: ext });
      return state.facet(markSources)[0].find(state);
    };
    await look("One.\n\nTwo.");
    await look("One.\n\nTwo.\n\nThree.");
    expect(asked.map((a) => a.paragraphs)).toEqual([["One.", "Two."], ["Three."]]);
  });

  it("sends the rules the Work switched off, and nothing at all for a Work not in English", async () => {
    const { harper, asked } = fakeHarper();
    const { marks } = await marksOf("She bought a apple.", harper, { work: { "grammar.rule.AnA": "off" } });
    expect(marks).toEqual([]);
    expect(asked[0].off).toEqual(["AnA", ...OFF_BY_DEFAULT].sort());
    const danish = await marksOf("She bought a apple.", harper, {}, "da");
    expect(danish.marks).toEqual([]);
    expect(asked).toHaveLength(1);
  });

  it("knows the rules switched off, and which kind of English a Work is checked as", () => {
    expect(rulesOff({ "grammar.rule.LongSentences": "off", "grammar.rule.AnA": "off", "grammar.rule.Dashes": "", "spellcheck.rule.name": "off" }))
      .toEqual(["AnA", ...OFF_BY_DEFAULT].sort());
    // Off by default until switched on.
    expect(rulesOff({})).toEqual([...OFF_BY_DEFAULT].sort());
    expect(rulesOff({ "grammar.rule.LongSentences": "on", "grammar.rule.CommaSplice": "off" })).toEqual(["BoringWords", "CommaSplice", "Excellent", "OxfordComma", "SentenceCapitalization"]);
    expect(["en-US", "en-gb", "en", "", "en-ZA", "en-AU"].map(dialectOf)).toEqual(["American", "British", "British", "British", "British", "Australian"]);
  });

  it("switches a rule by setting it only where that differs from its default", () => {
    expect(ruleSetting("AnA", false)).toEqual(["grammar.rule.AnA", "off"]);
    expect(ruleSetting("AnA", true)).toEqual(["grammar.rule.AnA", ""]);
    expect(ruleSetting("LongSentences", true)).toEqual(["grammar.rule.LongSentences", "on"]);
    expect(ruleSetting("LongSentences", false)).toEqual(["grammar.rule.LongSentences", ""]);
  });
});
