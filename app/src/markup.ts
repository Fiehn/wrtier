// Where the markup is in the prose part of Typst, for the Formatted view.
//
// A small parser on purpose (see the plan): headings, *strong*, _emphasis_, `raw`, comments,
// escapes, line breaks, #code, labels and references, math, links, list markers, and the
// dashes and ellipsis Typst turns into glyphs. It only tells the editor how to show the text;
// compiling always uses Typst itself. Nothing it finds reaches past a blank line, so parsing
// one paragraph gives the same result as parsing the whole Work, and where it disagrees with
// Typst the text just shows as raw markup.

export type Span =
  /** `= Title`: `mark` is the `=`s and the space after them. */
  | { kind: "heading"; from: number; to: number; level: number; mark: [number, number] }
  /** `*strong*` or `_emphasis_`: `open` and `close` are the delimiters. */
  | { kind: "strong" | "emph"; from: number; to: number; open: [number, number]; close: [number, number] }
  /** `` `raw` `` or ```` ```raw``` ````: `open` and `close` are the backticks (and language). */
  | { kind: "raw"; from: number; to: number; open: [number, number]; close: [number, number] }
  /** `\x`: the backslash hides, the character shows. */
  | { kind: "escape"; from: number; to: number }
  /** `--`, `---`, `...`: shown as `glyph`. */
  | { kind: "glyph"; from: number; to: number; glyph: string }
  /** Styled only: comments, code, labels and references, math, links, list markers, line breaks. */
  | { kind: "comment" | "code" | "label" | "math" | "link" | "list" | "linebreak"; from: number; to: number };

const isWordChar = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}]/u.test(c);
const isIdentStart = (c: string | undefined) => c !== undefined && /[\p{L}_]/u.test(c);
const isIdentChar = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}_-]/u.test(c);
/** Code that runs to the end of its line: `#set text(…)`, `#let x = …`. */
const STATEMENTS = new Set(["let", "set", "show", "import", "include"]);

/** Parses `text`, whose first character is at `base` in the Work's text. Spans come out sorted. */
export function parseMarkup(text: string, base = 0): Span[] {
  const spans: Span[] = [];
  const n = text.length;
  /** Open `*` and `_` (strong, emphasis) and `[` of content blocks, innermost last. */
  let open: { kind: "strong" | "emph" | "content"; at: number }[] = [];
  let lineStart = true;
  let i = 0;

  /** Where the line that `i` is on ends (at its `\n`, or the end). */
  const lineEnd = (i: number) => {
    const at = text.indexOf("\n", i);
    return at === -1 ? n : at;
  };
  /** Where the paragraph that `i` is in ends: the next blank line, or the end. */
  const paragraphEnd = (i: number) => {
    const blank = /\n[ \t]*\n/g;
    blank.lastIndex = i;
    const m = blank.exec(text);
    return m ? m.index : n;
  };
  /** The end of a bracketed stretch starting at `i` (`(`, `{` or `[`), or -1 if it doesn't close in the paragraph. */
  const balanced = (i: number) => {
    const end = paragraphEnd(i);
    const closing: Record<string, string> = { "(": ")", "{": "}", "[": "]" };
    const stack: string[] = [];
    for (let j = i; j < end; j++) {
      const c = text[j];
      if (c === '"') {
        // A string: brackets inside it don't count.
        for (j++; j < end && text[j] !== '"'; j++) if (text[j] === "\\") j++;
      } else if (c in closing) {
        stack.push(closing[c]);
      } else if (c === stack[stack.length - 1]) {
        stack.pop();
        if (stack.length === 0) return j + 1;
      }
    }
    return -1;
  };
  const add = (span: Span) => spans.push(span);
  const at = (from: number, to: number) => ({ from: base + from, to: base + to });

  while (i < n) {
    const c = text[i];

    // A blank line ends a paragraph: nothing open carries over.
    if (c === "\n") {
      if (/^\n[ \t]*\n/.test(text.slice(i, i + 64))) open = [];
      lineStart = true;
      i++;
      continue;
    }

    if (lineStart) {
      lineStart = false;
      let j = i;
      while (text[j] === " " || text[j] === "\t") j++;
      const heading = /^(=+)( |$|\n)/.exec(text.slice(j, j + 16));
      if (heading && open.length === 0) {
        const marks = heading[1].length + (heading[2] === " " ? 1 : 0);
        add({ kind: "heading", ...at(j, lineEnd(j)), level: heading[1].length, mark: [base + j, base + j + marks] });
        i = j + marks;
        continue;
      }
      const list = /^([-+]|\/|\d+\.) /.exec(text.slice(j, j + 8));
      if (list) {
        add({ kind: "list", ...at(j, j + list[1].length) });
        i = j + list[1].length + 1;
        continue;
      }
    }

    // Escapes and line breaks: `\#` shows `#`; `\` before a space or the line's end breaks the line.
    if (c === "\\") {
      const next = text.codePointAt(i + 1);
      if (next === undefined || text[i + 1] === " " || text[i + 1] === "\n") {
        add({ kind: "linebreak", ...at(i, i + 1) });
        i++;
      } else {
        const len = next > 0xffff ? 2 : 1;
        add({ kind: "escape", ...at(i, i + 1 + len) });
        i += 1 + len;
      }
      continue;
    }

    // Links before comments: `https://` isn't the start of a comment.
    const link = /^https?:\/\/[^\s<>\]\)]+/.exec(text.slice(i, i + 2048));
    if (link) {
      add({ kind: "link", ...at(i, i + link[0].length) });
      i += link[0].length;
      continue;
    }

    if (text.startsWith("//", i)) {
      const end = lineEnd(i);
      add({ kind: "comment", ...at(i, end) });
      i = end;
      continue;
    }
    if (text.startsWith("/*", i)) {
      const end = paragraphEnd(i);
      let depth = 0;
      let j = i;
      while (j < end) {
        if (text.startsWith("/*", j)) { depth++; j += 2; }
        else if (text.startsWith("*/", j)) { depth--; j += 2; if (depth === 0) break; }
        else j++;
      }
      add({ kind: "comment", ...at(i, Math.min(j, end)) });
      i = Math.min(j, end);
      continue;
    }

    if (c === "`") {
      let k = 0;
      while (text[i + k] === "`") k++;
      if (k === 2) { i += 2; continue; } // empty raw
      const fence = "`".repeat(k);
      const close = text.indexOf(fence, i + k);
      if (close !== -1 && close < paragraphEnd(i)) {
        // A block (``` ```) may name its language right after the opening backticks.
        let openEnd = i + k;
        if (k >= 3) while (isIdentChar(text[openEnd])) openEnd++;
        add({ kind: "raw", ...at(i, close + k), open: [base + i, base + openEnd], close: [base + close, base + close + k] });
        i = close + k;
        continue;
      }
      i += k;
      continue;
    }

    if (c === "$") {
      let j = i + 1;
      const end = paragraphEnd(i);
      while (j < end && text[j] !== "$") j += text[j] === "\\" ? 2 : 1;
      if (j < end) {
        add({ kind: "math", ...at(i, j + 1) });
        i = j + 1;
        continue;
      }
    }

    // Code: `#name`, `#name.field(args)`, `#(…)`, `#{…}`; a content block `[…]` right after
    // it is markup again, with only its brackets shown as code.
    if (c === "#") {
      let j = i + 1;
      if (isIdentStart(text[j])) {
        while (isIdentChar(text[j])) j++;
        if (STATEMENTS.has(text.slice(i + 1, j))) {
          const end = lineEnd(i);
          add({ kind: "code", ...at(i, end) });
          i = end;
          continue;
        }
        for (;;) {
          if (text[j] === "." && isIdentStart(text[j + 1])) {
            j++;
            while (isIdentChar(text[j])) j++;
          } else if (text[j] === "(") {
            const end = balanced(j);
            if (end === -1) break;
            j = end;
          } else break;
        }
      } else if (text[j] === "(" || text[j] === "{") {
        const end = balanced(j);
        if (end === -1) { i++; continue; }
        j = end;
      } else {
        i++;
        continue;
      }
      if (text[j] === "[" && balanced(j) !== -1) {
        add({ kind: "code", ...at(i, j + 1) });
        open.push({ kind: "content", at: j });
        i = j + 1;
        continue;
      }
      add({ kind: "code", ...at(i, j) });
      i = j;
      continue;
    }
    if (c === "]") {
      const index = open.map((o) => o.kind).lastIndexOf("content");
      if (index !== -1) {
        open = open.slice(0, index); // anything still open inside it ends with it
        add({ kind: "code", ...at(i, i + 1) });
        i++;
        continue;
      }
    }

    if (c === "<") {
      const label = /^<[\p{L}\p{N}_\-:.]+>/u.exec(text.slice(i, i + 256));
      if (label) {
        add({ kind: "label", ...at(i, i + label[0].length) });
        i += label[0].length;
        continue;
      }
    }
    if (c === "@" && !isWordChar(text[i - 1])) {
      const ref = /^@[\p{L}\p{N}_\-:.]*[\p{L}\p{N}_]/u.exec(text.slice(i, i + 256));
      if (ref) {
        add({ kind: "label", ...at(i, i + ref[0].length) });
        i += ref[0].length;
        continue;
      }
    }

    if (text.startsWith("---", i)) { add({ kind: "glyph", ...at(i, i + 3), glyph: "—" }); i += 3; continue; }
    if (text.startsWith("--", i)) { add({ kind: "glyph", ...at(i, i + 2), glyph: "–" }); i += 2; continue; }
    if (text.startsWith("...", i)) { add({ kind: "glyph", ...at(i, i + 3), glyph: "…" }); i += 3; continue; }

    // `*` and `_` mark strong and emphasis, except inside a word (snake_case, 2*3).
    if ((c === "*" || c === "_") && !(isWordChar(text[i - 1]) && isWordChar(text[i + 1]))) {
      const kind = c === "*" ? "strong" : "emph";
      const index = open.map((o) => o.kind).lastIndexOf(kind);
      const inner = open.map((o) => o.kind).lastIndexOf("content");
      if (index !== -1 && index > inner) {
        const start = open[index].at;
        open = open.slice(0, index);
        add({ kind, ...at(start, i + 1), open: [base + start, base + start + 1], close: [base + i, base + i + 1] });
      } else {
        open.push({ kind, at: i });
      }
      i++;
      continue;
    }

    i++;
  }
  return spans.sort((a, b) => a.from - b.from || b.to - a.to);
}
