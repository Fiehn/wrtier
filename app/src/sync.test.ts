import { describe, expect, it, vi } from "vitest";
import { EditorState, Text, type TransactionSpec } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

const backend = vi.hoisted(() => ({ handle: (_cmd: string, _args: any): Promise<unknown> => Promise.reject(new Error("no backend")) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args: unknown) => backend.handle(cmd, args) }));

import { difference, WorkSync, type WorkState } from "./sync";
import { INCLUSION } from "./inclusions";

/** Something that holds a call until it's let go. */
function gate() {
  let open!: () => void;
  const shut = new Promise<void>((resolve) => (open = resolve));
  return { shut, open };
}

/** Just enough of the Rust side (work.rs) to keep a Work's text and answer the editor. */
class FakeRust {
  batches: any[][] = [];
  /** The next batch fails before doing anything. */
  failNext: string | null = null;
  /** Holds the next batch (or `work_text`) until opened. */
  holdEdit: Promise<void> | null = null;
  holdText: Promise<void> | null = null;
  workTextFails: string | null = null;

  constructor(public text: string) {
    backend.handle = (cmd, args) => this.handle(cmd, args);
  }

  state(extra: Partial<WorkState> = {}): WorkState {
    return {
      len: this.text.length, patches: [], burst: null, saveError: null, textFileProblem: null, recoveryProblem: null,
      outside: 0, tookIn: null, syncProblem: null, moved: null, styleNotice: null, changes: 0, notices: 0,
      highlights: null, tints: null, dismissed: null, ...extra,
    };
  }

  async handle(cmd: string, args: any): Promise<unknown> {
    if (cmd === "work_text") {
      const hold = this.holdText;
      this.holdText = null;
      await hold;
      if (this.workTextFails) throw this.workTextFails;
      return { text: this.text, state: this.state() };
    }
    if (cmd !== "edit") throw new Error(`unexpected ${cmd}`);
    this.batches.push(args.ops);
    const hold = this.holdEdit;
    this.holdEdit = null;
    await hold;
    if (this.failNext) {
      const message = this.failNext;
      this.failNext = null;
      throw message;
    }
    const extra: Partial<WorkState> = {};
    for (const op of args.ops) {
      switch (op.op) {
        case "type":
        case "paste":
          this.text = this.text.slice(0, op.from) + op.text + this.text.slice(op.to);
          break;
        case "delete":
        case "cut":
          this.text = this.text.slice(0, op.from) + this.text.slice(op.to);
          break;
        case "include":
          if (this.text[op.at] !== INCLUSION) throw "there's no Inclusion there to record what it includes";
          break;
        case "locate":
          extra.located = [0, 3];
          break;
        case "stretch":
          extra.stretch = this.text.slice(op.from, op.to);
          break;
      }
    }
    return this.state(extra);
  }

  ops(kind: string) {
    return this.batches.flat().filter((op) => op.op === kind);
  }
}

/** A WorkSync on a stand-in for the editor: a real EditorState, with every transaction tracked. */
function editor(doc: string) {
  const events = { state: vi.fn(), burstClosed: vi.fn(), highlights: vi.fn(), tints: vi.fn(), grew: vi.fn(), error: vi.fn() };
  const view = {
    state: EditorState.create({ doc }),
    dispatch(spec: TransactionSpec) {
      const tr = view.state.update(spec);
      view.state = tr.state;
      sync.track(tr);
    },
  };
  const sync = new WorkSync("Rain", view as unknown as EditorView, events);
  const type = (at: number, text: string) => view.dispatch({ changes: { from: at, insert: text }, userEvent: "input.type" });
  const text = () => view.state.doc.toString();
  return { view, sync, events, type, text };
}

describe("WorkSync", () => {
  it("sends typing in order, and idle() waits until it has all arrived", async () => {
    const rust = new FakeRust("The rain had stopped.");
    const { sync, type, text } = editor(rust.text);
    const held = gate();
    rust.holdEdit = held.shut;
    type(4, "cold ");
    type(9, "grey ");
    let idle = false;
    const waiting = sync.idle().then(() => (idle = true));
    await Promise.resolve();
    expect(idle).toBe(false);
    held.open();
    await waiting;
    expect(sync.isIdle()).toBe(true);
    expect(rust.text).toBe("The cold grey rain had stopped.");
    expect(text()).toBe(rust.text);
    expect(rust.batches).toHaveLength(2);
  });

  it("keeps a Comment, a locate and typing queued behind a batch that fails, fitted to the History's text", async () => {
    const rust = new FakeRust("The rain had stopped.");
    const { sync, events, type, text } = editor(rust.text);
    const held = gate();
    rust.holdEdit = held.shut;
    rust.failNext = "The disk is full.";
    type(0, "X"); // this batch fails: Rust never has the X
    expect(text()).toBe("XThe rain had stopped.");
    type(22, "!");
    sync.comment({ op: "comment", from: 5, to: 9, sender: "author", body: "Which rain?" }); // "rain"
    const located = sync.locate("a", null);
    held.open();
    await sync.idle();

    expect(events.error).toHaveBeenCalledWith("The disk is full.");
    expect(rust.text).toBe("The rain had stopped.!");
    expect(text()).toBe(rust.text);
    const [comment] = rust.ops("comment");
    expect(rust.text.slice(comment.from, comment.to)).toBe("rain");
    await expect(located).resolves.toEqual([0, 3]);
  });

  it("keeps what's typed while it waits for the History's text", async () => {
    const rust = new FakeRust("The rain had stopped.");
    const { sync, type, text } = editor(rust.text);
    rust.failNext = "Locked.";
    const held = gate();
    rust.holdText = held.shut;
    type(4, "cold "); // fails
    await vi.waitFor(() => expect(rust.batches).toHaveLength(1));
    await Promise.resolve();
    type(text().length - 1, " at last"); // while the resync waits: "…stopped at last."
    held.open();
    await sync.idle();
    expect(rust.text).toBe("The rain had stopped at last.");
    expect(text()).toBe(rust.text);
  });

  it("keeps the editor's text where it already agreed with the History", async () => {
    const rust = new FakeRust("The rain had stopped.");
    const { view, sync, type } = editor(rust.text);
    view.dispatch({ selection: { anchor: 12 } });
    await sync.idle();
    rust.failNext = "Locked.";
    type(4, "cold ");
    await sync.idle();
    expect(view.state.doc.toString()).toBe("The rain had stopped.");
    expect(view.state.selection.main.head).toBe(12);
  });

  it("drops an Inclusion record whose character never reached the History, and says so", async () => {
    const rust = new FakeRust("Part one.");
    const { sync, events, type, text } = editor(rust.text);
    const held = gate();
    rust.holdEdit = held.shut;
    rust.failNext = "The disk is full.";
    type(9, INCLUSION);
    sync.include(9, "Chapter 1", null);
    type(0, "#");
    held.open();
    await sync.idle();
    expect(rust.ops("include")).toHaveLength(0);
    expect(events.error).toHaveBeenCalledWith(expect.stringMatching(/Inclusion/));
    expect(rust.text).toBe("#Part one.");
    expect(text()).toBe(rust.text);
  });

  it("answers what's waiting and says what was lost when the History's text can't be had", async () => {
    const rust = new FakeRust("The rain had stopped.");
    const { sync, events, type } = editor(rust.text);
    const held = gate();
    rust.holdEdit = held.shut;
    rust.failNext = "The Work was closed.";
    rust.workTextFails = "No such Work.";
    type(0, "X");
    type(1, "Y");
    sync.comment({ op: "reply", comment: "c1", sender: "author", body: "Yes." });
    const located = sync.locate("a", null);
    const stretched = sync.stretch(0, 3);
    held.open();
    await expect(located).resolves.toBeNull();
    await expect(stretched).resolves.toBeNull();
    await sync.idle();
    expect(events.error).toHaveBeenLastCalledWith("No such Work. (an edit and a change to Comments couldn't be kept.)");
  });

  it("keeps History actions queued behind a failing batch", async () => {
    const rust = new FakeRust("abc");
    const { sync, type } = editor(rust.text);
    const held = gate();
    rust.holdEdit = held.shut;
    rust.failNext = "Locked.";
    type(3, "d");
    sync.history({ op: "revert", changes: ["c1"], dependents: false });
    held.open();
    await sync.idle();
    expect(rust.ops("revert")).toHaveLength(1);
  });
});

describe("difference", () => {
  const apply = (from: string, to: string) => difference(Text.of(from.split("\n")), to).apply(Text.of(from.split("\n"))).toString();

  it("replaces only what lies between the common start and end", () => {
    const change = difference(Text.of(["The cold rain had stopped."]), "The rain had stopped.");
    const parts: [number, number, string][] = [];
    change.iterChanges((from, to, _a, _b, inserted) => parts.push([from, to, inserted.toString()]));
    expect(parts).toEqual([[4, 9, ""]]);
    expect(apply("abc\ndef", "abX\ndef")).toBe("abX\ndef");
    expect(apply("same", "same")).toBe("same");
    expect(apply("", "new")).toBe("new");
    expect(apply("aaa", "aa")).toBe("aa");
  });

  it("never splits a surrogate pair", () => {
    const change = difference(Text.of(["a😀b"]), "a😃b");
    change.iterChanges((from, to, _a, _b, inserted) => {
      expect([from, to, inserted.toString()]).toEqual([1, 3, "😃"]);
    });
  });
});
