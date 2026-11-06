// Keeps the editor and the History (on the Rust side) in step.
//
// The editor shows every edit at once and sends it to Rust, in order, in batches: while one
// batch is on its way, new edits wait and go together next. Rust may answer with patches (an
// undo that reverted a Change); those are fitted around anything typed in the meantime, the
// way collaborative editors do it. If the two ever disagree, the editor takes the History's text.

import { invoke } from "@tauri-apps/api/core";
import { Annotation, ChangeSet, Transaction } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { redo, redoDepth, undo, undoDepth } from "@codemirror/commands";

// The same shapes as the Rust side's (work.rs).
type Origin = "input" | "undo" | "redo";
type Op =
  | { op: "cursor"; pos: number }
  | { op: "type"; from: number; to: number; text: string; origin: Origin }
  | { op: "delete"; from: number; to: number; origin: Origin }
  | { op: "paste"; from: number; to: number; text: string }
  | { op: "cut"; from: number; to: number }
  | { op: "move"; from: number; to: number; at: number }
  | { op: "undo" }
  | { op: "redo" };

type Patch = { from: number; to: number; insert: string };

export type WorkState = {
  len: number;
  patches: Patch[];
  burst: string | null;
  saveError: string | null;
  textFileProblem: string | null;
};

/** Marks the editor's own changes from Rust, so they aren't sent back. */
export const fromHistory = Annotation.define<boolean>();

/** Something waiting to be sent. Positions are in the text as it was just before it. */
type Item =
  | { kind: "edit"; changes: ChangeSet; event: string; origin: Origin }
  | { kind: "cursor"; pos: number }
  | { kind: "undo" }
  | { kind: "redo" };

const EVENTS = ["move.drop", "input.paste", "input.drop", "delete.cut"];

/** Can Rust's text end up different from the editor's after this? Then nothing is sent after it in the same batch. */
const mayDiffer = (item: Item) =>
  item.kind === "undo" || item.kind === "redo" || (item.kind === "edit" && item.origin !== "input");

/** An edit as ops for Rust. Several changes are sent last-first, so each one's positions stay right. */
function toOps(item: Item): Op[] {
  switch (item.kind) {
    case "cursor":
      return [{ op: "cursor", pos: item.pos }];
    case "undo":
    case "redo":
      return [{ op: item.kind }];
  }
  const parts: { from: number; to: number; text: string }[] = [];
  item.changes.iterChanges((from, to, _fromB, _toB, inserted) => parts.push({ from, to, text: inserted.toString() }));
  if (item.event === "move.drop" && parts.length === 2) {
    const removed = parts.find((p) => p.text === "" && p.to > p.from);
    const added = parts.find((p) => p.from === p.to && p.text !== "");
    if (removed && added) return [{ op: "move", from: removed.from, to: removed.to, at: added.from }];
  }
  return parts.reverse().map((p): Op => {
    if (item.event === "input.paste" || item.event === "input.drop") return { op: "paste", ...p };
    if (item.event === "delete.cut") return { op: "cut", from: p.from, to: p.to };
    if (p.text === "") return { op: "delete", from: p.from, to: p.to, origin: item.origin };
    return { op: "type", ...p, origin: item.origin };
  });
}

export type SyncEvents = {
  /** What Rust said about the Work: errors to show, and whether the open burst changed. */
  state(state: WorkState): void;
  /** The open burst closed: the editor's undo steps no longer apply. */
  burstClosed(): void;
  error(message: string): void;
};

export class WorkSync {
  private queue: Item[] = [];
  private inFlight = false;
  private burst: string | null = null;
  /** Whose undo ran last: the editor's (inside a draft) or the History's (a Revert). Redo follows it. */
  private lastUndo: "editor" | "history" | null = null;

  constructor(
    readonly id: string,
    private view: EditorView,
    private events: SyncEvents,
  ) {}

  /** Every transaction in the editor comes through here. */
  track(tr: Transaction) {
    if (tr.annotation(fromHistory)) return;
    if (tr.docChanged) {
      const event = EVENTS.find((e) => tr.isUserEvent(e)) ?? "edit";
      const origin: Origin = tr.isUserEvent("undo") ? "undo" : tr.isUserEvent("redo") ? "redo" : "input";
      this.queue.push({ kind: "edit", changes: tr.changes, event, origin });
    } else if (tr.selection) {
      // Only where the cursor ends up matters: a burst closes when the cursor leaves it.
      const pos = tr.newSelection.main.head;
      const last = this.queue[this.queue.length - 1];
      if (last?.kind === "cursor") last.pos = pos;
      else this.queue.push({ kind: "cursor", pos });
    } else {
      return;
    }
    void this.flush();
  }

  /**
   * Ctrl+Z. While a burst is open, the editor's own undo steps through its draft (they're only
   * ever that draft's: they're cleared when a burst becomes a Change). Otherwise the History
   * reverts the latest Change. (Not just "the editor has undo steps": it counts cursor moves too.)
   */
  undo(): boolean {
    if (this.burst !== null && undoDepth(this.view.state) > 0) {
      this.lastUndo = "editor";
      return undo(this.view);
    }
    this.lastUndo = "history";
    this.command("undo");
    return true;
  }

  /** Ctrl+Shift+Z: redoes with whichever undo ran last. */
  redo(): boolean {
    if (this.lastUndo === "editor" && redoDepth(this.view.state) > 0) return redo(this.view);
    this.command("redo");
    return true;
  }

  /** Undo or redo by the History, beyond what the editor can undo itself. */
  private command(kind: "undo" | "redo") {
    this.queue.push({ kind });
    void this.flush();
  }

  /** Waits until everything typed so far has reached Rust. */
  async idle() {
    while (this.inFlight || this.queue.length > 0) {
      await new Promise((resolve) => setTimeout(resolve, 5));
    }
  }

  /** The timer on the Rust side saved something, or closed the burst. */
  status(state: WorkState) {
    this.noteBurst(state.burst);
    this.events.state(state);
  }

  /**
   * A burst that became a Change takes its undo steps with it. One the editor's undo emptied
   * was only dropped, so its steps stay: redo can type it again.
   */
  private noteBurst(burst: string | null, undone = false) {
    if (this.burst !== null && burst !== this.burst && !undone) this.events.burstClosed();
    this.burst = burst;
  }

  private async flush() {
    if (this.inFlight || this.queue.length === 0) return;
    const until = this.queue.findIndex(mayDiffer);
    const items = this.queue.splice(0, until === -1 ? this.queue.length : until + 1);
    // The text's length once these are applied: what's on screen, minus what's still waiting.
    let len = this.view.state.doc.length;
    for (const item of this.queue) if (item.kind === "edit") len -= item.changes.newLength - item.changes.length;
    const undone = items.some((item) => item.kind === "edit" && item.origin === "undo");
    this.inFlight = true;
    try {
      const state = await invoke<WorkState>("edit", { id: this.id, ops: items.flatMap(toOps) });
      this.receive(state, len, undone);
    } catch (e) {
      this.events.error(String(e));
      await this.resync();
    } finally {
      this.inFlight = false;
      void this.flush();
    }
  }

  /** Rust's answer to a batch. Its patches are for the text right after that batch. */
  private receive(state: WorkState, len: number, undone: boolean) {
    let expected = len;
    if (state.patches.length > 0) {
      let patch = ChangeSet.of(state.patches, len);
      expected = patch.newLength;
      // Fit the patch around what was typed since the batch left, and those edits around the patch.
      for (const item of this.queue) {
        if (item.kind === "edit") {
          const mine = item.changes;
          item.changes = mine.map(patch);
          patch = patch.map(mine, true);
        } else if (item.kind === "cursor") {
          item.pos = patch.mapPos(item.pos);
        }
      }
      this.view.dispatch({ changes: patch, annotations: [fromHistory.of(true), Transaction.addToHistory.of(false)] });
    }
    if (state.len !== expected) throw new Error(`The editor and the History disagree about the text's length (${expected} and ${state.len}).`);
    this.noteBurst(state.burst, undone);
    this.events.state(state);
  }

  /** The editor fell out of step: show the History's text. Anything not sent yet is dropped. */
  private async resync() {
    this.queue = [];
    try {
      const { text, state } = await invoke<{ text: string; state: WorkState }>("work_text", { id: this.id });
      const head = Math.min(this.view.state.selection.main.head, text.length);
      this.view.dispatch({
        changes: { from: 0, to: this.view.state.doc.length, insert: text },
        selection: { anchor: head },
        annotations: [fromHistory.of(true), Transaction.addToHistory.of(false)],
      });
      this.burst = state.burst;
      this.events.burstClosed();
      this.events.state(state);
    } catch (e) {
      this.events.error(String(e));
    }
  }
}
