import { beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { EditorState } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
import { setTints, tintAt, tintLabel, tintsForTests } from "./sessiontints";
import { WorkSync, type Tint, type WorkState } from "./sync";

const tint = (rank: number, ranges: [number, number][]): Tint => ({ ranges, rank, device: "Laptop", timeMs: new Date(2026, 10, 9, 14, 2).getTime() });

describe("Colour text by Session in the editor", () => {
  it("says who wrote the text, and when", () => {
    const now = new Date(2026, 10, 9, 18, 0).getTime();
    const label = tintLabel("Laptop", new Date(2026, 10, 9, 14, 2).getTime(), now);
    expect(label).toMatch(/^Laptop · today 14.02$/);
    expect(tintLabel("Phone", new Date(2026, 10, 8, 9, 30).getTime(), now)).toMatch(/^Phone · yesterday /);
  });

  it("tints each Session's text and moves the tints along with the typing", () => {
    let s = EditorState.create({ doc: "abcdef", extensions: [tintsForTests] })
      .update({ effects: setTints.of([tint(0, [[3, 6]]), tint(1, [[0, 3]])]) }).state;
    s = s.update({ changes: { from: 0, insert: "XY" } }).state; // "XYabcdef"
    expect(tintAt(s as unknown as EditorView["state"], 2)).toMatch(/^Laptop · /); // "a"
    expect(tintAt(s as unknown as EditorView["state"], 0)).toBeNull(); // typed since: untinted until Rust's next answer
    const classes: string[] = [];
    s.field(tintsForTests).between(0, s.doc.length, (_from, _to, value) => {
      classes.push(value.spec.class as string);
    });
    expect(classes).toEqual(["cm-tint cm-tint-1", "cm-tint cm-tint-0"]);
    s = s.update({ effects: setTints.of([]) }).state; // switched off
    expect(s.field(tintsForTests).size).toBe(0);
  });
});

describe("asking Rust for the tints", () => {
  /** What each batch sent to Rust asked for. */
  let batches: string[][];
  let history: number;

  const state = (tints: Tint[] | null = null): WorkState => ({
    len: 6, patches: [], burst: null, saveError: null, textFileProblem: null, recoveryProblem: null, outside: 0,
    tookIn: null, syncProblem: null, moved: null, styleNotice: null, changes: history, notices: 0, marks: null, tints,
  });

  beforeEach(() => {
    batches = [];
    history = 5;
    (invoke as Mock).mockImplementation(async (_command: string, args: { ops: { op: string }[] }) => {
      const ops = args.ops.map((o) => o.op);
      batches.push(ops);
      return state(ops.includes("tints") ? [tint(0, [[0, 6]])] : null);
    });
  });

  it("never asks while the switch is off, and asks whenever the History grows while it's on", async () => {
    const view = { state: EditorState.create({ doc: "abcdef" }), dispatch: () => {} } as unknown as EditorView;
    const got: Tint[][] = [];
    const sync = new WorkSync("Rain", view, { state: () => {}, burstClosed: () => {}, marks: () => {}, tints: (t) => got.push(t), grew: () => {}, error: (m) => { throw new Error(m); } });

    sync.status(state()); // the History grew: the highlights are asked for, the tints aren't
    await sync.idle();
    expect(batches).toEqual([["marks"]]);

    sync.setTinting(true);
    await sync.idle();
    expect(batches[1]).toEqual(["tints"]);
    expect(got).toEqual([[tint(0, [[0, 6]])]]);

    history = 6;
    sync.status(state());
    await sync.idle();
    expect(batches.slice(2).flat()).toEqual(["marks", "tints"]);

    sync.setTinting(false);
    expect(got[got.length - 1]).toEqual([]); // cleared
    const before = batches.length;
    history = 7;
    sync.status(state());
    await sync.idle();
    expect(batches.slice(before).flat()).toEqual(["marks"]);
  });
});
