import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { parseMarkup } from "./markup";
import { footnoteSpans, footnoteStretches, footnoteText, footnotesInText, insertFootnote, numberFootnotes, withoutFootnotes } from "./footnotes";
import { passagesOf, readPara } from "./prose";
import { checkDialogue } from "./dialogue";
import { editsFor, wordsOf } from "./spelling";
import { chaptersIn } from "./chapters";
import { bracketsForTests } from "./formatted";
import { inclusions } from "./inclusions";
import { findExtension, findState, setFind } from "./find";
import { Text } from "@codemirror/state";

describe("a Footnote in the markup", () => {
  it("is one span from its # to its ], with its head and bracket", () => {
    const text = "It ended.#footnote[Or did it?] Then";
    const [f] = footnotesInText(text);
    expect(text.slice(f.from, f.to)).toBe("#footnote[Or did it?]");
    expect(text.slice(f.open[0], f.open[1])).toBe("#footnote[");
    expect(text.slice(f.close[0], f.close[1])).toBe("]");
    // Its head and bracket aren't code any more; its text is markup again.
    expect(parseMarkup("A#footnote[_very_ odd]").map((s) => s.kind)).toEqual(["footnote", "emph"]);
  });

  it("nests, outer first", () => {
    const fs = footnotesInText("a#footnote[b#footnote[c]d]e");
    expect(fs.map((f) => [f.from, f.to])).toEqual([[1, 26], [12, 24]]);
  });

  it("isn't one when written with arguments, escaped or left open", () => {
    expect(footnotesInText("#footnote(numbering: \"*\")[x]")).toEqual([]);
    expect(footnotesInText("\\#footnote[x]")).toEqual([]);
    expect(footnotesInText("#footnote[never closed")).toEqual([]);
  });
});

describe("the editor's numbers", () => {
  it("go 1, 2, 3 in the order the Work reads, counting its Inclusions' Footnotes", () => {
    const own = footnoteSpans(parseMarkup("a#footnote[x] b ￼ c#footnote[y]"));
    const numbers = numberFootnotes(own, [{ at: 16, count: 2 }]);
    expect([...numbers.values()]).toEqual([1, 4]);
  });
});

describe("a Footnote's text as it reads", () => {
  it("drops its markup, and shows a Footnote inside it as a mark", () => {
    const text = "A#footnote[It was _very_ odd --- #footnote[inner] really.]";
    const [f] = footnotesInText(text);
    expect(footnoteText(text, f)).toBe("It was very odd — * really.");
  });
  it("is what's counted as words in Footnotes (the outermost once)", () => {
    expect(footnoteStretches("x#footnote[one #footnote[two]] y#footnote[three]")).toEqual(["one #footnote[two]", "three"]);
  });
});

describe("a heading with a Footnote", () => {
  it("is named without it in the Chapter list", () => {
    expect(chaptersIn("= The End#footnote[Or is it?]\nText.")[0].title).toBe("The End");
    expect(withoutFootnotes("Plain")).toBe("Plain");
  });
});

describe("Insert footnote", () => {
  it("writes one at the cursor, leaving the cursor in it", () => {
    const state = EditorState.create({ doc: "word", selection: { anchor: 4 } });
    const t = state.update(insertFootnote(state));
    expect(t.state.doc.toString()).toBe("word#footnote[]");
    expect(t.state.selection.main.head).toBe(14);
  });
  it("puts one around the selection, which becomes its text", () => {
    const state = EditorState.create({ doc: "a note here", selection: { anchor: 2, head: 6 } });
    const t = state.update(insertFootnote(state));
    expect(t.state.doc.toString()).toBe("a #footnote[note] here");
    expect(t.state.sliceDoc(t.state.selection.main.from, t.state.selection.main.to)).toBe("note");
  });
});

describe("reading for the Mods", () => {
  it("reads the sentence around a Footnote as if it weren't there", () => {
    const source = '"Wait,"#footnote[A Danish idiom.] she said.';
    expect(readPara(source, 0, source.length).text).toBe('"Wait," she said.');
  });

  it("reads each Footnote's text as a passage of its own, after its paragraph", () => {
    const source = "One#footnote[two #footnote[three]] four.\n\nFive.";
    expect(passagesOf(source).map((p) => [source.slice(p.start, p.end), p.note])).toEqual([
      ["One#footnote[two #footnote[three]] four.", false],
      ["two #footnote[three]", true],
      ["three", true],
      ["Five.", false],
    ]);
  });

  it("keeps a Dialogue tag after a Footnote a Dialogue tag", () => {
    expect(checkDialogue('"Wait,"#footnote[A Danish idiom.] she said.')).toEqual([]);
    // Without the Footnote taken out, a period before the tag is still found.
    expect(checkDialogue('"Wait."#footnote[A Danish idiom.] she said.').map((f) => f.rule)).toEqual(["comma-before-tag"]);
  });

  it("checks a Footnote's text, and never runs a word across its edge", () => {
    expect(wordsOf("A wrod#footnote[a mispeled note]s here.").map((w) => w.word)).toEqual(["A", "wrod", "s", "here", "a", "mispeled", "note"]);
  });

  it("edits a word inside a Footnote where it is", () => {
    const doc = Text.of(["A#footnote[a _mispeled_ note]."]);
    const from = doc.toString().indexOf("mispeled");
    let out = doc.toString();
    for (const e of editsFor(doc, from, from + 8, "misspelled").sort((a, b) => b.from - a.from)) out = out.slice(0, e.from) + e.insert + out.slice(e.to);
    expect(out).toBe("A#footnote[a _misspelled_ note].");
  });
});

describe("Find and replace", () => {
  const finding = (doc: string, text: string) => {
    const state = EditorState.create({ doc, extensions: [bracketsForTests, inclusions(), findExtension()] });
    const s = state.update({ effects: setFind.of({ query: { text, matchCase: false, wholeWords: false }, scope: null }) }).state;
    return findState(s).matches.map((m) => s.doc.sliceString(m.from, m.to));
  };
  it("finds a Footnote's text", () => {
    expect(finding("He left.#footnote[In March.]", "march")).toEqual(["March"]);
  });
  it("never runs a match across a Footnote's edge", () => {
    expect(finding('"Wait,"#footnote[x] she said.', "wait, she")).toEqual([]);
    expect(finding("left#footnote[In March.] home", "March. home")).toEqual([]);
  });
});

// ---- Annotations (ADR 0020) ----

import { includedNoteCount, includedNotes, findAnnotation, type AnnotationShown } from "./footnotes";
import { INCLUSION, readerText, setInclusions, type InclusionView } from "./inclusions";
import { replacedAnnotations } from "./find";
import { noteRows } from "./footnotespanel";

const note = (id: string, at: number, body: string, mine = true, notes: AnnotationShown[] = []): AnnotationShown => ({
  id, owner: mine ? "me" : "truant", ownerLabel: mine ? "Book" : "Truant", mine, at, ranges: [[at - 4, at]], body, quote: "word", notes,
});

/** A Work with one Inclusion showing `text` with `annotations` on it. */
function withInclusion(doc: string, text: string, annotations: AnnotationShown[], extra: object[] = []) {
  const at = doc.indexOf(INCLUSION);
  const view: InclusionView = { at, work: "Rain", label: "Rain", text, missing: null, language: "en", whole: true, starts: 0, comments: [], annotations };
  const state = EditorState.create({ doc, extensions: [bracketsForTests, inclusions(), findExtension(), ...extra as never[]] });
  return state.update({ effects: setInclusions.of([view]) }).state;
}

describe("notes printed in an Inclusion", () => {
  it("are its text's Footnotes and the Annotations on it, in the order they print, one on another right after it", () => {
    const text = "The rain#footnote[Zampano's.] had stopped. She went home.";
    const truant = note("t", 8, "Truant's.", false, [note("e", 8, "The Editors'.")]);
    const editors = note("e2", 42, "Editors on home.");
    const notes = includedNotes(text, [editors, truant]);
    expect(notes.map((n) => (n.kind === "footnote" ? `fn ${n.text}` : `${n.note.id}${n.on ? ` on ${n.on.id}` : ""}`))).toEqual(["fn Zampano's.", "t", "e on t", "e2"]);
    expect(includedNoteCount({ text, annotations: [editors, truant] })).toBe(4);
    expect(includedNoteCount({ text, annotations: [truant], missing: "gone" })).toBe(0);
  });

  it("number on after the Work's own Footnotes, in the editor and the Footnotes tab", () => {
    const doc = `Own.#footnote[First.]\n${INCLUSION}\nLater.#footnote[Last.]`;
    const state = withInclusion(doc, "Rain fell#footnote[Theirs.] on it.", [note("a", 13, "Mine on it.")]);
    const rows = noteRows(state);
    expect(rows.map((r) => [r.number, r.text, r.from ?? "", r.annotation?.mine ?? null])).toEqual([
      [1, "First.", "", null],
      [2, "Theirs.", "Rain", null],
      [3, "Mine on it.", "", true],
      [4, "Last.", "", null],
    ]);
  });

  it("are counted: the Work's own Annotations as its words, another Work's as included", () => {
    const doc = `Mine.\n${INCLUSION}`;
    const state = withInclusion(doc, "Their text.", [note("a", 5, "Two words", true), note("t", 11, "Three more words", false)]);
    const read = readerText(state);
    expect(read.text).toContain("#footnote[Two words]");
    expect(read.included).not.toContain("Two words");
    expect(read.included).toContain("#footnote[Three more words]");
  });

  it("can be found by their IDs, the Work in front's own without naming it", () => {
    const views = [{ annotations: [note("t", 4, "Truant's.", false, [note("e", 4, "Mine.")])] }];
    expect(findAnnotation(views, "e", "")?.on?.id).toBe("t");
    expect(findAnnotation(views, "t", "truant")?.note.body).toBe("Truant's.");
    expect(findAnnotation(views, "t", "")).toBeNull();
  });
});

describe("Find and replace in Annotations", () => {
  const finding = (state: EditorState, text: string) => state.update({ effects: setFind.of({ query: { text, matchCase: false, wholeWords: false }, scope: null }) }).state;

  it("finds the Work's own Annotations' text as it reads, never another Work's", () => {
    const doc = `Navidson.\n${INCLUSION}`;
    let state = withInclusion(doc, "Their Navidson.", [note("a", 14, "The _Navidson_ Record."), note("t", 14, "Navidson again.", false)]);
    state = finding(state, "navidson record");
    const matches = findState(state).matches;
    expect(matches).toHaveLength(1);
    expect(matches[0].annotation).toMatchObject({ id: "a", read: [4, 19] });
    state = finding(state, "navidson");
    expect(findState(state).matches.map((m) => (m.annotation ? `a:${m.annotation.id}` : m.included ? "included" : "own"))).toEqual(["own", "included", "a:a"]);
  });

  it("replaces only the words that differ, keeping an Annotation's markup", () => {
    const doc = `x\n${INCLUSION}`;
    let state = withInclusion(doc, "Text.", [note("a", 4, "The _Navidson_ Record, and Navidson.")]);
    state = finding(state, "navidson");
    const matches = findState(state).matches.filter((m) => m.annotation);
    expect(replacedAnnotations(matches, "Holloway", false)).toEqual([{ id: "a", body: "The _Holloway_ Record, and Holloway." }]);
  });
});

describe("a Footnote's brackets", () => {
  const typed = (doc: string, from: number, to: number) => {
    const state = EditorState.create({ doc, extensions: [bracketsForTests] });
    return state.update({ changes: { from, to }, userEvent: "delete.backward" }).state.doc.toString();
  };
  it("are never deleted on their own, and go with its head (its text stays)", () => {
    const doc = "Word#footnote[Its note.] after";
    expect(typed(doc, 23, 24)).toBe(doc); // its ] alone: kept
    // Its head (one unit in the editor) deleted: its ] goes with it, its text stays.
    expect(typed(doc, 4, 14)).toBe("WordIts note. after");
  });
});
