import { describe, expect, it } from "vitest";
import { EditorSelection, EditorState, Text } from "@codemirror/state";
import { imageDecorations, imageLine, imageWidgets, namedImages, namedIn, pictures, type Picture } from "./images";

const picture = (name: string): Picture => ({ name, url: `blob:${name}`, kind: "PNG", standIn: false });

function state(doc: string, cursor: number, known = true) {
  const byName = new Map([["map.png", picture("map.png")]]);
  return EditorState.create({ doc, selection: EditorSelection.cursor(cursor), extensions: [imageWidgets(), pictures({ known, byName })] });
}

/** What the Formatted view shows in place of the text: (from, to, block or chip, the name, the picture's address or null). */
function shown(s: EditorState) {
  const out: [number, number, string, string, string | null][] = [];
  imageDecorations(s).between(0, s.doc.length, (from, to, deco) => {
    const widget = deco.spec.widget as { name: string; picture?: Picture };
    out.push([from, to, deco.spec.block ? "block" : "chip", widget.name, widget.picture?.url ?? null]);
  });
  return out;
}

describe("Images in the editor", () => {
  it("finds the names image calls give, as Rust does", () => {
    const text = '#image("map.png", width: 80%) #my-image("no.png") #figure(std.image( "a\\"b.svg"))';
    expect(namedIn(text).map((n) => n.name)).toEqual(["map.png", 'a\\"b.svg']);
  });

  it("shows a line that's only an image as its picture, and the markup while the cursor is there", () => {
    const doc = 'Before.\n#image("Map.png", width: 80%)\nAfter.';
    expect(shown(state(doc, 0))).toEqual([[8, 37, "block", "Map.png", "blob:map.png"]]);
    expect(shown(state(doc, 12))).toEqual([]); // the cursor's on its line
  });

  it("shows one inside other markup as a chip, and a name no Image has as a placeholder", () => {
    const doc = '#figure(image("map.png"), caption: [The map])\n\n#image("mpa.png")';
    const found = shown(state(doc, 46));
    expect(found).toEqual([[14, 23, "chip", "map.png", "blob:map.png"], [47, 64, "block", "mpa.png", null]]);
  });

  it("shows nothing until the Work's pictures are known", () => {
    expect(shown(state('#image("map.png")\n', 18, false))).toEqual([]);
  });

  it("keeps the names up to date as the text changes", () => {
    let s = state('A.\n#image("map.png")\nB.', 0);
    s = s.update({ changes: { from: 0, insert: 'X #image("river.png")\n' } }).state;
    s = s.update({ changes: { from: s.doc.length, insert: '\n#image("sea.png")' } }).state;
    expect(namedImages(s).map((n) => [n.name, s.sliceDoc(n.from, n.to)])).toEqual([
      ["river.png", "river.png"], ["map.png", "map.png"], ["sea.png", "sea.png"],
    ]);
    const edited = s.update({ changes: { from: namedImages(s)[1].from, to: namedImages(s)[1].to, insert: "Westmarch.png" } }).state;
    expect(namedImages(edited).map((n) => n.name)).toEqual(["river.png", "Westmarch.png", "sea.png"]);
  });

  it("writes an image in on a line of its own", () => {
    const doc = Text.of(["She went home.", "", "The end."]);
    expect(imageLine(doc, 15, "map.png")).toEqual({ from: 15, insert: '#image("map.png")', cursor: 32 }); // an empty line
    expect(imageLine(doc, 14, "map.png").insert).toBe('\n#image("map.png")'); // the end of a line
    expect(imageLine(doc, 4, "map.png").insert).toBe('\n#image("map.png")\n'); // the middle of one: split
  });
});
