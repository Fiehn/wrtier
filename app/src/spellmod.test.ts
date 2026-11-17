import { beforeEach, describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { markSources, type Mark } from "./marks";
import { noSettings, type ModContext, type ModSettings } from "./mods";
import { forgetSpelling, spellchecker, type Dictionary } from "./spellmod";

/** A Dictionary that knows a few words, and counts what it's asked. */
function fakeDictionary(knows: string[] | null) {
  const asked: string[][] = [];
  const dictionary: Dictionary = {
    unknown: async (_language, words) => {
      asked.push(words);
      return knows === null ? null : words.filter((w) => !knows.includes(w.toLowerCase()));
    },
    suggest: async (_language, word) => (word.toLowerCase() === "teh" ? ["the", "tea"] : []),
  };
  return { dictionary, asked };
}

const KNOWN = ["the", "rain", "fell", "on", "and", "saw", "left", "then", "she", "said", "it", "was"];

async function marksOf(doc: string, dictionary: Dictionary, settings: Partial<ModSettings> = {}, set: ModContext["setSetting"] = () => {}) {
  const ctx: ModContext = { language: "en-GB", settings: { ...noSettings(), ...settings }, setSetting: set };
  const state = EditorState.create({ doc, extensions: spellchecker(ctx, dictionary) });
  const marks = await state.facet(markSources)[0].find(state);
  return { state, marks };
}

const brief = (marks: Mark[]) => marks.map((m) => `${m.level}:${m.rule}:${m.from}-${m.to}`);

describe("the Spellchecker", () => {
  beforeEach(() => forgetSpelling());

  it("marks unknown words as Mistakes, and those that look like names as Queries", async () => {
    const { dictionary } = fakeDictionary(KNOWN);
    // "Kvothe" starts a sentence, but the Work capitalises it mid-sentence too: a name.
    const { marks } = await marksOf("Teh rain fell. Kvothe left. Then she saw Kvothe and Bast.", dictionary);
    expect(brief(marks)).toEqual(["mistake:unknown:0-3", "query:name:15-21", "query:name:41-47", "query:name:52-56"]);
    expect(marks.every((m) => m.dismissAs === "word" && m.dismissExact && m.actions?.length === 2)).toBe(true);
  });

  it("asks the Dictionary only about words it hasn't asked about before", async () => {
    const { dictionary, asked } = fakeDictionary(KNOWN);
    await marksOf("Teh rain fell.", dictionary);
    await marksOf("Teh rain fell on teh rain.", dictionary);
    expect(asked).toEqual([["Teh", "rain", "fell"], ["on", "teh"]]);
  });

  it("leaves the Author's own words alone: the Work's Allowed words and the Added words for its language", async () => {
    const { dictionary, asked } = fakeDictionary(KNOWN);
    const settings = {
      work: { "spellcheck.allow.Kvothe": "on" },
      vault: { "spellcheck.word.en.wrtier": "add", "spellcheck.word.da.teh": "add" },
    };
    const { marks } = await marksOf("Teh Wrtier saw Kvothe, KVOTHE and kvothe.", dictionary, settings);
    expect(marks.map((m) => m.from)).toEqual([0, 34]); // "Teh" (added for Danish only) and "kvothe"
    expect(asked[0]).not.toContain("Wrtier");
  });

  it("marks nothing where there's no Dictionary for the language", async () => {
    const { dictionary } = fakeDictionary(null);
    expect((await marksOf("Teh rain.", dictionary)).marks).toEqual([]);
  });

  it("checks neither rule once it's switched off in the Work", async () => {
    const { dictionary } = fakeDictionary(KNOWN);
    const { marks } = await marksOf("Teh rain fell on Kvothe.", dictionary, { work: { "spellcheck.rule.name": "off" } });
    expect(brief(marks)).toEqual(["mistake:unknown:0-3"]);
  });

  it("allows a word in the Work or adds it to the Dictionary for its language, without a capital only a sentence put there", async () => {
    const { dictionary } = fakeDictionary(KNOWN);
    const set: [string, string, string][] = [];
    const { marks } = await marksOf("Teh rain fell on Kvothe.", dictionary, {}, (scope, key, value) => set.push([scope, key, value]));
    for (const m of marks) for (const a of m.actions ?? []) a.run();
    expect(set).toEqual([
      ["work", "spellcheck.allow.teh", "on"],
      ["vault", "spellcheck.word.en.teh", "add"],
      ["work", "spellcheck.allow.Kvothe", "on"],
      ["vault", "spellcheck.word.en.Kvothe", "add"],
    ]);
  });

  it("asks for Suggestions only when they're shown, capitalised as typed, as edits to the word", async () => {
    const { dictionary } = fakeDictionary(KNOWN);
    const { state, marks } = await marksOf("It was *Teh* rain.", dictionary);
    const teh = marks[0];
    expect(teh.suggestions).toEqual([]);
    const suggestions = await teh.suggest!(teh, state);
    expect(suggestions.map((s) => s.label)).toEqual(["The", "Tea"]);
    expect(suggestions[0].edits).toEqual([{ from: 9, to: 11, insert: "he" }]);
  });
});
