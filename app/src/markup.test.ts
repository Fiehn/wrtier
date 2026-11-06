import { describe, expect, it } from "vitest";
import { parseMarkup, type Span } from "./markup";

/** The spans of `text` as (kind, the text they cover). */
const found = (text: string) => parseMarkup(text).map((s) => [s.kind, text.slice(s.from, s.to)]);
const one = (text: string, kind: Span["kind"]) => parseMarkup(text).find((s) => s.kind === kind)!;

describe("headings", () => {
  it("are = at the start of a line, with the level and the marks to hide", () => {
    const h = one("== The Long Night\nIt was dark.", "heading") as Extract<Span, { kind: "heading" }>;
    expect(h.level).toBe(2);
    expect(h.mark).toEqual([0, 3]);
    expect([h.from, h.to]).toEqual([0, 17]);
  });
  it("aren't = elsewhere", () => {
    expect(found("a = b")).toEqual([]);
  });
});

describe("strong and emphasis", () => {
  it("pair their delimiters", () => {
    const s = one("It was *very* dark.", "strong") as Extract<Span, { kind: "strong" | "emph" }>;
    expect([s.from, s.to, s.open, s.close]).toEqual([7, 13, [7, 8], [12, 13]]);
    expect(found("_quietly_")).toEqual([["emph", "_quietly_"]]);
  });
  it("nest", () => {
    expect(found("*a _b_ c*")).toEqual([["strong", "*a _b_ c*"], ["emph", "_b_"]]);
  });
  it("don't count inside words", () => {
    expect(found("snake_case and 2*3*4")).toEqual([]);
  });
  it("span lines but never a blank line", () => {
    expect(found("*one\ntwo*")).toEqual([["strong", "*one\ntwo*"]]);
    expect(found("*one\n\ntwo*")).toEqual([]);
  });
  it("leave an unclosed one as text", () => {
    expect(found("a *b")).toEqual([]);
  });
});

describe("raw, comments and escapes", () => {
  it("raw text hides nothing inside it", () => {
    expect(found("Use `*not strong*` here")).toEqual([["raw", "`*not strong*`"]]);
    const block = one("```typ\n#set page()\n```", "raw") as Extract<Span, { kind: "raw" }>;
    expect(block.open).toEqual([0, 6]);
  });
  it("comments run to the end of the line, or the end of the block", () => {
    expect(found("text // a note\nmore")).toEqual([["comment", "// a note"]]);
    expect(found("a /* b /* c */ d */ e")).toEqual([["comment", "/* b /* c */ d */"]]);
  });
  it("a link isn't a comment", () => {
    expect(found("see https://typst.app/docs here")).toEqual([["link", "https://typst.app/docs"]]);
  });
  it("escapes and line breaks", () => {
    expect(found("\\#not code \\* and \\\nnext")).toEqual([["escape", "\\#"], ["escape", "\\*"], ["linebreak", "\\"]]);
    expect(found("\\😊")).toEqual([["escape", "\\😊"]]);
  });
});

describe("code", () => {
  it("function calls and fields", () => {
    expect(found("#smallcaps(\"Ab\") and #text.size")).toEqual([["code", '#smallcaps("Ab")'], ["code", "#text.size"]]);
  });
  it("a content block is markup again, with code brackets", () => {
    expect(found("#quote[It *was* dark]")).toEqual([["code", "#quote["], ["strong", "*was*"], ["code", "]"]]);
  });
  it("statements run to the end of the line", () => {
    expect(found("#set text(lang: \"en\") *x*\nnext")).toEqual([["code", '#set text(lang: "en") *x*']]);
  });
  it("a bracket in a string doesn't close anything", () => {
    expect(found('#f("(")')).toEqual([["code", '#f("(")']]);
  });
  it("a lone # is text", () => {
    expect(found("# not code")).toEqual([]);
  });
});

describe("other markup", () => {
  it("labels, references, math, list markers", () => {
    expect(found("<intro> see @intro, $x^2$")).toEqual([["label", "<intro>"], ["label", "@intro"], ["math", "$x^2$"]]);
    expect(found("- one\n+ two")).toEqual([["list", "-"], ["list", "+"]]);
    expect(found("mail me@home")).toEqual([]);
  });
  it("dashes and the ellipsis become glyphs", () => {
    const glyphs = parseMarkup("a -- b --- c...").filter((s) => s.kind === "glyph").map((s) => (s as { glyph: string }).glyph);
    expect(glyphs).toEqual(["–", "—", "…"]);
  });
});

describe("one paragraph at a time", () => {
  it("parsing a paragraph alone gives what parsing the whole text gives", () => {
    const text = "= One\nIt was *dark*.\n\n_Two_ -- #quote[three]\n\nfour `raw` // five";
    const whole = parseMarkup(text);
    const parts: Span[] = [];
    let start = 0;
    for (const paragraph of text.split("\n\n")) {
      parts.push(...parseMarkup(paragraph, start));
      start += paragraph.length + 2;
    }
    expect(parts).toEqual(whole);
  });
  it("positions count from where the paragraph starts", () => {
    expect(parseMarkup("*x*", 100)[0]).toMatchObject({ from: 100, to: 103 });
  });
});
