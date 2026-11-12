import { describe, expect, it } from "vitest";
import { EditorState, type Extension } from "@codemirror/state";
import { bracketsForTests } from "./formatted";
import { elementsFacet, type ElementInfo } from "./elements";
import { INCLUSION, inclusions, rawInclusions, setInclusions, type InclusionView } from "./inclusions";
import { findExtension, findState, fold, matchesIn, readingOf, replacementChanges, setCurrent, setFind, wordEdits, withCase, type Query } from "./find";

const elements: ElementInfo[] = [
  { name: "epigraph", title: "Epigraph", shownAs: "block", inWork: true },
  { name: "scene-break", title: "Scene break", shownAs: "chip", inWork: true },
];

const q = (text: string, more: Partial<Query> = {}): Query => ({ text, matchCase: false, wholeWords: false, ...more });

/** An editor's state in the Formatted view (or the Raw view), finding `query`. */
function finding(doc: string, query: Query | null, view: "formatted" | "raw" = "formatted", extra: Extension[] = []) {
  const shown = view === "formatted" ? bracketsForTests : rawInclusions.of(true);
  const state = EditorState.create({ doc, extensions: [shown, elementsFacet.of(elements), inclusions(), findExtension(), ...extra] });
  return state.update({ effects: setFind.of({ query, scope: null }) }).state;
}

/** What each match covers, as typed. */
const found = (state: EditorState) => findState(state).matches.map((m) => (m.included ? `[${m.included.label}] ${m.included.from}-${m.included.to}` : state.doc.sliceString(m.from, m.to)));

/** The text once every own-text match is replaced (as Replace all does). */
function replaceAll(state: EditorState, replacement: string, matchCase = false): string {
  const { matches, reading } = findState(state);
  const changes = matches.filter((m) => !m.included).flatMap((m) => replacementChanges(reading!, m, replacement, matchCase));
  return state.update({ changes }).state.doc.toString();
}

describe("Find", () => {
  it("in the Formatted view, reads past hidden markup", () => {
    const doc = "The *rain* had stopped. = Not a heading\n= Rain fell\n_Rain_ \\#rain";
    expect(found(finding(doc, q("rain had")))).toEqual(["rain* had"]);
    expect(found(finding(doc, q("rain fell")))).toEqual(["Rain fell"]);
    expect(found(finding(doc, q("#rain")))).toEqual(["#rain"]); // the escape's backslash is hidden
    // The Raw view: every character as typed.
    expect(found(finding(doc, q("rain had"), "raw"))).toEqual([]);
    expect(found(finding(doc, q("*rain*"), "raw"))).toEqual(["*rain*"]);
    expect(found(finding(doc, q("*rain*")))).toEqual([], ); // the stars aren't there as it reads
  });

  it("reads dashes and the ellipsis as they show, and never matches one dash for another", () => {
    const doc = "Wait --- no... well-known";
    expect(found(finding(doc, q("—")))).toEqual(["---"]);
    expect(found(finding(doc, q("-")))).toEqual(["-"]); // only the hyphen in well-known
    expect(found(finding(doc, q("no…")))).toEqual(["no..."]);
    expect(found(finding(doc, q("--"), "raw"))).toEqual(["--"]);
  });

  it("ignores case unless asked, and quotes always match each other", () => {
    expect(found(finding("Don’t. don't. “Hi” \"hi\"", q("don't")))).toEqual(["Don’t", "don't"]);
    expect(found(finding("Don’t. don't.", q("Don't", { matchCase: true })))).toEqual(["Don’t"]);
    expect(found(finding("“Hi” \"hi\"", q('"hi"')))).toEqual(["“Hi”", '"hi"']);
    expect(found(finding("café cafe", q("cafe")))).toEqual(["cafe"]); // accents count
  });

  it("matches parts of words unless asked for whole words", () => {
    expect(found(finding("rain rainbow drain Rain.", q("rain")))).toHaveLength(4);
    expect(found(finding("rain rainbow drain Rain.", q("rain", { wholeWords: true })))).toEqual(["rain", "Rain"]);
  });

  it("never runs across a Story element or an Inclusion", () => {
    expect(found(finding("stopped. #scene-break() Then", q(". Then")))).toEqual([]);
    expect(found(finding("#epigraph[Old words.] Then", q("words. Then")))).toEqual([]);
    expect(found(finding("#epigraph[Old words.] Then", q("old words")))).toEqual(["Old words"]);
    expect(found(finding(`End.\n${INCLUSION}\nStart`, q("end.\n\nstart")))).toEqual([]);
  });

  it("folds case without moving places", () => {
    expect(fold("İstanbul ‘x’", false)).toHaveLength("İstanbul ‘x’".length);
    expect(fold("A’B", true)).toBe("A'B");
    expect(matchesIn("aaaa", q("aa"))).toEqual([[0, 2], [2, 4]]); // not overlapping
  });

  it("searches included text as its block shows it, and the Raw view's line instead", () => {
    const view: InclusionView = {
      at: 5, work: "Rain", label: "Rain: Three", chapter: "Three", text: "= Three\nThe rain had stopped.", missing: null, language: "en", whole: false, starts: 0, comments: [],
    };
    const doc = `Rain\n${INCLUSION}\nrain`;
    const withView = (state: EditorState) => state.update({ effects: setInclusions.of([view]) }).state;
    expect(found(withView(finding(doc, q("rain"))))).toEqual(["Rain", "[Rain] 12-16", "rain"]); // places in the included text
    expect(found(withView(finding(doc, q("three"))))).toEqual(["[Rain] 2-7"]); // the heading's = is hidden
    // The Raw view: `#inclusion("Rain", chapter: "Three")`.
    expect(found(withView(finding(doc, q("three"), "raw")))).toEqual(["[Rain] 29-34"]);
    expect(found(withView(finding(doc, q("stopped"), "raw")))).toEqual([]);
  });

  it("keeps the current match current while it's there, and starts at the cursor", () => {
    let state = finding("rain, rain, rain", null);
    state = state.update({ selection: { anchor: 3 } }).state;
    state = state.update({ effects: setFind.of({ query: q("rain"), scope: null }) }).state;
    expect(findState(state).current).toBe(1); // the first after the cursor
    state = state.update({ changes: { from: 0, insert: "Rain! " } }).state;
    expect(findState(state).matches).toHaveLength(4);
    expect(findState(state).current).toBe(2); // still the same "rain"
    state = state.update({ effects: setCurrent.of(-1) }).state;
    expect(findState(state).current).toBe(3); // wraps round
  });

  it("looks only inside the selection when asked", () => {
    const state = finding("rain. rain. rain.", null);
    const next = state.update({ effects: setFind.of({ query: q("rain"), scope: { from: 5, to: 12 } }) }).state;
    expect(findState(next).matches.map((m) => m.from)).toEqual([6]);
  });
});

describe("Replace", () => {
  it("rewrites only the words that differ, keeping the markup", () => {
    expect(replaceAll(finding("The *rain* had stopped.", q("rain had")), "snow has")).toBe("The *snow* has stopped.");
    expect(replaceAll(finding("The *rain* fell.", q("rain")), "the storm")).toBe("The *the storm* fell.");
    expect(replaceAll(finding("The *rain* fell.", q("rain")), "heavy rain")).toBe("The *heavy rain* fell.");
    expect(replaceAll(finding("= Rain fell\nmore", q("rain fell")), "snow fell")).toBe("= Snow fell\nmore");
  });

  it("copies the match's capitals when case is ignored", () => {
    expect(replaceAll(finding("Rain fell. rain. RAIN! McRain", q("rain")), "snow")).toBe("Snow fell. snow. SNOW! McSnow");
    expect(replaceAll(finding("Rain fell.", q("rain")), "snow", true)).toBe("snow fell.");
    expect(withCase("marie", "Anna")).toBe("Marie");
    expect(withCase("marie anne", "ANNA")).toBe("MARIE ANNE");
    expect(withCase("Marie", "anna")).toBe("Marie");
    expect(withCase("snow", "McRain")).toBe("snow");
  });

  it("works out edits word by word", () => {
    expect(wordEdits("rain had", "snow has")).toEqual([{ from: 0, to: 4, text: "snow" }, { from: 5, to: 8, text: "has" }]);
    expect(wordEdits("rain fell", "heavy rain")).toEqual([{ from: 0, to: 0, text: "heavy " }, { from: 4, to: 9, text: "" }]);
    expect(wordEdits("same", "same")).toEqual([]);
  });

  it("maps the reading back to the text as typed", () => {
    const r = readingOf("a*b*c", [{ from: 1, to: 2, kind: "hide" }, { from: 3, to: 4, kind: "hide" }]);
    expect(r.text).toBe("abc");
  });
});
