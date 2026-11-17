import { describe, expect, it } from "vitest";
import { Text } from "@codemirror/state";
import { capitalisedLike, distance, editsFor, namesOf, OwnWords, suggestionsFor, wordsOf, wordToKeep } from "./spelling";

const words = (source: string) => wordsOf(source).map((w) => w.word);

/** The text after applying edits. */
function apply(source: string, edits: { from: number; to: number; insert: string }[]): string {
  let out = source;
  for (const e of [...edits].sort((a, b) => b.from - a.from)) out = out.slice(0, e.from) + e.insert + out.slice(e.to);
  return out;
}

describe("the words the Spellchecker checks", () => {
  it("are the text as it reads, with apostrophes inside words and compounds split at hyphens", () => {
    expect(words("She didn’t see the half-elf's *brave* face.")).toEqual(["She", "didn't", "see", "the", "half", "elf's", "brave", "face"]);
  });

  it("leave out Typst code, comments, raw, math, labels, links and words with digits", () => {
    const source = '#set text(size: 12pt)\nThe 3rd man #fnord(1) // a commnet\nsaw `rawwr` and $x y$ at <labl> https://exampel.com B52s here.';
    expect(words(source)).toEqual(["The", "man", "saw", "and", "at", "here"]);
  });

  it("include headings, without their =, and the text inside Story elements, but not their code", () => {
    expect(words("= Chapter Won\n\n#epigraph(from: \"Anan\")[Teh rain fell.]")).toEqual(["Chapter", "Won", "Teh", "rain", "fell"]);
    // A word never runs across a Story element's edge.
    expect(words("abc#letter[def]ghi")).toEqual(["abc", "def", "ghi"]);
  });

  it("keep their place in the text as typed, emphasis inside included", () => {
    const source = "It was un#emph[beliv]able, *relly*.";
    const w = wordsOf(source).find((x) => x.word === "unbelivable")!;
    expect(source.slice(w.from, w.to)).toBe("un#emph[beliv]able");
    expect(words(source)).toEqual(["It", "was", "unbelivable", "relly"]);
  });

  it("know a capital where no sentence starts: a name, most likely", () => {
    const found = wordsOf("Kvothe left. Then Anna saw Kvothe. “Bast,” she said.");
    expect(found.filter((w) => w.midSentence).map((w) => w.word)).toEqual(["Anna", "Kvothe"]);
    expect([...namesOf(found)]).toEqual(["Anna", "Kvothe"]);
  });
});

describe("the Author's own words", () => {
  const own = new OwnWords(["Zampanò", "wrtier", "don’t"]);

  it("count with their capitals or in all capitals; in small letters, however capitalised", () => {
    expect(["Zampanò", "ZAMPANÒ", "wrtier", "Wrtier", "WRTIER", "don't"].every((w) => own.knows(w))).toBe(true);
    expect(own.knows("zampanò")).toBe(false);
    expect(own.knows("Zampano")).toBe(false);
  });

  it("are kept as typed, except a capital only there because a sentence starts", () => {
    expect(wordToKeep("Wrtier", false)).toBe("wrtier");
    expect(wordToKeep("Kvothe", true)).toBe("Kvothe");
    expect(wordToKeep("NASA", false)).toBe("NASA");
    expect(wordToKeep("McAllister", false)).toBe("McAllister");
    expect(wordToKeep("don’t", false)).toBe("don't");
  });
});

describe("spelling Suggestions", () => {
  it("are capitalised as the word was typed, unless they have capitals of their own", () => {
    expect(capitalisedLike("Teh", "the")).toBe("The");
    expect(capitalisedLike("TEH", "the")).toBe("THE");
    expect(capitalisedLike("londun", "London")).toBe("London");
  });

  it("count letters added, taken away, changed and swapped", () => {
    expect(distance("teh", "the")).toBe(1);
    expect(distance("zampano", "zampanò")).toBe(1);
    expect(distance("recieve", "receive")).toBe(1);
    expect(distance("cat", "dog")).toBe(3);
    expect(distance("a", "abcdef")).toBe(3);
  });

  it("put the Author's own words one letter away first, at most five, none twice", () => {
    expect(suggestionsFor("Zampano", ["Zamboni", "Campano"], ["Zampanò"])).toEqual(["Zampanò", "Zamboni", "Campano"]);
    expect(suggestionsFor("teh", ["the", "The", "tea", "ten", "Tex", "eh", "meh"], [])).toEqual(["the", "tea", "ten", "Tex", "eh"]);
  });

  it("change only the letters that differ, leaving markup where it is", () => {
    const source = "It was un#emph[beliv]able.";
    const at = source.indexOf("un");
    const to = source.indexOf(".");
    expect(apply(source, editsFor(Text.of([source]), at, to, "unbelievable"))).toBe("It was un#emph[believ]able.");
    // Letters going on both sides of the markup: the markup stays.
    expect(apply(source, editsFor(Text.of([source]), at, to, "incredible"))).toBe("It was #emph[incredi]ble.");
    // A whole word in emphasis: the markers stay around it.
    expect(apply("*Teh* rain", editsFor(Text.of(["*Teh* rain"]), 0, 5, "The"))).toBe("*The* rain");
  });

  it("follow the word's apostrophes, and reach words in later paragraphs", () => {
    const source = "First.\n\nShe dont know, she did’nt.";
    const doc = Text.of(source.split("\n"));
    const dont = source.indexOf("dont");
    expect(apply(source, editsFor(doc, dont, dont + 4, "don't"))).toBe("First.\n\nShe don't know, she did’nt.");
    const didnt = source.indexOf("did’nt");
    expect(apply(source, editsFor(doc, didnt, didnt + 6, "didn't"))).toBe("First.\n\nShe dont know, she didn’t.");
  });
});
