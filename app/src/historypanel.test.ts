import { describe, expect, it } from "vitest";
import { dayOf, entries, fieldLine, leftOutLine, libraryLines, moveLine, noticeLine, replacementLine, showMeLabel, wordCounts, type HistoryView, type NoticeView, type SessionRow } from "./historypanel";

const session = (key: string, endMs: number): SessionRow => ({
  key,
  changes: [key],
  device: "Laptop",
  startMs: endMs - 1000,
  endMs,
  continued: false,
  source: null,
  addedWords: 1,
  removedWords: 0,
  reverted: "no",
  notIn: [],
});

describe("the History tab", () => {
  it("heads days as today, yesterday, or the date", () => {
    const now = new Date(2026, 10, 9, 15, 0).getTime();
    expect(dayOf(new Date(2026, 10, 9, 0, 5).getTime(), now)).toBe("Today");
    expect(dayOf(new Date(2026, 10, 8, 23, 55).getTime(), now)).toBe("Yesterday");
    expect(dayOf(new Date(2026, 10, 3, 12, 0).getTime(), now)).not.toMatch(/Today|Yesterday|2026/);
    expect(dayOf(new Date(2025, 10, 3, 12, 0).getTime(), now)).toMatch(/2025/);
  });

  it("counts words with a sign, leaving out a zero side", () => {
    expect(wordCounts(412, 58)).toBe("+412 −58 words");
    expect(wordCounts(1, 0)).toBe("+1 word");
    expect(wordCounts(0, 3)).toBe("−3 words");
    expect(wordCounts(0, 0)).toBe("no words");
  });

  it("says what a Preamble edit did", () => {
    const f = { by: "a", change: "a", record: "preamble", field: "title", value: "The Rain", before: "Rain", takenBack: false, library: false, inEffect: true };
    expect(fieldLine(f)).toBe("Title: The Rain (was Rain)");
    expect(fieldLine({ ...f, field: "looks", value: "Paperback\nManuscript", before: null })).toBe("Looks: Paperback, Manuscript");
    expect(fieldLine({ ...f, field: "raw", takenBack: true })).toBe("Taken back: Raw Typst edited");
  });

  it("says what a Replace all did, as one line", () => {
    const r = { find: "rain", with: "snow", count: 14, changes: ["a", "b"], inEffect: true, revertedBy: null };
    expect(replacementLine(r)).toBe("Replaced “rain” with “snow” (14)");
    expect(replacementLine({ ...r, find: "rain had", with: "snow has", count: 1 })).toBe("Replaced “rain had” with “snow has”");
  });

  it("says where moved text came from", () => {
    expect(moveLine({ back: false, chapter: "Three", after: "The harbour wall.", earlier: false })).toBe("Moved here from Three, after “The harbour wall.”");
    expect(moveLine({ back: true, chapter: null, after: "", earlier: true })).toBe("Moved back from earlier in the text");
    // In a preview: where it would go.
    expect(moveLine({ back: false, chapter: "Three", after: "", earlier: false }, true)).toBe("Moves back to Three");
    expect(moveLine({ back: true, chapter: "One", after: "", earlier: true }, true)).toBe("Moves to One");
  });

  it("says what a Revert leaves alone", () => {
    expect(leftOutLine({ library: ["Book text"], alreadyReverted: 0, comments: 2, milestones: ["Draft 2"] }))
      .toBe("Not reverted: 2 Comments, Milestone “Draft 2”, the recorded copies of Book text.");
    expect(leftOutLine({ library: [], alreadyReverted: 0, comments: 0, milestones: [] })).toBeNull();
  });

  it("puts Milestones between Sessions by time, and waits with older ones for Show older", () => {
    const view: HistoryView = {
      sessions: [session("c", 3000), session("b", 2000)],
      milestones: [
        { id: "m1", name: "Old", timeMs: 500, deleted: false, device: "Laptop" },
        { id: "m2", name: "Draft", timeMs: 2500, deleted: false, device: "Laptop" },
      ],
      more: true,
      notices: [],
    };
    expect(entries(view).map((e) => (e.kind === "session" ? e.row.key : e.row.name))).toEqual(["c", "Draft", "b"]);
    expect(entries({ ...view, more: false }).map((e) => (e.kind === "session" ? e.row.key : e.row.name))).toEqual(["c", "Draft", "b", "Old"]);
  });

  it("says each Merge notice as a plain sentence", () => {
    const notice = (kind: NoticeView["kind"], quote = "the harbour wall", otherDevice: string | null = "Phone"): NoticeView =>
      ({ key: "k", kind, device: "Laptop", timeMs: 0, otherDevice, quote, places: [] });
    expect(noticeLine(notice("overlap"))).toBe(
      "Phone and Laptop moved overlapping text without seeing each other's Move. The part both moved, “the harbour wall”, went with Laptop's, the later one; the rest of each Move still happened.",
    );
    expect(noticeLine(notice("overlap", ""))).toContain("The part both moved went with Laptop's");
    expect(noticeLine(notice("loop", "the harbour wall", null))).toBe(
      "Laptop's Move of “the harbour wall” was skipped: another Move at the same time put the two passages inside each other. That text stayed where it was.",
    );
    expect(noticeLine(notice("externalEdit", "eased", null))).toMatch(/^The Text file was edited outside the app \(on Laptop\).*Check what it wrote: “eased”$/);
    expect(noticeLine(notice("preamble", ""))).toMatch(/^The Preamble's raw Typst was edited on Phone and on Laptop without/);
    expect(noticeLine(notice("preamble", "", "Laptop"))).toContain("edited twice on Laptop");
  });

  it("steps \"Show me\" through a notice's places", () => {
    expect(showMeLabel(1, undefined)).toBe("Show me");
    expect(showMeLabel(3, undefined)).toBe("Show me (3 places)");
    expect(showMeLabel(3, 1)).toBe("Next place (2 of 3)");
    expect(showMeLabel(3, 0)).toBe("Next place (1 of 3)"); // round again
  });
});

describe("what a Session recorded on its own", () => {
  const field = (record: string, included: string | null = null) => ({
    by: "1", change: "1", record, field: "content", value: null, before: null, takenBack: false, library: true, included, inEffect: true,
  });

  it("names library files, included Works and their Story elements in plain words", () => {
    expect(libraryLines([
      field("style/Book text"), field("included/0a1b", "Zampanò"), field("included/9f8e"), field("included/story-elements"),
    ])).toEqual([
      "Recorded library changes: Book text",
      "Recorded the versions it shows of the included Zampanò, a Work no longer in the Vault",
      "Recorded the Story elements its Inclusions use",
    ]);
  });
});
