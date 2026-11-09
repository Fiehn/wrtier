import { describe, expect, it } from "vitest";
import { applyMove, chaptersIn, itemAt, mapThrough, neighbourDest, planMove } from "./chapters";

describe("finding the Chapters", () => {
  const text = "A dedication.\n= One\nThe rain had _stopped_.\n#part[Part Two]\n= Two: *Gulls*\nGulls circled the harbour.\n== Smaller\nStill two.";

  it("lists Chapters and Parts with their words, leaving out what comes before them", () => {
    expect(chaptersIn(text).map(({ kind, title, words }) => [kind, title, words])).toEqual([
      ["chapter", "One", 4],
      ["part", "Part Two", 0],
      ["chapter", "Two: Gulls", 7],
    ]);
  });

  it("knows which one a position is in", () => {
    const items = chaptersIn(text);
    expect(itemAt(items, 0)).toBe(-1);
    expect(itemAt(items, text.indexOf("rain"))).toBe(0);
    expect(itemAt(items, text.length)).toBe(2);
  });
});

/** Every arrangement worth trying: front matter or not, a Part or not, a final line break or not. */
function texts(): string[] {
  const out: string[] = [];
  for (const front of ["", "Title page.\n"]) {
    for (const part of [false, true]) {
      for (const n of [1, 2, 3]) {
        const chapters = Array.from({ length: n }, (_, i) => `= C${i}\nWords of ${i}.\n`);
        if (part && n > 1) chapters.splice(1, 0, "#part[Two]\n");
        const body = front + chapters.join("");
        out.push(body, body.slice(0, -1)); // with and without the last line break
      }
    }
  }
  return out;
}

const lines = (text: string) => text.split("\n").filter((l) => l !== "").sort();
const order = (text: string) => chaptersIn(text).map((i) => i.title);

describe("moving a Chapter", () => {
  it("puts it where it was dropped, keeps every line whole, and types nothing", () => {
    for (const text of texts()) {
      const items = chaptersIn(text);
      items.forEach((item, a) => {
        if (item.kind !== "chapter") return;
        const dests = [...items.map((i) => i.from), text.length];
        dests.forEach((dest, d) => {
          const moves = planMove(text, item.from, item.to, dest);
          let after = text;
          let heading = item.from;
          for (const m of moves) {
            expect(m.at < m.from || m.at > m.to).toBe(true); // a real Move, never onto itself
            after = applyMove(after, m);
            heading = mapThrough(heading, m);
          }
          const expected = items.map((i) => i.title);
          if (moves.length > 0) {
            expected.splice(a, 1);
            expected.splice(d > a ? d - 1 : d, 0, item.title);
          }
          const what = JSON.stringify({ text, a, dest });
          expect(order(after), what).toEqual(expected);
          expect(lines(after), what).toEqual(lines(text));
          expect(after.endsWith("\n"), what).toBe(text.endsWith("\n"));
          expect(after.startsWith(`= ${item.title}`, heading), what).toBe(true);
          expect(moves.length, what).toBeLessThanOrEqual(2);
          if (d === a || d === a + 1) expect(moves, what).toEqual([]);
        });
      });
    }
  });

  it("goes up or down past one neighbour, a Part included", () => {
    const text = "= A\na\n#part[P]\n= B\nb\n= C\nc";
    const items = chaptersIn(text);
    expect(neighbourDest(items, 0, -1, text.length)).toBeNull();
    expect(neighbourDest(items, 0, 1, text.length)).toBe(items[2].from); // after the Part
    expect(neighbourDest(items, 2, -1, text.length)).toBe(items[1].from); // before the Part
    expect(neighbourDest(items, 3, 1, text.length)).toBeNull();
    expect(neighbourDest(items, 2, 1, text.length)).toBe(text.length);
    expect(neighbourDest(items, 1, 1, text.length)).toBeNull(); // a Part doesn't move
  });
});
