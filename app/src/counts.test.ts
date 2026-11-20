import { describe, expect, it } from "vitest";
import { countsLine, countsOf, sentencesIn, wordsIn, wordsPerSentence } from "./counts";

describe("counting words and characters", () => {
  it("counts words as the History does, without function names or comments", () => {
    expect(wordsIn("She said, \"It's late.\" #scene-break() // a note\nAfter.")).toBe(5);
    expect(wordsIn("#epigraph[Every word] /* not this */ counts")).toBe(3);
    expect(wordsIn("Café — 1984 l’été")).toBe(3);
    // As the History counts them (Unicode's Alphabetic): a vowel sign doesn't split a word.
    expect(wordsIn("किताब")).toBe(1); // ि and ा are vowel signs
    expect(wordsIn("’ ' —")).toBe(0);
  });

  it("counts characters as they read: no markup, comments or line breaks, spaces apart", () => {
    // "The rain" + "Gulls _flew_." -> "The rain" (8) and "Gulls flew." (11)
    expect(countsOf("= The rain\nGulls _flew_.")).toEqual({ words: 4, sentences: 1, characters: 19, charactersNoSpaces: 17 });
    expect(countsOf("#epigraph[Hi] // note")).toEqual({ words: 1, sentences: 1, characters: 2, charactersNoSpaces: 2 });
    expect(countsOf("5 \\* 3")).toEqual({ words: 2, sentences: 1, characters: 5, charactersNoSpaces: 3 }); // an escaped * is text
    expect(countsOf("Smørrebrød 🙂")).toEqual({ words: 1, sentences: 1, characters: 12, charactersNoSpaces: 11 }); // one each, emoji too
    expect(countsOf("")).toEqual({ words: 0, sentences: 0, characters: 0, charactersNoSpaces: 0 });
  });

  it("reads as a line, with the selection's share when there is one", () => {
    const all = { words: 12340, sentences: 812, characters: 68201, charactersNoSpaces: 56100 };
    expect(countsLine(all, null)).toBe(`${(12340).toLocaleString()} words · 812 sentences · ${(68201).toLocaleString()} characters`);
    expect(countsLine(all, { words: 120, sentences: 8, characters: 640, charactersNoSpaces: 520 }))
      .toBe(`120 of ${(12340).toLocaleString()} words · 8 of 812 sentences · 640 of ${(68201).toLocaleString()} characters`);
    expect(countsLine({ words: 1, sentences: 1, characters: 1, charactersNoSpaces: 1 }, null)).toBe("1 word · 1 sentence · 1 character");
  });
});

describe("counting sentences", () => {
  it("ends one at . ! ? or … when the next one starts", () => {
    expect(sentencesIn("She left. He stayed! Why? Then… Nothing.")).toBe(5);
    expect(sentencesIn("It was late…and cold.")).toBe(1); // no new one starts after the …
  });

  it("keeps dialogue with what's said about it", () => {
    expect(sentencesIn("“Go!” she said. “Now.” He went.")).toBe(3);
    expect(sentencesIn("\"Stay,\" she said.")).toBe(1);
  });

  it("isn't ended by a title, an initial, an abbreviation inside a word, or a number", () => {
    expect(sentencesIn("Mr. Holm met Dr. Ali at St. Mary's.")).toBe(1);
    expect(sentencesIn("J. R. R. Tolkien wrote it.")).toBe(1);
    expect(sentencesIn("The U.S.A. is big. It cost 3.50 kroner.")).toBe(2);
  });

  it("runs on across a line break and ends at a blank line, unfinished or not", () => {
    expect(sentencesIn("The rain\nhad stopped. Gulls\n\nflew")).toBe(3);
  });

  it("doesn't count headings, markup or comments", () => {
    expect(sentencesIn("= Chapter one\nIt began. // Not this. Or this.\n#scene-break()\n\nIt ended.")).toBe(2);
    expect(sentencesIn("")).toBe(0);
  });

  it("gives how long the sentences are on average", () => {
    expect(wordsPerSentence(countsOf("One two three. Four five."))).toBe((2.5).toLocaleString());
    expect(wordsPerSentence(countsOf(""))).toBeNull();
  });
});
