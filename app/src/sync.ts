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
  | { op: "redo" }
  | { op: "sync" }
  | { op: "marks" }
  | CommentOp;

/** Something done to Comments. Sent in order with the typing; Rust closes the open burst first. */
export type CommentOp =
  | { op: "comment"; from: number; to: number; sender: string; body: string }
  | { op: "reply"; comment: string; sender: string; body: string }
  | { op: "editComment"; id: string; body: string }
  | { op: "setCommentSender"; id: string; sender: string }
  | { op: "resolve"; id: string; resolved: boolean }
  | { op: "deleteComment"; id: string };

type Patch = { from: number; to: number; insert: string };

/** A Reply, as the Comments panel shows it (work.rs `ReplyMark`). */
export type ReplyMark = {
  id: string;
  sender: string;
  name: string;
  colour: string;
  body: string;
  timeMs: number;
  /** The other version of its body, if two Devices edited it without seeing each other. */
  otherBody: string | null;
};

/** A Comment: its highlight, and what the Comments panel shows (work.rs `Mark`). Positions are UTF-16, in the editor's text. */
export type Mark = {
  id: string;
  /** Empty: the Comment is detached. */
  ranges: [number, number][];
  sender: string;
  name: string;
  /** A colour slot, "author" or "unknown". */
  colour: string;
  body: string;
  resolved: boolean;
  /** The text it was attached to, as selected. */
  quote: string;
  timeMs: number;
  replies: ReplyMark[];
  otherBody: string | null;
};

export type WorkState = {
  len: number;
  patches: Patch[];
  burst: string | null;
  saveError: string | null;
  textFileProblem: string | null;
  recoveryProblem: string | null;
  /** Goes up each time something changed outside the app: the editor then asks to take it in. */
  outside: number;
  /** What taking it in brought (only in the answer to that request). */
  tookIn: string | null;
  syncProblem: string | null;
  /** The Work's History moved away underneath the tab (on another Device). */
  moved: Whereabouts | null;
  /** A Style it uses changed in the Style library since the Work recorded it. */
  styleNotice: string | null;
  /** How many Changes the History holds: when it goes up, the highlights are asked for again. */
  changes: number;
  /** The Comments' highlights (in the answer to a batch that asked for them). */
  marks: Mark[] | null;
};

export type Whereabouts = { kind: "renamed"; to: string } | { kind: "trashed" } | { kind: "gone" } | { kind: "diverged" };

/** Marks the editor's own changes from Rust, so they aren't sent back. */
export const fromHistory = Annotation.define<boolean>();

/** Something waiting to be sent. Positions are in the text as it was just before it. */
type Item =
  | { kind: "edit"; changes: ChangeSet; event: string; origin: Origin }
  | { kind: "cursor"; pos: number }
  | { kind: "undo" }
  | { kind: "redo" }
  | { kind: "sync" }
  | { kind: "marks" }
  | { kind: "comment"; op: CommentOp };

const EVENTS = ["move.drop", "input.paste", "input.drop", "delete.cut"];

/** Can Rust's text end up different from the editor's after this? Then nothing is sent after it in the same batch. */
const mayDiffer = (item: Item) =>
  item.kind === "undo" || item.kind === "redo" || item.kind === "sync" || (item.kind === "edit" && item.origin !== "input");

/** An edit as ops for Rust. Several changes are sent last-first, so each one's positions stay right. */
function toOps(item: Item): Op[] {
  switch (item.kind) {
    case "cursor":
      return [{ op: "cursor", pos: item.pos }];
    case "undo":
    case "redo":
    case "sync":
    case "marks":
      return [{ op: item.kind }];
    case "comment":
      return [item.op];
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
  /** The Comments' highlights, fitted to the editor's text as it is now. */
  marks(marks: Mark[]): void;
  error(message: string): void;
};

/**
 * Highlights fitted around edits made since Rust worked them out. A highlight doesn't grow at
 * its edges (text typed there is outside it), only when text is typed strictly inside.
 */
export function fitMarks(marks: Mark[], edits: ChangeSet[]): Mark[] {
  return marks.map((mark) => {
    let ranges = mark.ranges;
    for (const edit of edits) {
      ranges = ranges
        .map(([from, to]): [number, number] => [edit.mapPos(from, 1), edit.mapPos(to, -1)])
        .filter(([from, to]) => from < to);
    }
    return { ...mark, ranges };
  });
}

export class WorkSync {
  private queue: Item[] = [];
  private inFlight = false;
  private burst: string | null = null;
  /** Whose undo ran last: the editor's (inside a draft) or the History's (a Revert). Redo follows it. */
  private lastUndo: "editor" | "history" | null = null;
  /** The last `outside` count asked about (see `WorkState.outside`). */
  private outside = 0;
  /** The History's size when the highlights were last asked for (see `WorkState.changes`). */
  private changes = -1;

  constructor(
    /** The Work's ID; it changes when the Work is renamed. */
    public id: string,
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

  /** The timer on the Rust side saved something, closed the burst, or noticed a change from outside. */
  status(state: WorkState) {
    this.noteBurst(state.burst);
    this.noteOutside(state.outside);
    this.noteChanges(state.changes);
    this.events.state(state);
  }

  /**
   * Does something to Comments, in order with the typing (a new Comment's positions are in the
   * text as it is now). The History grows, so the highlights and the panel follow by themselves.
   */
  comment(op: CommentOp) {
    this.queue.push({ kind: "comment", op });
    void this.flush();
  }

  /** Asks for the Comments' highlights (when the tab opens, or a Reviewer's colour changed). */
  requestMarks() {
    if (!this.queue.some((item) => item.kind === "marks")) {
      this.queue.push({ kind: "marks" });
      void this.flush();
    }
  }

  /**
   * The History changed (a burst was recorded, something taken in, a Move, a Revert): where
   * the Comments are may differ from what the editor worked out by itself, so it asks again.
   */
  private noteChanges(changes: number) {
    if (changes <= this.changes) return;
    this.changes = changes;
    this.requestMarks();
  }

  /**
   * Something changed outside the app (Syncthing, another editor). It's taken in through the
   * queue, in order with the typing, so the patches it brings fit around everything typed.
   */
  private noteOutside(outside: number) {
    if (outside <= this.outside) return;
    this.outside = outside;
    if (!this.queue.some((item) => item.kind === "sync")) {
      this.queue.push({ kind: "sync" });
      void this.flush();
    }
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
        } else if (item.kind === "comment" && item.op.op === "comment") {
          item.op = { ...item.op, from: patch.mapPos(item.op.from, 1), to: patch.mapPos(item.op.to, -1) };
        }
      }
      this.view.dispatch({ changes: patch, annotations: [fromHistory.of(true), Transaction.addToHistory.of(false)] });
    }
    if (state.len !== expected) throw new Error(`The editor and the History disagree about the text's length (${expected} and ${state.len}).`);
    if (state.marks) {
      // For the text right after the batch (and its patches): fitted around what was typed since.
      const since = this.queue.flatMap((item) => (item.kind === "edit" ? [item.changes] : []));
      this.events.marks(fitMarks(state.marks, since));
    }
    this.noteBurst(state.burst, undone);
    this.noteOutside(state.outside);
    this.noteChanges(state.changes);
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
      this.requestMarks();
    } catch (e) {
      this.events.error(String(e));
    }
  }
}
