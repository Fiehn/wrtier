import { describe, expect, it } from "vitest";
import { countsLine, countsOf, wordsIn } from "./counts";

describe("counting words and characters", () => {
  it("counts words as the History does, without function names or comments", () => {
    expect(wordsIn("She said, \"It's late.\" #scene-break() // a note\nAfter.")).toBe(5);
    expect(wordsIn("#epigraph[Every word] /* not this */ counts")).toBe(3);
    expect(wordsIn("Café — 1984 l’été")).toBe(3);
  });

  it("counts characters as they read: no markup, comments or line breaks, spaces apart", () => {
    // "The rain" + "Gulls _flew_." -> "The rain" (8) and "Gulls flew." (11)
    expect(countsOf("= The rain\nGulls _flew_.")).toEqual({ words: 4, characters: 19, charactersNoSpaces: 17 });
    expect(countsOf("#epigraph[Hi] // note")).toEqual({ words: 1, characters: 2, charactersNoSpaces: 2 });
    expect(countsOf("5 \\* 3")).toEqual({ words: 2, characters: 5, charactersNoSpaces: 3 }); // an escaped * is text
    expect(countsOf("Smørrebrød 🙂")).toEqual({ words: 1, characters: 12, charactersNoSpaces: 11 }); // one each, emoji too
    expect(countsOf("")).toEqual({ words: 0, characters: 0, charactersNoSpaces: 0 });
  });

  it("reads as a line, with the selection's share when there is one", () => {
    const all = { words: 12340, characters: 68201, charactersNoSpaces: 56100 };
    expect(countsLine(all, null)).toBe(`${(12340).toLocaleString()} words · ${(68201).toLocaleString()} characters`);
    expect(countsLine(all, { words: 120, characters: 640, charactersNoSpaces: 520 }))
      .toBe(`120 of ${(12340).toLocaleString()} words · 640 of ${(68201).toLocaleString()} characters`);
    expect(countsLine({ words: 1, characters: 1, charactersNoSpaces: 1 }, null)).toBe("1 word · 1 character");
  });
});
