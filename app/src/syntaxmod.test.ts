import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import type { DecorationSet } from "@codemirror/view";
import { noSettings, type ModContext } from "./mods";
import { colour, paragraphEdges, styleOf, styleSetting, syntaxHighlighting, typeOf, typeSetting, typesShown, WordTypes, type Tagger, type WordTypeId } from "./syntaxmod";

const TAGS: Record<string, string> = {
  she: "PRON", he: "PRON", walked: "VERB", was: "AUX", ran: "VERB", left: "VERB", slowly: "ADV", quietly: "ADV",
  truly: "ADV", fast: "ADV", old: "ADJ", house: "NOUN", and: "CCONJ", because: "SCONJ", the: "DET", then: "ADV",
};

/** A Harper that types the words it knows, and counts what it's asked. */
function fakeTagger() {
  const asked: string[][] = [];
  const tagger: Tagger = async (language, paragraphs) => {
    asked.push(paragraphs);
    if (!["", "en"].includes(language.split("-")[0])) return null;
    return paragraphs.map((p) => [...p.matchAll(/\p{L}+/gu)]
      .filter((m) => TAGS[m[0].toLowerCase()])
      .map((m) => ({ from: m.index, to: m.index + m[0].length, tag: TAGS[m[0].toLowerCase()] })));
  };
  return { tagger, asked };
}

const ALL = new Set<WordTypeId>(["noun", "verb", "adjective", "adverb", "conjunction"]);

/** Each coloured stretch of `doc` as "text/type". */
function coloured(doc: string, decorations: DecorationSet): string[] {
  const out: string[] = [];
  decorations.between(0, doc.length, (from, to, deco) => {
    out.push(`${doc.slice(from, to)}/${String(deco.spec.class).replace("cm-word cm-word-", "")}`);
  });
  return out;
}

/** The colours of all of `doc`, once Harper has answered. */
async function colours(doc: string, shown = ALL) {
  const { tagger, asked } = fakeTagger();
  const words = new WordTypes(tagger);
  const state = EditorState.create({ doc });
  const first = colour(state, 0, doc.length, words, shown);
  await words.ask("en-GB", first.missing);
  return { first, asked, words, state, after: coloured(doc, colour(state, 0, doc.length, words, shown).decorations) };
}

describe("Syntax highlighting", () => {
  it("colours nouns, verbs, adjectives, adverbs and conjunctions, and nothing else", async () => {
    const { first, after } = await colours("She walked slowly and the old house was quiet because…");
    expect(first.missing).toEqual(["She walked slowly and the old house was quiet because…"]);
    expect(coloured("", first.decorations)).toEqual([]);
    expect(after).toEqual(["walked/verb", "slowly/adverb", "and/conjunction", "old/adjective", "house/noun", "was/verb", "because/conjunction"]);
    expect(typeOf("PROPN")).toBe("noun");
    expect(typeOf("AUX")).toBe("verb");
    expect(typeOf("PRON")).toBeUndefined();
    expect(typeOf("DET")).toBeUndefined();
  });

  it("colours the word as typed, not the markup around it", async () => {
    const { after } = await colours("She *quietly* left /* a note */ slowly.");
    expect(after).toEqual(["quietly/adverb", "left/verb", "slowly/adverb"]);
  });

  it("colours only the word types the Vault hasn't switched off", async () => {
    const settings = noSettings();
    const [key, value] = typeSetting("adverb", false);
    settings.vault[key] = value;
    expect(typesShown(settings)).toEqual(new Set(["noun", "verb", "adjective", "conjunction"]));
    expect(typeSetting("adverb", true)).toEqual(["syntax.type.adverb", ""]);
    const { after } = await colours("She walked slowly.", typesShown(settings));
    expect(after).toEqual(["walked/verb"]);
  });

  it("shows word types as a block of colour behind the words when the Vault chose that", async () => {
    const settings = noSettings();
    expect(styleOf(settings)).toBe("text");
    const [key, value] = styleSetting("block");
    settings.vault[key] = value;
    expect(styleOf(settings)).toBe("block");
    expect(styleSetting("text")).toEqual(["syntax.style", ""]);
    const doc = "She walked slowly.";
    const { words, state } = await colours(doc);
    const classes: string[] = [];
    colour(state, 0, doc.length, words, ALL, undefined, styleOf(settings)).decorations.between(0, doc.length, (from, to, deco) => {
      classes.push(`${doc.slice(from, to)}:${deco.spec.class}`);
    });
    expect(classes).toEqual(["walked:cm-word cm-word-block-verb", "slowly:cm-word cm-word-block-adverb"]);
  });

  it("asks only about the paragraphs on screen, each Footnote apart", async () => {
    const doc = "She walked.\n\nHe ran.#footnote[Truly fast.] Then left.\nThe old house.\n\nShe left.";
    const state = EditorState.create({ doc });
    const at = doc.indexOf("Then");
    expect(paragraphEdges(state.doc, at, at)).toEqual([doc.indexOf("He ran"), doc.indexOf("house.") + 6]);
    const { tagger, asked } = fakeTagger();
    const words = new WordTypes(tagger);
    const { missing } = colour(state, at, at, words, ALL);
    expect(missing).toEqual(["He ran. Then left. The old house.", "Truly fast."]);
    await words.ask("en-GB", missing);
    expect(asked).toEqual([missing]);
    expect(coloured(doc, colour(state, at, at, words, ALL).decorations)).toEqual(["ran/verb", "Truly/adverb", "fast/adverb", "Then/adverb", "left/verb", "old/adjective", "house/noun"]);
  });

  it("asks about a passage once, however often it's looked at or in how many Works", async () => {
    const { tagger, asked } = fakeTagger();
    const words = new WordTypes(tagger);
    await Promise.all([words.ask("en", ["She left.", "She left."]), words.ask("en", ["She left."])]);
    await words.ask("en", ["She left.", "He ran."]);
    expect(asked).toEqual([["She left."], ["He ran."]]);
  });

  it("asks again after a failed answer", async () => {
    let fail = true;
    const asked: string[][] = [];
    const words = new WordTypes(async (_, paragraphs) => {
      asked.push(paragraphs);
      if (fail) throw new Error("Harper fell over");
      return paragraphs.map(() => []);
    });
    await words.ask("en", ["She left."]);
    expect(words.get("She left.")).toBeUndefined();
    fail = false;
    await words.ask("en", ["She left."]);
    expect(words.get("She left.")).toEqual([]);
    expect(asked).toHaveLength(2);
  });

  it("keeps a changed paragraph's old colours until Harper answers about it", async () => {
    const doc = "She walked slowly.\n\nHe ran.";
    const { words, state } = await colours(doc);
    const before = colour(state, 0, doc.length, words, ALL).decorations;
    // "very " typed before "slowly": the first paragraph is new to Harper, the second isn't.
    const tr = state.update({ changes: { from: doc.indexOf("slowly"), insert: "very " } });
    const next = tr.state.doc.toString();
    const { decorations, missing } = colour(tr.state, 0, next.length, words, ALL, before.map(tr.changes));
    expect(missing).toEqual(["She walked very slowly."]);
    expect(coloured(next, decorations)).toEqual(["walked/verb", "slowly/adverb", "ran/verb"]);
  });

  it("does nothing in a Work that isn't in English, or with every word type switched off", () => {
    const ctx = (language: string, vault: Record<string, string> = {}): ModContext => ({ language, settings: { ...noSettings(), vault }, setSetting: () => {} });
    expect(syntaxHighlighting(ctx("da"), new WordTypes(fakeTagger().tagger))).toEqual([]);
    const off = Object.fromEntries((["noun", "verb", "adjective", "adverb", "conjunction"] as const).map((t) => typeSetting(t, false)));
    expect(syntaxHighlighting(ctx("en-GB", off), new WordTypes(fakeTagger().tagger))).toEqual([]);
    expect(syntaxHighlighting(ctx("en-GB"), new WordTypes(fakeTagger().tagger))).not.toEqual([]);
  });
});
