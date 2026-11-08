// Tabs: the Works open in the app, and the Style library files. Each has its own editor (so a
// tab in the background still takes in changes from other Devices) and its own undo; one is
// shown at a time.

import { invoke } from "@tauri-apps/api/core";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, dropCursor, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import { history, historyKeymap, standardKeymap } from "@codemirror/commands";
import { LibraryFile, fromDisk } from "./libraryfile";
import { storyElements, type ElementInfo } from "./elements";
import { WorkSync, type WorkState } from "./sync";
import { formattedView } from "./formatted";
import { toggleMarkup } from "./shortcuts";
import { dragToMove } from "./dragging";
import { completions } from "./completion";

export type OpenedWork = { id: string; name: string; text: string; problems: { path: string; reason: string }[]; notice: string | null };

/** A Style library file to open in a tab. */
export type OpenedLibraryFile = { path: string; title: string; text: string };

export type Layout = { tabs: string[]; active: string | null; cursors: Record<string, number> };

/** What the rest of the page does for the tabs. */
export type TabEvents = {
  /** The tabs changed (opened, closed, renamed, another in front): worth remembering. */
  changed(): void;
  error(message: string): void;
  /** Asks the Author for a name; null if they cancelled. */
  askName(question: string, initial: string): Promise<string | null>;
  /** A Work's text changed in its editor (typed, or taken in from elsewhere). */
  edited(id: string): void;
  /** A typing burst in a Work was recorded as a Change. */
  committed(id: string): void;
  /** The cursor moved, or the text around it changed (UTF-16 position). */
  cursor(id: string, at: number): void;
  /** A Work's tab closed, or it now goes by another ID. */
  closed(id: string): void;
  /** "Add it?" was clicked on a Story element the Work doesn't have yet. */
  addElement(id: string, name: string): void;
};

const historyCompartment = new Compartment();
/** The Formatted view, or nothing (the Raw view). */
const viewCompartment = new Compartment();
/** Line numbers and the shaded cursor line (settings). */
const linesCompartment = new Compartment();
/** The Story elements the Work has (from its Preamble and the Style library). */
const elementsCompartment = new Compartment();

export type EditorOptions = { lineNumbers: boolean; highlightLine: boolean };

function lineExtensions(options: EditorOptions) {
  return [
    options.lineNumbers ? [lineNumbers(), options.highlightLine ? highlightActiveLineGutter() : []] : [],
    options.highlightLine ? highlightActiveLine() : [],
  ];
}

/** One open Work, or one Style library file. */
class Tab {
  readonly view: EditorView;
  /** Keeps a Work in step with its History; a library file has none. */
  private readonly worksSync: WorkSync | null = null;
  /** A library file, saved to disk as it is. */
  readonly file: LibraryFile | null = null;
  readonly pane: HTMLElement;
  readonly label: HTMLElement;
  /** The last word from Rust about it, and a line to show at its top (recovered, took in…). */
  state: WorkState | null = null;
  notice: string | null;

  /** A Work's sync (only Work tabs have one). */
  get sync(): WorkSync {
    if (!this.worksSync) throw new Error(`${this.id} isn't a Work.`);
    return this.worksSync;
  }

  get isWork() {
    return this.worksSync !== null;
  }

  get id(): string {
    return this.worksSync?.id ?? this.file!.id;
  }

  /** Resolves once everything typed has reached Rust (a Work) or the disk (a library file). */
  idle(): Promise<void> {
    return this.worksSync ? this.worksSync.idle() : this.file!.flush();
  }

  constructor(content: OpenedWork | OpenedLibraryFile, tabs: Tabs) {
    this.notice = "path" in content
      ? `Style library: ${content.path}. Saved as you type, and used by every Work that has it.`
      : content.notice;
    this.pane = document.createElement("div");
    this.pane.className = "pane";
    this.pane.hidden = true;
    const editor = document.createElement("div");
    editor.className = "editor";
    this.pane.append(editor);
    tabs.panes.append(this.pane);

    this.label = document.createElement("div");
    this.label.className = "tab";
    const name = document.createElement("button");
    name.type = "button";
    name.className = "tab-name";
    name.addEventListener("click", () => void tabs.activate(this.id));
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tab-close";
    close.title = "Close (Ctrl+W)";
    close.textContent = "×";
    close.addEventListener("click", () => void tabs.close(this.id));
    this.label.append(name, close);
    tabs.bar.append(this.label);

    if ("path" in content) {
      // A library file: plain Typst in the Raw view, with the editor's own undo.
      this.label.classList.add("library");
      this.file = new LibraryFile(content.path, (message) => tabs.events.error(message));
      this.view = new EditorView({
        parent: editor,
        state: EditorState.create({
          doc: content.text,
          extensions: [
            EditorView.lineWrapping,
            history(),
            keymap.of([...historyKeymap, ...standardKeymap]),
            linesCompartment.of(lineExtensions(tabs.options)),
            EditorView.updateListener.of((update) => {
              if (update.transactions.some((tr) => tr.docChanged && !tr.annotation(fromDisk))) this.file!.edited();
            }),
          ],
        }),
      });
      this.file.view = this.view;
      this.setLabel(content.path, content.title);
      return;
    }
    const work = content;

    // Ctrl+Z and redo: the sync decides between the editor's own undo (inside a draft) and the History's.
    const undoKeys = keymap.of([
      { key: "Mod-z", run: () => this.sync.undo(), preventDefault: true },
      { key: "Mod-Shift-z", run: () => this.sync.redo(), preventDefault: true },
      { key: "Mod-y", run: () => this.sync.redo(), preventDefault: true },
      { key: "Mod-b", run: toggleMarkup("strong"), preventDefault: true },
      { key: "Mod-i", run: toggleMarkup("emph"), preventDefault: true },
    ]);
    this.view = new EditorView({
      parent: editor,
      state: EditorState.create({
        doc: work.text,
        extensions: [
          EditorView.lineWrapping,
          historyCompartment.of(history()),
          undoKeys,
          // Only the basics: no shortcuts that move lines, which would bypass Moves.
          keymap.of(standardKeymap),
          dropCursor(),
          dragToMove(),
          completions({ work: () => this.sync.id, settled: () => this.sync.idle() }),
          elementsCompartment.of(storyElements([])),
          viewCompartment.of(tabs.formatted ? formattedView() : []),
          linesCompartment.of(lineExtensions(tabs.options)),
          EditorView.updateListener.of((update) => {
            for (const tr of update.transactions) this.sync.track(tr);
            if (update.docChanged) tabs.events.edited(this.sync.id);
            if (update.docChanged || update.selectionSet) tabs.events.cursor(this.sync.id, update.state.selection.main.head);
          }),
        ],
      }),
    });
    this.worksSync = new WorkSync(work.id, this.view, {
      state: (state) => {
        this.state = state;
        if (state.tookIn) this.notice = state.tookIn;
        tabs.heard(this, state);
      },
      burstClosed: () => {
        this.view.dispatch({ effects: historyCompartment.reconfigure([]) });
        this.view.dispatch({ effects: historyCompartment.reconfigure(history()) });
        tabs.events.committed(this.sync.id);
      },
      error: (message) => tabs.events.error(message),
    });
    this.rename(work.id, work.name);
    this.pane.addEventListener("wrtier-add-element", (e) => tabs.events.addElement(this.id, (e as CustomEvent<string>).detail));

    // A click anywhere in the writing area (the margins beside the text, the space below it)
    // puts the cursor in the text, at the nearest place.
    this.pane.addEventListener("mousedown", (e) => {
      if (e.button !== 0 || (e.target as Element).closest(".cm-content, .cm-gutters")) return;
      e.preventDefault();
      const box = this.view.contentDOM.getBoundingClientRect();
      const x = Math.min(Math.max(e.clientX, box.left + 1), box.right - 1);
      const pos = this.view.posAtCoords({ x, y: e.clientY }, false);
      this.view.focus();
      this.view.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
    });
  }

  rename(id: string, name: string) {
    this.sync.id = id;
    this.setLabel(id, name);
  }

  private setLabel(title: string, name: string) {
    const button = this.label.querySelector(".tab-name") as HTMLElement;
    button.textContent = name;
    button.title = title;
  }

  get name() {
    return (this.label.querySelector(".tab-name") as HTMLElement).textContent ?? "";
  }

  destroy() {
    this.view.destroy();
    this.pane.remove();
    this.label.remove();
  }
}

export class Tabs {
  private open: Tab[] = [];
  private front: Tab | null = null;
  /** The Formatted view (markup hidden except at the cursor), or the Raw view. */
  formatted = true;

  /** Line numbers and the shaded cursor line, for every tab. */
  options: EditorOptions = { lineNumbers: false, highlightLine: false };

  setOptions(options: EditorOptions) {
    this.options = options;
    for (const tab of this.open) {
      tab.view.dispatch({ effects: linesCompartment.reconfigure(lineExtensions(options)) });
    }
  }

  /** Switches every tab between the Formatted and the Raw view. */
  setFormatted(on: boolean) {
    this.formatted = on;
    for (const tab of this.open) {
      if (tab.isWork) tab.view.dispatch({ effects: viewCompartment.reconfigure(on ? formattedView() : []) });
    }
  }

  constructor(
    readonly bar: HTMLElement,
    readonly panes: HTMLElement,
    /** Where the active tab's lines go: what was recovered or taken in, and problems. */
    private lines: { notice: HTMLElement; status: HTMLElement; moved: HTMLElement; style: HTMLElement },
    readonly events: TabEvents,
  ) {}

  get active(): string | null {
    return this.front?.id ?? null;
  }

  has(id: string) {
    return this.open.some((tab) => tab.id === id);
  }

  private find(id: string) {
    return this.open.find((tab) => tab.id === id);
  }

  /** Shows an opened Work in a new tab (or the tab it already has). `cursor`: where to put it. */
  async add(work: OpenedWork, cursor?: number) {
    let tab = this.find(work.id);
    if (!tab) {
      tab = new Tab(work, this);
      this.open.push(tab);
      if (cursor !== undefined) {
        const anchor = Math.min(cursor, tab.view.state.doc.length);
        // (Rust hears of it as a cursor move, which changes nothing: no burst is open yet.)
        tab.view.dispatch({ selection: { anchor }, scrollIntoView: true });
      }
    }
    await this.activate(work.id);
  }

  /** Brings a tab to the front. The one that was there is left: its Text file catches up. */
  async activate(id: string) {
    const tab = this.find(id);
    if (!tab || tab === this.front) return;
    const previous = this.front;
    this.front = tab;
    for (const t of this.open) {
      t.pane.hidden = t !== tab;
      t.label.classList.toggle("active", t === tab);
    }
    this.show(tab);
    tab.view.focus();
    this.events.changed();
    if (previous) await this.leave(previous);
  }

  /** The Author left a Work for now (another tab, another window): its Text file catches up. */
  async leave(tab: Tab | null = this.front) {
    if (!tab) return;
    await tab.idle();
    if (!tab.isWork) return; // a library file: saved, nothing more
    try {
      const state = await invoke<WorkState>("leave_work", { id: tab.sync.id });
      tab.state = state;
      if (tab === this.front) this.show(tab);
    } catch (e) {
      this.events.error(String(e));
    }
  }

  /** Closes a tab: everything typed reaches Rust, is saved, and the Text file is written. */
  async close(id: string, how: "close" | "discard" = "close") {
    const tab = this.find(id);
    if (!tab) return;
    await tab.idle();
    if (tab.isWork) {
      try {
        await invoke(how === "close" ? "close_work" : "discard_work", { id: tab.sync.id });
      } catch (e) {
        this.events.error(String(e));
      }
    }
    const index = this.open.indexOf(tab);
    this.open.splice(index, 1);
    tab.destroy();
    this.events.closed(tab.id);
    if (tab === this.front) {
      this.front = null;
      const next = this.open[Math.min(index, this.open.length - 1)];
      if (next) await this.activate(next.id);
      else this.show(null);
    }
    this.events.changed();
  }

  async closeAll() {
    for (const tab of [...this.open]) await this.close(tab.id);
  }

  /** A Work was renamed (here, or on another Device and followed). */
  renamed(id: string, to: string) {
    const tab = this.find(id);
    if (!tab) return;
    tab.rename(to, to.split("/").pop() ?? to);
    this.events.closed(id);
    this.events.changed();
  }

  /** Where the cursor is in a Work (UTF-16). */
  cursorOf(id: string): number | null {
    return this.find(id)?.view.state.selection.main.head ?? null;
  }

  /** Resolves once everything typed in this Work has reached Rust. */
  async settled(id: string) {
    await this.find(id)?.idle();
  }

  /** Brings a Work to the front and selects a stretch of its text (UTF-16 positions). */
  async reveal(id: string, from: number, to: number) {
    const tab = this.find(id);
    if (!tab) return;
    await this.activate(id);
    const length = tab.view.state.doc.length;
    tab.view.dispatch({ selection: { anchor: Math.min(from, length), head: Math.min(to, length) }, scrollIntoView: true });
    tab.view.focus();
  }

  /** What the timer said about a Work (Rust names it by the ID the tab had). */
  status(id: string, state: WorkState) {
    const tab = this.find(id);
    if (tab?.isWork) tab.sync.status(state);
  }

  /** The open Works (not library files). */
  get works(): string[] {
    return this.open.filter((t) => t.isWork).map((t) => t.id);
  }

  /** Tells a Work's editor which Story elements it has, and how they look. */
  setElements(id: string, elements: ElementInfo[]) {
    const tab = this.find(id);
    if (tab?.isWork) tab.view.dispatch({ effects: elementsCompartment.reconfigure(storyElements(elements)) });
  }

  /** Is the tab in front a Work's (not a library file's)? */
  get activeIsWork(): boolean {
    return this.front?.isWork ?? false;
  }

  /** Opens a Style library file in a tab (or brings its tab to the front). */
  async addLibrary(file: OpenedLibraryFile) {
    const tab = this.open.find((t) => t.file?.path === file.path) ?? new Tab(file, this);
    if (!this.open.includes(tab)) this.open.push(tab);
    await this.activate(tab.id);
  }

  /** The Style library changed on disk: open library files take it in. */
  async libraryChanged() {
    for (const tab of this.open) {
      if (!tab.file) continue;
      if (!(await tab.file.reload())) {
        tab.notice = `${tab.file.path} is no longer in the Style library (deleted or renamed elsewhere). What's here is saved again if you type in it.`;
        if (tab === this.front) this.show(tab);
      }
    }
  }

  /** A tab heard from Rust: follow a rename, and refresh what's shown if it's in front. */
  heard(tab: Tab, state: WorkState) {
    if (state.moved?.kind === "renamed") {
      const from = tab.name;
      this.renamed(tab.sync.id, state.moved.to);
      tab.notice = `${from} was renamed to ${state.moved.to} on another Device.`;
    }
    if (tab === this.front) this.show(tab);
  }

  /** Where each tab's cursor is, for remembering. */
  layout(): Layout {
    const cursors: Record<string, number> = {};
    for (const tab of this.open) cursors[tab.id] = tab.view.state.selection.main.head;
    return { tabs: this.open.map((t) => t.id), active: this.active, cursors };
  }

  /** Waits until everything typed in every tab has reached Rust. */
  async idle() {
    for (const tab of this.open) await tab.idle();
  }

  private show(tab: Tab | null) {
    const { notice, status } = this.lines;
    notice.textContent = tab?.notice ?? "";
    notice.hidden = !tab?.notice;
    const state = tab?.state;
    const problems = [state?.saveError, state?.textFileProblem, state?.recoveryProblem, state?.syncProblem]
      .filter((line): line is string => !!line);
    status.textContent = problems.join("\n");
    status.hidden = problems.length === 0;
    // A Style changed in the library: shown until the next edit records it.
    this.lines.style.textContent = state?.styleNotice ?? "";
    this.lines.style.hidden = !state?.styleNotice;
    this.showMoved(tab);
  }

  /** A banner for a tab whose Work went to the trash, or vanished, on another Device. */
  private showMoved(tab: Tab | null) {
    const el = this.lines.moved;
    el.replaceChildren();
    const kind = tab?.state?.moved?.kind;
    el.hidden = !tab || (kind !== "trashed" && kind !== "gone" && kind !== "diverged");
    if (!tab || el.hidden) return;
    const text = document.createElement("p");
    const button = (label: string, run: () => Promise<void>) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.addEventListener("click", () => run().catch((e) => this.events.error(String(e))));
      return b;
    };
    const id = tab.sync.id;
    if (kind === "trashed") {
      text.textContent = `${tab.name} was moved to the trash on another Device. Nothing is saved until you choose.`;
      el.append(text,
        button("Restore it", async () => {
          const restored = await invoke<string>("restore_open", { id });
          if (restored !== id) this.renamed(id, restored);
          tab.state = tab.state && { ...tab.state, moved: null };
          this.show(tab);
        }),
        button("Close (saved in the trash)", () => this.close(id)));
    } else {
      text.textContent = kind === "diverged"
        ? `${tab.name}'s History file was replaced by another copy of it that was edited separately. Mixing the two would break it, so nothing more is saved there. Keep what's here as a new Work.`
        : `${tab.name} is no longer in the Vault (deleted outside the app). Nothing is saved until you choose.`;
      el.append(text,
        button("Save as a new Work…", async () => {
          const name = await this.events.askName("What should the new Work be called?", `${tab.name} (saved)`);
          if (!name) return;
          const work = await invoke<OpenedWork>("save_as_new", { id, name });
          await this.close(id, "discard");
          await this.add(work);
        }),
        button("Close without saving", () => this.close(id, "discard")));
    }
  }
}
