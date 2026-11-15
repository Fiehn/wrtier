import { describe, expect, it } from "vitest";
import { changeBetween } from "./reading";

/** `old` with the change made. */
const applied = (old: string, next: string) => {
  const change = changeBetween(old, next);
  return change ? old.slice(0, change.from) + change.insert + old.slice(change.to) : old;
};

describe("taking in a new version of the Work being read", () => {
  it("changes nothing when it's the same", () => {
    expect(changeBetween("= One\n\nRain.", "= One\n\nRain.")).toBeNull();
  });

  it("changes only the stretch that differs, so the place being read stays put", () => {
    expect(changeBetween("The rain had stopped.", "The snow had stopped.")).toEqual({ from: 4, to: 8, insert: "snow" });
    expect(changeBetween("Dark.", "Dark. And wet.")).toEqual({ from: 5, to: 5, insert: " And wet." });
    expect(changeBetween("Prologue.\n\nDark.", "Dark.")).toEqual({ from: 0, to: 11, insert: "" });
  });

  it("gives the new text, whatever changed", () => {
    for (const [old, next] of [["", "abc"], ["abc", ""], ["aaa", "aa"], ["abcabc", "abc"], ["x", "xyx"], ["Rain", "Rain"]]) {
      expect(applied(old, next)).toBe(next);
    }
  });
});
