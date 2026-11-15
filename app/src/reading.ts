// The Reading panel (v2 plan, "Reading a second Work side by side"): one Work's text, between
// the editor and the Side panel, to read while writing another Work (or the same one, elsewhere
// in it). Nothing can be typed in it, nothing is dragged out of it, and reading a Work never
// opens it or records anything: a Work open in a tab is mirrored from its tab, so typing shows at
// once; any other is read from its History by Rust (vault/reading.rs) and read again whenever the
// Works list refreshes. Which Work, where it's scrolled to and the panel's width are Device state.
//
// The Work in front stays the one being written: the Side panel, the Chapter list, the counts
// and undo are never about the Work being read. Find and replace finds here too (never replaces).

import { invoke } from "@tauri-apps/api/core";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, keymap, type ViewUpdate } from "@codemirror/view";
import { standardKeymap } from "@codemirror/commands";
import { storyElements, type ElementInfo } from "./elements";
import { commentClasses, commentHighlights, rangesOf, setMarks, type CommentOptions } from "./comments";
import { inclusions, inclusionsOf, readOnlyInclusions, setInclusions, workBeingDragged, type InclusionView } from "./inclusions";
import { findExtension } from "./find";
import { chaptersIn } from "./chapters";
import { viewExtension } from "./tabs";
import type { Mark } from "./sync";

/** What a Device remembers about the Reading panel (device.rs `ReadingPlace`). */
export type ReadingPlace = { work: string; identity: string | null; top: number };

/** What Rust read (vault/reading.rs `Reading`). */
type Reading =
  | { kind: "unchanged" }
  | { kind: "text"; id: string; name: string; identity: string | null; stamp: string; text: string; inclusions: InclusionView[]; marks: Mark[]; elements: string[] }
  | { kind: "unreadable"; id: string; name: string; identity: string | null; reason: string };

export type ReadingHooks = {
  /** The editor of the Work's tab, if it's open here (null otherwise). */
  tabView(id: string): EditorView | null;
  /** That tab's Comments, as Rust last gave them. */
  tabMarks(id: string): Mark[];
  /** The Story elements a Work has, from their names (null: ask its Preamble). */
  elements(id: string, names: string[] | null): Promise<ElementInfo[]>;
  /** "Write in it": the Work comes to the front, with the cursor at `at`. */
  writeIn(id: string, at: number): void;
  /** "Open in the Comments panel": the Work comes to the front, showing these Comments. */
  openComments(id: string, comments: string[]): void;
  /** What's read, or where, changed: worth remembering. */
  changed(): void;
  /** The panel was shown or closed. */
  shown(on: boolean): void;
  error(message: string): void;
};

/**
 * The one change that turns `old` into `next`: the stretch between what they start and end
 * with alike (so what's scrolled to, and the selection, stay where they were). Null if the same.
 */
export function changeBetween(old: string, next: string): { from: number; to: number; insert: string } | null {
  if (old === next) return null;
  const most = Math.min(old.length, next.length);
  let start = 0;
  while (start < most && old.charCodeAt(start) === next.charCodeAt(start)) start++;
  let end = 0;
  while (end < most - start && old.charCodeAt(old.length - 1 - end) === next.charCodeAt(next.length - 1 - end)) end++;
  return { from: start, to: old.length - end, insert: next.slice(start, next.length - end) };
}

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

const viewCompartment = new Compartment();
const elementsCompartment = new Compartment();
const commentsCompartment = new Compartment();

export class ReadingPanel {
  readonly element = $("reading-panel");
  /** Where the Find bar goes while it finds in here. */
  readonly findSlot = $("reading-find-slot");
  private readonly label = $("reading-name");
  private readonly chapters = $<HTMLSelectElement>("reading-chapters");
  private readonly writeButton = $<HTMLButtonElement>("reading-write");
  private readonly why = $("reading-why");
  private readonly text = $("reading-text");
  private readonly card = $("reading-card");
  readonly view: EditorView;

  /** The Work being read (its ID in the Vault when last seen), or null. */
  private id: string | null = null;
  private identity: string | null = null;
  /** What Rust last sent, to ask with ("" before anything was sent). */
  private stamp = "";
  /** Mirroring the Work's tab, or reading its History. */
  private mirrored = false;
  /** Its own Comments: what the highlights are of. */
  private marks: Mark[] = [];
  /** Where to scroll to once the text is in (a restored place, or "Go to" on an Inclusion). */
  private scrollTo: number | null = null;
  private reading = false;
  private formatted = true;
  private options: CommentOptions = { highlights: true, resolved: false };
  /** Where the pointer last went down in the text, to put a Comment's card there. */
  private pointer = { x: 0, y: 0 };
  private chaptersTimer: number | undefined;
  private scrolled: number | undefined;

  constructor(private readonly hooks: ReadingHooks) {
    this.view = new EditorView({
      parent: this.text,
      state: EditorState.create({ doc: "", extensions: this.extensions() }),
    });
    $("reading-close").addEventListener("click", () => this.close());
    this.writeButton.addEventListener("click", () => {
      if (this.id) this.hooks.writeIn(this.id, this.top());
    });
    this.chapters.addEventListener("change", () => {
      const at = Number(this.chapters.value);
      this.chapters.value = "";
      if (Number.isFinite(at)) this.view.dispatch({ effects: EditorView.scrollIntoView(Math.min(at, this.view.state.doc.length), { y: "start" }) });
    });
    // "Go to" on an Inclusion: the Work it includes is read here, at the included text.
    this.element.addEventListener("wrtier-open-work", (e) => {
      const { work, at } = (e as CustomEvent<{ work: string; at: number }>).detail;
      void this.open(work, null, at);
    });
    // A highlight in included text: its Comments' card (they belong to the included Work).
    this.element.addEventListener("wrtier-open-comments", (e) => this.showCard((e as CustomEvent<string[]>).detail));
    this.text.addEventListener("pointerdown", (e) => {
      this.pointer = { x: e.clientX, y: e.clientY };
      if (!(e.target as Element).closest("#reading-card")) this.hideCard();
    }, { capture: true });
    this.element.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && !this.card.hidden) {
        e.preventDefault();
        this.hideCard();
      }
    });
    // A Work dragged here from the Vault browser is read here.
    this.element.addEventListener("dragover", (e) => {
      if (!workBeingDragged(e)) return;
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    });
    this.element.addEventListener("drop", (e) => {
      const work = workBeingDragged(e);
      if (!work) return;
      e.preventDefault();
      e.stopPropagation();
      void this.open(work);
    });
    this.view.scrollDOM.addEventListener("scroll", () => {
      window.clearTimeout(this.scrolled);
      this.scrolled = window.setTimeout(() => this.hooks.changed(), 500);
    });
  }

  private extensions() {
    return [
      EditorView.lineWrapping,
      // Read-only, but focusable: the text can be selected, copied, and found in.
      EditorState.readOnly.of(true),
      keymap.of(standardKeymap),
      // Text is never dragged out of it (that would look like a Move); it's copied.
      EditorView.domEventHandlers({
        dragstart: (event) => {
          event.preventDefault();
          return true;
        },
      }),
      viewCompartment.of(viewExtension(this.formatted)),
      elementsCompartment.of(storyElements([])),
      readOnlyInclusions.of(true),
      inclusions(),
      commentHighlights(() => this.options, (ids) => this.showCard(ids)),
      commentsCompartment.of(commentClasses(this.options)),
      findExtension(),
    ];
  }

  /** Is a Work being read? */
  get isReading(): boolean {
    return this.reading;
  }

  /** The Work being read, if one is. */
  get work(): string | null {
    return this.id;
  }

  /** Has the focus (or the Find bar in it has): then Find and replace finds in here. */
  hasFocus(): boolean {
    return this.reading && this.element.contains(document.activeElement);
  }

  /** What a Device remembers: the Work, and where it's scrolled to. */
  place(): ReadingPlace | null {
    return this.reading && this.id ? { work: this.id, identity: this.identity, top: this.top() } : null;
  }

  /**
   * Reads a Work here (in place of the one being read), by its ID in the Vault or, if it has
   * one, its Work ID; scrolled to `top` (else to its start).
   */
  async open(work: string, identity: string | null = null, top = 0) {
    const another = work !== this.id;
    this.id = work;
    this.identity = another ? identity : identity ?? this.identity;
    this.stamp = "";
    this.mirrored = false;
    this.scrollTo = top;
    this.hideCard();
    this.show(true);
    this.label.textContent = work.split("/").pop() ?? work;
    this.label.title = work;
    if (another) {
      // Nothing of the last one stays while this one is read.
      this.marks = [];
      this.view.dispatch({ changes: { from: 0, to: this.view.state.doc.length, insert: "" }, effects: [setInclusions.of([]), setMarks.of([])] });
    }
    await this.refresh();
    this.hooks.changed();
  }

  /** Closes the panel: nothing is read any more. */
  close(remember = true) {
    if (!this.reading) return;
    this.id = null;
    this.identity = null;
    this.stamp = "";
    this.mirrored = false;
    this.marks = [];
    this.hideCard();
    this.view.dispatch({ changes: { from: 0, to: this.view.state.doc.length, insert: "" }, effects: [setInclusions.of([]), setMarks.of([])] });
    this.show(false);
    if (remember) this.hooks.changed();
  }

  private show(on: boolean) {
    if (this.reading === on) return;
    this.reading = on;
    this.element.hidden = !on;
    this.hooks.shown(on);
  }

  /**
   * Reads the Work again if it changed: from its tab if it has one here now, else from its
   * History. Called when it's opened, and whenever the Works list refreshes.
   */
  async refresh() {
    const id = this.id;
    if (!id) return;
    const tab = this.hooks.tabView(id);
    if (tab) {
      if (!this.mirrored) await this.mirror(id, tab);
      return;
    }
    if (this.mirrored) {
      // Its tab closed: it's read from its History from now on.
      this.mirrored = false;
      this.stamp = "";
    }
    let reading: Reading;
    try {
      reading = await invoke<Reading>("read_work", { work: id, identity: this.identity, known: this.stamp || null });
    } catch (e) {
      this.hooks.error(String(e));
      return;
    }
    if (this.id !== id || this.mirrored) return; // another Work, or its tab opened, meanwhile
    if (reading.kind === "unchanged") return;
    const moved = reading.id !== this.id;
    this.id = reading.id;
    this.identity = reading.identity ?? this.identity;
    this.label.textContent = reading.name;
    this.label.title = reading.id;
    if (moved) this.hooks.changed();
    if (reading.kind === "unreadable") {
      this.stamp = "";
      this.showWhy(reading.reason);
      return;
    }
    this.showWhy(null);
    this.stamp = reading.stamp;
    this.marks = reading.marks;
    this.setText(reading.text, reading.inclusions, reading.marks);
    await this.setElements(this.id, reading.elements);
  }

  /** Its Work's tab is open here: what it shows, as it is now (typing included), from now on. */
  private async mirror(id: string, tab: EditorView) {
    this.mirrored = true;
    this.stamp = "";
    this.showWhy(null);
    this.marks = this.hooks.tabMarks(id).map((m) => ({ ...m, ranges: rangesOf(tab, m.id) }));
    this.setText(tab.state.doc.toString(), inclusionsOf(tab.state), this.marks);
    await this.setElements(id, null);
  }

  /** A tab's editor changed: if it's the Work being read, the panel follows at once. */
  tabUpdated(id: string, update: ViewUpdate) {
    if (!this.mirrored || id !== this.id) return;
    const doc = this.view.state.doc;
    if (update.docChanged) {
      if (doc.length !== update.startState.doc.length) {
        // Out of step (shouldn't happen): take the whole text again.
        this.mirrored = false;
        void this.refresh();
        return;
      }
      this.view.dispatch({ changes: update.changes });
    }
    // New Inclusions or Comments from Rust: taken as the tab has them.
    if (update.transactions.some((tr) => tr.effects.length > 0)) {
      this.marks = this.hooks.tabMarks(id).map((m) => ({ ...m, ranges: rangesOf(update.view, m.id) }));
      this.view.dispatch({ effects: [setInclusions.of(inclusionsOf(update.state)), setMarks.of(this.marks)] });
    }
    if (update.docChanged) this.chaptersSoon();
  }

  /** A tab opened: if it's the Work being read, it's mirrored from now on. */
  tabOpened(id: string) {
    if (id === this.id && !this.mirrored) void this.refresh();
  }

  /** A tab closed (or now goes by another ID): read from its History again if it was mirrored. */
  tabClosed(id: string) {
    if (id === this.id && this.mirrored && !this.hooks.tabView(id)) void this.refresh();
  }

  /** A Work was renamed or moved here: the panel follows it. */
  renamed(from: string, to: string) {
    if (this.id !== from) return;
    this.id = to;
    this.label.textContent = to.split("/").pop() ?? to;
    this.label.title = to;
    this.hooks.changed();
  }

  /** The Formatted or the Raw view, as in the editor. */
  setFormatted(on: boolean) {
    this.formatted = on;
    this.view.dispatch({ effects: viewCompartment.reconfigure(viewExtension(on)) });
  }

  /** Comment highlights shown or not, as in the editor; resolved ones never are here. */
  setCommentHighlights(on: boolean) {
    this.options = { highlights: on, resolved: false };
    this.view.dispatch({ effects: commentsCompartment.reconfigure(commentClasses(this.options)) });
  }

  /** The Story elements changed (the library, or a Preamble): read them again. */
  async elementsChanged() {
    if (this.id) await this.setElements(this.id, null);
  }

  /** The first position shown (UTF-16). */
  private top(): number {
    const { view } = this;
    if (view.state.doc.length === 0) return 0;
    return view.lineBlockAtHeight(view.scrollDOM.scrollTop).from;
  }

  /** Puts the text in (changing only what changed), with its Inclusions and Comments. */
  private setText(text: string, views: InclusionView[], marks: Mark[]) {
    const change = changeBetween(this.view.state.doc.toString(), text);
    this.view.dispatch({ ...(change ? { changes: change } : {}), effects: [setInclusions.of(views), setMarks.of(marks)] });
    if (this.scrollTo !== null) {
      const at = Math.min(this.scrollTo, this.view.state.doc.length);
      this.scrollTo = null;
      // Once drawn: the panel may only just have been shown.
      requestAnimationFrame(() => this.view.dispatch({ effects: EditorView.scrollIntoView(at, { y: "start" }) }));
    }
    this.fillChapters();
  }

  private async setElements(id: string, names: string[] | null) {
    try {
      const elements = await this.hooks.elements(id, names);
      if (this.id === id) this.view.dispatch({ effects: elementsCompartment.reconfigure(storyElements(elements)) });
    } catch {
      // gone or unreadable right now: it keeps what it had
    }
  }

  /** Why the Work can't be read, in place of its text (null: it can). */
  private showWhy(reason: string | null) {
    this.why.textContent = reason ?? "";
    this.why.hidden = reason === null;
    this.text.hidden = reason !== null;
    this.chapters.hidden = reason !== null;
    this.writeButton.hidden = reason !== null;
    if (reason !== null) this.hideCard();
  }

  private chaptersSoon() {
    window.clearTimeout(this.chaptersTimer);
    this.chaptersTimer = window.setTimeout(() => this.fillChapters(), 300);
  }

  /** The Chapter menu: the Work's Chapters and Parts, and its Inclusions, in order. */
  private fillChapters() {
    const { state } = this.view;
    const included = inclusionsOf(state).filter((v) => !v.missing).map((v) => ({
      at: v.at,
      title: v.whole ? v.label : v.label.slice(v.label.indexOf(": ") + 2),
      words: 0,
    }));
    const items = chaptersIn(state.doc.toString(), included);
    const first = document.createElement("option");
    first.value = "";
    first.textContent = items.length > 0 ? "Go to Chapter…" : "No Chapters";
    this.chapters.replaceChildren(first, ...items.map((item) => {
      const option = document.createElement("option");
      option.value = String(item.from);
      option.textContent = item.kind === "part" ? item.title.toUpperCase() : `${item.included ? "↳ " : ""}${item.title}`;
      return option;
    }));
    this.chapters.disabled = items.length === 0;
  }

  /** A highlight was clicked: its Comments, read-only, by the click. */
  private showCard(ids: string[]) {
    const all = [...this.marks, ...inclusionsOf(this.view.state).flatMap((v) => v.comments)];
    const shown = ids.map((id) => all.find((m) => m.id === id)).filter((m): m is Mark => !!m);
    if (shown.length === 0 || !this.id) return;
    const work = this.id;
    // A Comment on included text belongs to that Work, but shows in this one's panel too.
    this.card.replaceChildren(...shown.map((mark) => {
      const item = document.createElement("div");
      item.className = "reading-card-comment";
      const who = document.createElement("strong");
      who.className = `comment-sender comment-sender-${mark.colour}`;
      who.textContent = mark.name;
      const body = document.createElement("p");
      body.textContent = mark.body;
      item.append(who, body);
      for (const reply of mark.replies) {
        const line = document.createElement("p");
        line.className = "reading-card-reply";
        const name = document.createElement("strong");
        name.className = `comment-sender comment-sender-${reply.colour}`;
        name.textContent = reply.name;
        line.append(name, ` ${reply.body}`);
        item.append(line);
      }
      return item;
    }));
    const go = document.createElement("button");
    go.type = "button";
    go.className = "link";
    go.textContent = "Open in the Comments panel";
    go.title = `Bring ${work.split("/").pop()} to the front, with its Comments`;
    go.addEventListener("click", () => {
      this.hideCard();
      this.hooks.openComments(work, shown.map((m) => m.id));
    });
    this.card.append(go);
    this.card.hidden = false;
    // By the click, inside the panel.
    const box = this.element.getBoundingClientRect();
    const width = Math.min(320, box.width - 24);
    this.card.style.width = `${width}px`;
    this.card.style.left = `${Math.max(12, Math.min(this.pointer.x - box.left - width / 2, box.width - width - 12))}px`;
    this.card.style.top = `${Math.max(48, this.pointer.y - box.top + 18)}px`;
  }

  private hideCard() {
    this.card.hidden = true;
  }
}
