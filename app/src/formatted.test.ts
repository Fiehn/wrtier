import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { spansForTests } from "./formatted";
import { parseMarkup } from "./markup";

describe("the Formatted view's spans", () => {
  it("parsing only the edited paragraph always gives what parsing everything gives", () => {
    // A fixed-seed run of random edits: typing markup, deleting, joining and splitting paragraphs.
    const pieces = ["*", "_", "`", "= ", "\n", "\n\n", "#quote[", "]", "--", "word ", "\\", "// note", "😊", "$", "<a>", "@a "];
    let state = EditorState.create({ doc: "= One\nIt was *dark*.\n\n_Two_ -- #quote[three]\n\nfour `raw`", extensions: [spansForTests] });
    let seed = 7;
    const next = (n: number) => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed % Math.max(n, 1);
    };
    for (let step = 0; step < 400; step++) {
      const len = state.doc.length;
      const from = next(len + 1);
      const to = Math.min(len, from + (next(3) === 0 ? next(6) : 0));
      const insert = next(4) === 0 ? "" : pieces[next(pieces.length)];
      state = state.update({ changes: { from, to, insert } }).state;
      expect(state.field(spansForTests), `after step ${step}: ${JSON.stringify(state.doc.toString())}`)
        .toEqual(parseMarkup(state.doc.toString()));
    }
  });
});
