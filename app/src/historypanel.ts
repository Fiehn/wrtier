// The History tab: the Work's Sessions, newest first under day headings, with its Milestones as
// dividers between them. Opening a Session shows what it did, as snippets of the text as it is
// now; clicking one selects that text in the editor.
//
// A Session, a snippet or a Preamble line can be reverted (or, once reverted, restored), and a
// Milestone returned to. Each first shows a preview inline, under what it's about; one preview
// at a time. The action itself goes through the editor's queue, as one Revert that Ctrl+Z can
// take back.
//
// "History of this text" narrows the list to a stretch of text (the selection, or the
// paragraph at the cursor), under a bar with ×. "Compare…" shows what changed between two of
// the Milestones and now instead of the list, until it's closed.
//
// Merge notices are pinned at the top of the whole History, each as a plain sentence with
// "Show me" (stepping through the places it's about) and Dismiss.
//
// Asked for beside the editor's queue (nothing here holds positions); only "go to this text",
// picking the text for "History of this text", and the actions go through it. Drawn again a
// moment after the History grows, keeping which Sessions are open, a preview, and a Milestone
// name being typed.

import type { HistoryOp } from "./sync";

// The same shapes as the Rust side's (vault/history.rs).
export type SessionRow = {
  key: string;
  changes: string[];
  device: string;
  startMs: number;
  endMs: number;
  continued: boolean;
  source: string | null;
  addedWords: number;
  removedWords: number;
  reverted: "no" | "partly" | "all";
  notIn: string[];
};
export type MilestoneRow = { id: string; name: string; timeMs: number; deleted: boolean; device: string };
export type NoticeView = {
  key: string;
  kind: "overlap" | "loop" | "externalEdit" | "preamble";
  device: string;
  timeMs: number;
  otherDevice: string | null;
  quote: string;
  places: { first: string; last: string | null }[];
};
export type HistoryView = { sessions: SessionRow[]; milestones: MilestoneRow[]; more: boolean; notices: NoticeView[] };
export type SegmentView = { kind: "plain" | "added" | "removed" | "moved" | "cut" | "elided"; text: string; faded: boolean; words: number };
export type MoveView = { back: boolean; chapter: string | null; after: string; earlier: boolean };
export type SnippetView = {
  segments: SegmentView[];
  changes: string[];
  find: { first: string; last: string | null } | null;
  moves: MoveView[];
  inEffect: boolean;
  revertedBy: { device: string; timeMs: number } | null;
};
export type FieldView = {
  by: string;
  change: string;
  record: string;
  field: string;
  value: string | null;
  before: string | null;
  takenBack: boolean;
  library: boolean;
  inEffect: boolean;
};
export type SnippetsView = { snippets: SnippetView[]; fields: FieldView[] };
export type GroupView = { device: string; startMs: number; endMs: number; addedWords: number; removedWords: number; changes: string[]; snippets: SnippetsView };
export type RevertPreview = {
  goes: SnippetsView;
  dependents: GroupView[];
  leftOut: { library: string[]; alreadyReverted: number; comments: number; milestones: string[] };
  nothing: boolean;
};
export type RestorePreview = { back: SnippetsView; places: number; nothing: boolean };
export type ComparisonView = { snippets: SnippetView[]; addedWords: number; removedWords: number; moves: number };
export type ReturnPreview = { since: ComparisonView; earlier: GroupView[]; detached: number; preamble: boolean; nothing: boolean };

export type HistoryHooks = {
  /** The Work in front, if it's a Work. */
  work(): string | null;
  /** `filtered`: only what touched the text "History of this text" is about. */
  load(work: string, limit: number, filtered: boolean): Promise<HistoryView>;
  snippets(work: string, changes: string[], filtered: boolean): Promise<SnippetsView>;
  /** What changed from one version to another: a Milestone's ID, or null for now. */
  compare(work: string, from: string | null, to: string | null): Promise<ComparisonView>;
  previewRevert(work: string, changes: string[]): Promise<RevertPreview>;
  previewRestore(work: string, changes: string[]): Promise<RestorePreview>;
  previewReturn(work: string, milestone: string): Promise<ReturnPreview>;
  /** Reverts, restores or returns, through the editor's queue. */
  act(work: string, op: HistoryOp): void;
  /** Selects some characters in the editor; false if they aren't in the text any more. */
  find(work: string, first: string, last: string | null): Promise<boolean>;
  nameMilestone(work: string): Promise<void>;
  renameMilestone(work: string, milestone: string, name: string): Promise<void>;
  setMilestoneDeleted(work: string, milestone: string, deleted: boolean): Promise<void>;
  /** Dismisses a Merge notice (recorded, so it goes on every Device). */
  dismissNotice(work: string, key: string): Promise<void>;
  /** "Open Preamble…": the dialog shows the other version of the raw Typst. */
  openPreamble(work: string): void;
  error(message: string): void;
};

/** How many Sessions load at first, and with each "Show older". */
const PAGE = 50;
/** How long after the History grows the tab is drawn again (typing records a Change every few seconds). */
const SETTLE_MS = 400;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
}

function button(label: string, run: () => void, title = ""): HTMLButtonElement {
  const b = el("button", "link", label);
  b.type = "button";
  if (title) b.title = title;
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    run();
  });
  return b;
}

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

/** "Today", "Yesterday", or the date. */
export function dayOf(ms: number, now = Date.now()): string {
  const day = new Date(ms);
  const start = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((start(new Date(now)) - start(day)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  const sameYear = day.getFullYear() === new Date(now).getFullYear();
  return day.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: sameYear ? undefined : "numeric" });
}

/** "+412 −58 words", leaving out a zero side. */
export function wordCounts(added: number, removed: number): string {
  const parts: string[] = [];
  if (added > 0) parts.push(`+${added.toLocaleString()}`);
  if (removed > 0) parts.push(`−${removed.toLocaleString()}`);
  if (parts.length === 0) return "no words";
  return `${parts.join(" ")} ${added + removed === 1 ? "word" : "words"}`;
}

const FIELD_NAMES: Record<string, string> = {
  looks: "Looks",
  styles: "Extra Styles",
  elements: "Story elements",
  title: "Title",
  author: "Author name",
  language: "Language",
  raw: "Raw Typst",
};

/** A Preamble edit as a line ("Title: The Rain (was Rain)"). Library copies are summed up apart. */
export function fieldLine(f: FieldView): string {
  const name = FIELD_NAMES[f.field] ?? f.field;
  const shown = (v: string | null) => (v === null || v.trim() === "" ? "none" : v.split("\n").map((s) => s.trim()).filter(Boolean).join(", "));
  const line = f.field === "raw"
    ? "Raw Typst edited"
    : `${name}: ${shown(f.value)}${f.before !== null ? ` (was ${shown(f.before)})` : ""}`;
  return f.takenBack ? `Taken back: ${line}` : line;
}

/** "Moved here from Chapter 3, after “…the harbour wall.”"; in a preview, where it would go. */
export function moveLine(m: MoveView, preview = false): string {
  const where = m.chapter ? m.chapter : m.earlier ? "earlier in the text" : "later in the text";
  const after = m.after ? `, after “${m.after}”` : "";
  if (preview) return `${m.back ? "Moves to" : "Moves back to"} ${where}${after}`;
  return `${m.back ? "Moved back from" : "Moved here from"} ${where}${after}`;
}

/** "Not reverted: 2 Comments, Milestone “Draft 2”, …", or null if nothing is left out. */
export function leftOutLine(l: RevertPreview["leftOut"]): string | null {
  const parts: string[] = [];
  if (l.comments > 0) parts.push(l.comments === 1 ? "1 Comment" : `${l.comments} Comments`);
  for (const m of l.milestones) parts.push(`Milestone “${m}”`);
  if (l.library.length > 0) parts.push(`the recorded copies of ${l.library.join(", ")}`);
  if (l.alreadyReverted > 0) parts.push(`${l.alreadyReverted} already reverted`);
  return parts.length > 0 ? `Not reverted: ${parts.join(", ")}.` : null;
}

/** A Merge notice as a plain sentence. `device` made the Change it's about (the later Move, the
 *  skipped Move, the External edit, the raw Typst used); `otherDevice` the other side. */
export function noticeLine(n: NoticeView): string {
  const quote = n.quote ? `“${n.quote}”` : "";
  switch (n.kind) {
    case "overlap": {
      const who = n.otherDevice === null || n.otherDevice === n.device ? `Two Moves on ${n.device}` : `${n.otherDevice} and ${n.device}`;
      const part = quote ? `The part both moved, ${quote},` : "The part both moved";
      return `${who} moved overlapping text without seeing each other's Move. ${part} went with ${n.device}'s, the later one; the rest of each Move still happened.`;
    }
    case "loop":
      return `${n.device}'s Move${quote ? ` of ${quote}` : ""} was skipped: another Move at the same time put the two passages inside each other. That text stayed where it was.`;
    case "externalEdit":
      return `The Text file was edited outside the app (on ${n.device}) from a version the History doesn't have, so the edit was worked out against the text as it was then. Check what it wrote${quote ? `: ${quote}` : "."}`;
    case "preamble": {
      const who = n.otherDevice === null || n.otherDevice === n.device ? `twice on ${n.device}` : `on ${n.otherDevice} and on ${n.device}`;
      return `The Preamble's raw Typst was edited ${who} without either seeing the other's edit. ${n.device}'s, the later one, is used; the other version is in the Preamble dialog, and saving the dialog settles it.`;
    }
  }
}

/** "Show me" before the first step; then which place comes next. */
export function showMeLabel(places: number, next: number | undefined): string {
  if (next === undefined) return places === 1 ? "Show me" : `Show me (${places} places)`;
  return `Next place (${next + 1} of ${places})`;
}

type Entry = { kind: "session"; at: number; row: SessionRow } | { kind: "milestone"; at: number; row: MilestoneRow };

/** Sessions and Milestones as one list, newest first. Milestones older than every Session shown
 *  wait for "Show older", unless there's nothing older. */
export function entries(view: HistoryView): Entry[] {
  const oldest = view.sessions.length > 0 ? view.sessions[view.sessions.length - 1].endMs : Infinity;
  const list: Entry[] = [
    ...view.sessions.map((row): Entry => ({ kind: "session", at: row.endMs, row })),
    ...view.milestones.filter((m) => !view.more || m.timeMs >= oldest).map((row): Entry => ({ kind: "milestone", at: row.timeMs, row })),
  ];
  // Newest first; at the same moment a Milestone goes above the Session it ends.
  return list.sort((a, b) => b.at - a.at || (a.kind === "milestone" ? -1 : 1));
}

export class HistoryPanel {
  private visible = false;
  /** The Work last drawn: another Work starts afresh. */
  private drawn: string | null = null;
  private limit = PAGE;
  private view: HistoryView | null = null;
  /** Open Sessions (by key) and their snippets, as last fetched. */
  private open = new Map<string, SnippetsView | null>();
  /** A Milestone being renamed: its text box survives redraws. */
  private renaming: { id: string; input: HTMLInputElement } | null = null;
  /** The preview shown, under what it's about (named by `anchor`): it survives redraws. */
  private preview: { anchor: string; element: HTMLElement } | null = null;
  /** "History of this text": the Work and a quote of the text it's narrowed to. */
  private filter: { work: string; quote: string } | null = null;
  /** A comparison shown instead of the list (null: now), and what it found once loaded. */
  private comparing: { from: string | null; to: string | null; view: ComparisonView | null } | null = null;
  /** The Compare picker, while it's open. */
  private picker: HTMLElement | null = null;
  /** The place each Merge notice's "Show me" goes to next (by key), once it's been clicked. */
  private stepping = new Map<string, number>();
  private timer: number | undefined;
  /** Counts loads, so an answer that arrives after a newer one started is dropped. */
  private loads = 0;

  constructor(
    private readonly list: HTMLElement,
    private readonly hooks: HistoryHooks,
  ) {}

  /** The tab is showing or not: it only loads while it is. */
  setVisible(visible: boolean) {
    const was = this.visible;
    this.visible = visible;
    if (visible && !was) void this.refresh();
  }

  /** Another tab is in front. */
  workChanged() {
    if (this.hooks.work() === this.drawn) return;
    this.reset();
    this.view = null;
    if (this.visible) void this.refresh();
    else this.list.replaceChildren();
  }

  /** Starts afresh: no filter, no comparison, nothing open. */
  private reset() {
    this.open.clear();
    this.renaming = null;
    this.preview = null;
    this.filter = null;
    this.comparing = null;
    this.picker = null;
    this.stepping.clear();
    this.limit = PAGE;
  }

  /** "History of this text": the list narrows to the text Rust now holds for this Work. */
  showTextHistory(work: string, quote: string) {
    this.reset();
    this.filter = { work, quote };
    this.view = null;
    void this.refresh();
  }

  private clearFilter() {
    this.filter = null;
    this.open.clear();
    this.preview = null;
    void this.refresh();
  }

  /** "Compare…": a picker of two versions (the Milestones and now). */
  startCompare() {
    const work = this.hooks.work();
    if (!work || !this.view) return;
    const milestones = this.view.milestones.filter((m) => !m.deleted);
    if (milestones.length === 0) {
      this.hooks.error("There's nothing to compare yet: name a Milestone first.");
      return;
    }
    const select = (label: string, chosen: string) => {
      const s = el("select");
      s.setAttribute("aria-label", label);
      const options = [...milestones].reverse().map((m) => {
        const o = el("option", "", m.name);
        o.value = m.id;
        return o;
      });
      const now = el("option", "", "Now");
      now.value = "";
      s.append(now, ...options);
      s.value = chosen;
      return s;
    };
    const from = select("From", milestones[milestones.length - 1].id);
    const to = select("To", "");
    const box = el("div", "history-picker");
    box.append(el("span", "", "Compare"), from, el("span", "", "with"), to);
    const go = el("button", "primary", "Compare");
    go.type = "button";
    go.addEventListener("click", () => this.compare(work, from.value || null, to.value || null));
    const cancel = el("button", "", "Cancel");
    cancel.type = "button";
    cancel.addEventListener("click", () => {
      this.picker = null;
      this.draw(work);
    });
    box.append(cancel, go);
    this.picker = box;
    this.draw(work);
  }

  /** Shows what changed from `from` to `to` (Milestone IDs, or null for now). */
  private compare(work: string, from: string | null, to: string | null) {
    this.picker = null;
    this.preview = null;
    this.comparing = { from, to, view: null };
    this.draw(work);
    void this.refresh();
  }

  private closeComparison() {
    this.comparing = null;
    void this.refresh();
  }

  /** A Work's History grew: drawn again once things settle, if it's the one shown. */
  grew(work: string) {
    if (!this.visible || work !== this.hooks.work()) return;
    // A comparison between two Milestones doesn't change; one with now does.
    if (this.comparing && this.comparing.from !== null && this.comparing.to !== null) return;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.refresh(), SETTLE_MS);
  }

  /** Loads the list (and every open Session's snippets) again and draws it. */
  async refresh() {
    const work = this.hooks.work();
    const load = ++this.loads;
    if (!work) {
      this.drawn = null;
      this.view = null;
      this.draw(null);
      return;
    }
    if (this.filter && this.filter.work !== work) this.filter = null;
    const filtered = this.filter !== null;
    try {
      const comparing = this.comparing;
      const compared = comparing ? await this.hooks.compare(work, comparing.from, comparing.to) : null;
      const view = await this.hooks.load(work, this.limit, filtered);
      const keys = new Set(view.sessions.map((s) => s.key));
      const open = [...this.open.keys()].filter((key) => keys.has(key));
      const fetched = await Promise.all(open.map((key) => this.hooks.snippets(work, view.sessions.find((s) => s.key === key)!.changes, filtered)));
      if (load !== this.loads) return; // a newer load has started
      if (comparing && this.comparing === comparing) comparing.view = compared;
      this.open = new Map(open.map((key, i) => [key, fetched[i]]));
      this.drawn = work;
      this.view = view;
      this.draw(work);
    } catch (e) {
      if (load === this.loads) this.hooks.error(String(e));
    }
  }

  private draw(work: string | null) {
    const scroll = this.list.parentElement?.scrollTop ?? 0;
    const children: HTMLElement[] = [];
    if (work && this.picker) children.push(this.picker);
    if (work && this.comparing) {
      children.push(...this.comparison(work, this.comparing));
      this.list.replaceChildren(...children);
      return;
    }
    if (work && this.filter) {
      const bar = el("div", "history-filter");
      bar.append(el("span", "", `History of “${this.filter.quote}”`), button("×", () => this.clearFilter(), "Show the whole History again"));
      children.push(bar);
    }
    // Merge notices, pinned at the top of the whole History.
    if (work && this.view && !this.filter) children.push(...this.view.notices.map((n) => this.notice(work, n)));
    if (!work || !this.view) {
      children.push(el("p", "muted", "Open a Work to see its History."));
    } else if (this.filter && this.view.sessions.length === 0) {
      children.push(el("p", "muted", "Nothing in the History touched this text."));
    } else if (this.view.sessions.length === 0 && this.view.milestones.length === 0) {
      children.push(el("p", "muted", "Nothing written yet. Each sitting you write in shows here."));
    } else {
      const names = new Map(this.view.milestones.map((m) => [m.id, m.name]));
      let day = "";
      for (const entry of entries(this.view)) {
        const heading = dayOf(entry.at);
        if (heading !== day) {
          day = heading;
          children.push(el("h3", "history-day", heading));
        }
        children.push(entry.kind === "session" ? this.session(work, entry.row, names) : this.milestone(work, entry.row));
      }
      if (this.view.more) {
        const older = button("Show older", () => {
          this.limit += PAGE;
          void this.refresh();
        });
        older.classList.add("history-older");
        children.push(older);
      }
    }
    this.list.replaceChildren(...children);
    if (this.list.parentElement) this.list.parentElement.scrollTop = scroll;
    if (this.renaming && document.activeElement !== this.renaming.input) this.renaming.input.focus();
  }

  /** The preview for `anchor`, if it's the one shown (to go right after it). */
  private previewFor(anchor: string): HTMLElement[] {
    return this.preview?.anchor === anchor ? [this.preview.element] : [];
  }

  private closePreview() {
    this.preview = null;
    this.draw(this.drawn);
  }

  // ---- Sessions ----

  private session(work: string, row: SessionRow, names: Map<string, string>): HTMLElement {
    const card = el("article", "history-session");
    card.classList.toggle("reverted", row.reverted === "all");
    const isOpen = this.open.has(row.key);
    card.classList.toggle("open", isOpen);

    const head = el("button", "history-session-head");
    head.type = "button";
    head.setAttribute("aria-expanded", String(isOpen));
    const time = row.startMs === row.endMs || clock(row.startMs) === clock(row.endMs) ? clock(row.endMs) : `${clock(row.startMs)}–${clock(row.endMs)}`;
    head.append(el("strong", "history-device", row.device), el("span", "history-time", time), el("span", "history-words", wordCounts(row.addedWords, row.removedWords)));
    const tags: string[] = [];
    if (row.source) tags.push(row.source);
    if (row.continued) tags.push("continued");
    if (row.reverted === "partly") tags.push("partly reverted");
    if (row.reverted === "all") tags.push("reverted");
    for (const m of row.notIn) tags.push(`not in ${names.get(m) ?? "a Milestone"}`);
    for (const tag of tags) head.append(el("span", "history-tag", tag));
    head.addEventListener("click", () => void this.toggle(work, row));
    card.append(head);

    if (isOpen) {
      const actions = el("div", "history-actions");
      if (row.reverted !== "all") actions.append(button("Revert this Session…", () => void this.showRevert(work, `session:${row.key}`, row.changes)));
      if (row.reverted !== "no") actions.append(button("Restore…", () => void this.showRestore(work, `session:${row.key}`, row.changes)));
      card.append(actions, ...this.previewFor(`session:${row.key}`));
      card.append(this.snippets(work, row.key, this.open.get(row.key) ?? null));
    }
    return card;
  }

  private async toggle(work: string, row: SessionRow) {
    if (this.open.has(row.key)) {
      this.open.delete(row.key);
      if (this.preview?.anchor.includes(row.key)) this.preview = null;
      this.draw(work);
      return;
    }
    this.open.set(row.key, null); // open at once, filled in when the snippets arrive
    this.draw(work);
    try {
      const shown = await this.hooks.snippets(work, row.changes, this.filter !== null);
      if (this.open.has(row.key) && this.drawn === work) {
        this.open.set(row.key, shown);
        this.draw(work);
      }
    } catch (e) {
      this.hooks.error(String(e));
    }
  }

  /** A Session's snippets and Preamble lines, each with Revert or Restore. */
  private snippets(work: string, key: string, shown: SnippetsView | null): HTMLElement {
    const box = el("div", "history-snippets");
    if (!shown) {
      box.append(el("p", "muted", "…"));
      return box;
    }
    shown.snippets.forEach((snippet, i) => {
      const anchor = `snippet:${key}:${i}`;
      box.append(this.snippet(work, snippet, "session", this.snippetActions(work, anchor, snippet.changes, snippet.inEffect, snippet.revertedBy)), ...this.previewFor(anchor));
    });
    shown.fields.filter((f) => !f.library).forEach((f, i) => {
      const anchor = `field:${key}:${i}`;
      const line = el("div", "history-field");
      line.append(el("span", "", fieldLine(f)), this.snippetActions(work, anchor, [f.by], f.inEffect, null));
      box.append(line, ...this.previewFor(anchor));
    });
    const library = shown.fields.filter((f) => f.library);
    if (library.length > 0) {
      const names = [...new Set(library.map((f) => f.record.slice(f.record.indexOf("/") + 1)))];
      box.append(el("p", "history-field quiet", `Recorded library changes: ${names.join(", ")}`));
    }
    if (shown.snippets.length === 0 && shown.fields.length === 0) box.append(el("p", "muted", "Nothing to show here any more."));
    return box;
  }

  /** "Revert…" while some of it is in effect; "Reverted by …" and "Restore…" once reverted. */
  private snippetActions(work: string, anchor: string, changes: string[], inEffect: boolean, revertedBy: SnippetView["revertedBy"]): HTMLElement {
    const actions = el("span", "history-snippet-actions");
    if (revertedBy) actions.append(el("span", "history-tag", `Reverted by ${revertedBy.device}, ${dayOf(revertedBy.timeMs)} ${clock(revertedBy.timeMs)}`));
    if (inEffect) actions.append(button("Revert…", () => void this.showRevert(work, anchor, changes)));
    if (revertedBy) actions.append(button("Restore…", () => void this.showRestore(work, anchor, changes)));
    return actions;
  }

  /** One snippet: in a Session, in a preview (not clickable, Moves said as what would happen),
   *  or in a comparison. */
  private snippet(work: string, snippet: SnippetView, mode: "session" | "preview" | "comparison", actions: HTMLElement | null = null): HTMLElement {
    const preview = mode === "preview";
    const box = el("div", "history-snippet");
    box.classList.toggle("reverted", mode === "session" && !snippet.inEffect);
    for (const move of snippet.moves) box.append(el("p", "history-move", moveLine(move, preview)));
    const text = el("p", "history-text");
    for (const seg of snippet.segments) {
      if (seg.kind === "cut") {
        text.append(el("span", "history-cut", "…"));
        continue;
      }
      if (seg.kind === "elided") {
        text.append(el("span", "history-cut", ` …+${seg.words.toLocaleString()} words… `));
        continue;
      }
      // A line break that changed shows as ¶, so it's visible.
      const shownText = seg.kind === "plain" ? seg.text : seg.text.replace(/\n/g, "¶\n");
      const span = el("span", seg.kind === "plain" ? "" : `history-${seg.kind}`, shownText);
      if (seg.faded) {
        span.classList.add("faded");
        span.title = seg.kind === "added" ? "Since deleted" : "Since back";
      }
      text.append(span);
    }
    box.append(text);
    if (actions) box.append(actions);
    if (snippet.find && !preview) {
      const find = snippet.find;
      box.classList.add("findable");
      box.title = "Show this in the text";
      box.addEventListener("click", () => {
        void this.hooks.find(work, find.first, find.last).then((found) => {
          if (!found) this.hooks.error("That text isn't in the Work any more.");
        });
      });
    }
    return box;
  }

  // ---- Previews ----

  /** Shows a preview under `anchor` once it's built (replacing any other). */
  private async showPreview(anchor: string, build: () => Promise<HTMLElement>) {
    try {
      const element = await build();
      this.preview = { anchor, element };
      this.draw(this.drawn);
      element.scrollIntoView({ block: "nearest" });
    } catch (e) {
      this.hooks.error(String(e));
    }
  }

  private previewBox(heading: string): HTMLElement {
    const box = el("div", "history-preview");
    box.append(el("p", "history-preview-heading", heading));
    box.addEventListener("click", (e) => e.stopPropagation());
    return box;
  }

  /** Cancel, and the button that does it. */
  private previewButtons(label: string, run: () => void): HTMLElement {
    const row = el("div", "history-preview-buttons");
    const cancel = el("button", "", "Cancel");
    cancel.type = "button";
    cancel.addEventListener("click", () => this.closePreview());
    const go = el("button", "primary", label);
    go.type = "button";
    go.addEventListener("click", () => {
      run();
      this.closePreview();
    });
    row.append(cancel, go);
    queueMicrotask(() => go.focus());
    return row;
  }

  private snippetList(work: string, view: SnippetsView | ComparisonView): HTMLElement[] {
    const out: HTMLElement[] = view.snippets.map((s) => this.snippet(work, s, "preview"));
    if ("fields" in view) for (const f of view.fields.filter((f) => !f.library)) out.push(el("p", "history-field", fieldLine(f)));
    return out;
  }

  private group(work: string, group: GroupView, tick: HTMLInputElement | null): HTMLElement {
    const box = el("div", "history-group");
    const head = el(tick ? "label" : "p", "history-group-head");
    if (tick) head.append(tick);
    head.append(el("strong", "", group.device), ` · ${dayOf(group.endMs)} ${clock(group.endMs)} · ${wordCounts(group.addedWords, group.removedWords)}`);
    box.append(head, ...this.snippetList(work, group.snippets));
    return box;
  }

  private showRevert(work: string, anchor: string, changes: string[]) {
    return this.showPreview(anchor, async () => {
      const p = await this.hooks.previewRevert(work, changes);
      if (p.nothing) {
        const box = this.previewBox("There's nothing here to revert: it's all reverted already, or it's only Comments and Milestones.");
        box.append(this.previewButtons("OK", () => {}));
        return box;
      }
      const box = this.previewBox("Reverting takes out what's struck through and brings back what's highlighted:");
      box.append(...this.snippetList(work, p.goes));
      let also: HTMLInputElement | null = null;
      if (p.dependents.length > 0) {
        const count = p.dependents.reduce((n, g) => n + g.changes.length, 0);
        also = el("input");
        also.type = "checkbox";
        also.checked = true;
        const label = el("label", "history-also");
        label.append(also, ` Also revert ${count === 1 ? "1 later change" : `${count} later changes`} made inside this text`);
        const list = el("div", "history-dependents");
        list.append(...p.dependents.map((g) => this.group(work, g, null)));
        also.addEventListener("change", () => list.classList.toggle("off", !also!.checked));
        box.append(label, list);
      }
      const left = leftOutLine(p.leftOut);
      if (left) box.append(el("p", "history-note", left));
      box.append(this.previewButtons("Revert", () => this.hooks.act(work, { op: "revert", changes, dependents: also?.checked ?? true })));
      return box;
    });
  }

  private showRestore(work: string, anchor: string, changes: string[]) {
    return this.showPreview(anchor, async () => {
      const p = await this.hooks.previewRestore(work, changes);
      if (p.nothing) {
        const box = this.previewBox("There's nothing to restore: none of it is reverted now.");
        box.append(this.previewButtons("OK", () => {}));
        return box;
      }
      const places = p.places === 1 ? "1 place" : `${p.places} places`;
      const box = this.previewBox(`Restoring takes back the whole Revert, so it brings back ${places}:`);
      box.append(...this.snippetList(work, p.back));
      box.append(this.previewButtons(p.places === 1 ? "Restore" : `Restore ${places}`, () => this.hooks.act(work, { op: "restore", changes })));
      return box;
    });
  }

  private showReturn(work: string, row: MilestoneRow) {
    return this.showPreview(`milestone:${row.id}`, async () => {
      const p = await this.hooks.previewReturn(work, row.id);
      if (p.nothing) {
        const box = this.previewBox(`The Work is already as it was at “${row.name}”.`);
        box.append(this.previewButtons("OK", () => {}));
        return box;
      }
      const moves = p.since.moves === 0 ? "" : p.since.moves === 1 ? ", 1 Move" : `, ${p.since.moves} Moves`;
      const box = this.previewBox(`Back to “${row.name}”: ${wordCounts(p.since.addedWords, p.since.removedWords)}${moves}. What comes back is highlighted, what goes is struck through:`);
      box.append(...this.snippetList(work, p.since));
      const keep: { tick: HTMLInputElement; changes: string[] }[] = [];
      if (p.earlier.length > 0) {
        box.append(el("p", "history-note", `Made before “${row.name}” on another Device that hadn't synced yet, so not part of it. Tick any to keep:`));
        for (const g of p.earlier) {
          const tick = el("input");
          tick.type = "checkbox";
          keep.push({ tick, changes: g.changes });
          box.append(this.group(work, g, tick));
        }
      }
      if (p.preamble) box.append(el("p", "history-note", "The Preamble goes back to how it was then too."));
      if (p.detached > 0) box.append(el("p", "history-note", `${p.detached === 1 ? "1 Comment" : `${p.detached} Comments`} will be detached (attached again if you undo this).`));
      box.append(this.previewButtons(`Return to “${row.name}”`, () => this.hooks.act(work, {
        op: "returnTo",
        milestone: row.id,
        keep: keep.filter((k) => k.tick.checked).flatMap((k) => k.changes),
      })));
      return box;
    });
  }

  // ---- Comparing ----

  private comparison(work: string, c: { from: string | null; to: string | null; view: ComparisonView | null }): HTMLElement[] {
    const name = (id: string | null) => (id === null ? "now" : `“${this.view?.milestones.find((m) => m.id === id)?.name ?? "a Milestone"}”`);
    const bar = el("div", "history-filter");
    const title = `${name(c.from).replace(/^now$/, "Now")} → ${name(c.to)}`;
    const counts = c.view
      ? `: ${wordCounts(c.view.addedWords, c.view.removedWords)}${c.view.moves === 0 ? "" : c.view.moves === 1 ? ", 1 Move" : `, ${c.view.moves} Moves`}`
      : "";
    bar.append(el("span", "", title + counts), button("×", () => this.closeComparison(), "Back to the History"));
    const out: HTMLElement[] = [bar];
    if (!c.view) out.push(el("p", "muted", "Comparing…"));
    else if (c.view.snippets.length === 0) out.push(el("p", "muted", "No differences."));
    else out.push(...c.view.snippets.map((s) => this.snippet(work, s, "comparison")));
    return out;
  }

  // ---- Merge notices ----

  private notice(work: string, n: NoticeView): HTMLElement {
    const card = el("div", "history-notice");
    card.setAttribute("role", "note");
    card.append(el("p", "", noticeLine(n)));
    const actions = el("div", "history-notice-actions");
    actions.append(el("span", "history-time", `${dayOf(n.timeMs)} ${clock(n.timeMs)}`));
    if (n.places.length > 0) {
      const next = this.stepping.get(n.key);
      actions.append(button(showMeLabel(n.places.length, next), () => this.showPlace(work, n, next ?? 0), "Select this text in the editor"));
    }
    if (n.kind === "preamble") actions.append(button("Open Preamble…", () => this.hooks.openPreamble(work)));
    actions.append(button("Dismiss", () => this.run(() => this.hooks.dismissNotice(work, n.key)), "Hide this notice, on every Device"));
    card.append(actions);
    return card;
  }

  /** Selects one of a notice's places; the button then offers the next. */
  private showPlace(work: string, n: NoticeView, i: number) {
    const place = n.places[Math.min(i, n.places.length - 1)];
    void this.hooks.find(work, place.first, place.last).then((found) => {
      if (!found) this.hooks.error("That text isn't in the Work any more.");
    });
    if (n.places.length > 1) {
      this.stepping.set(n.key, (i + 1) % n.places.length);
      this.draw(work);
    }
  }

  // ---- Milestones ----

  private milestone(work: string, row: MilestoneRow): HTMLElement {
    const wrap = el("div", "history-milestone-wrap");
    const line = el("div", "history-milestone");
    line.classList.toggle("deleted", row.deleted);
    if (this.renaming?.id === row.id) {
      line.append(this.renaming.input);
    } else {
      line.append(el("strong", "history-milestone-name", row.name));
    }
    line.append(el("span", "history-milestone-when", `${clock(row.timeMs)} · ${row.device}`));
    const actions = el("span", "history-milestone-actions");
    if (row.deleted) {
      actions.append(button("Restore", () => this.run(() => this.hooks.setMilestoneDeleted(work, row.id, false)), "Bring this Milestone back"));
    } else if (this.renaming?.id !== row.id) {
      actions.append(
        button("Compare with now", () => this.compare(work, row.id, null), "What changed since then"),
        button("Return to…", () => void this.showReturn(work, row), "Make the text as it was then (it can be undone)"),
        button("Rename", () => this.startRename(work, row)),
        button("Delete", () => this.run(() => this.hooks.setMilestoneDeleted(work, row.id, true)), "Delete it (the History keeps it, so it can be restored)"),
      );
    }
    line.append(actions);
    wrap.append(line, ...this.previewFor(`milestone:${row.id}`));
    return wrap;
  }

  private startRename(work: string, row: MilestoneRow) {
    const input = el("input", "history-milestone-input");
    input.value = row.name;
    input.setAttribute("aria-label", "The Milestone's new name");
    const done = (save: boolean) => {
      if (this.renaming?.input !== input) return;
      this.renaming = null;
      const name = input.value.trim();
      if (save && name && name !== row.name) this.run(() => this.hooks.renameMilestone(work, row.id, name));
      else this.draw(work);
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        done(true);
      } else if (e.key === "Escape") {
        e.preventDefault();
        done(false);
      }
    });
    // Leaving the box saves. A redraw moves the box (it may blur), then focuses it again at
    // once: only a blur that lasts counts.
    input.addEventListener("blur", () => setTimeout(() => {
      if (document.activeElement !== input) done(true);
    }, 0));
    this.renaming = { id: row.id, input };
    this.draw(work);
    input.select();
  }

  /** Runs an action, then shows the result (the History grew, but nudging is cheap). */
  private run(action: () => Promise<void>) {
    action().then(() => this.refresh(), (e) => this.hooks.error(String(e)));
  }

  /** "Name a Milestone…": the header's button. */
  nameMilestone() {
    const work = this.hooks.work();
    if (work) this.run(() => this.hooks.nameMilestone(work));
  }
}
