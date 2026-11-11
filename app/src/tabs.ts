// Tabs: the Works open in the app, and the Style library files. Each has its own editor (so a
// tab in the background still takes in changes from other Devices) and its own undo; one is
// shown at a time.

import { invoke } from "@tauri-apps/api/core";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, dropCursor, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers } from "@codemirror/view";
import { history, historyKeymap, standardKeymap } from "@codemirror/commands";
import { LibraryFile, fromDisk, libraryId } from "./libraryfile";
import { storyElements, type ElementInfo } from "./elements";
import { WorkSync, type WorkState } from "./sync";
import { formattedView } from "./formatted";
import { typstColouring } from "./colouring";
import { toggleMarkup } from "./shortcuts";
import { dragToMove } from "./dragging";
import { completions } from "./completion";
import { commentClasses, commentHighlights, pendingRange, rangesOf, setMarks, setPending, type CommentOptions } from "./comments";
import type { CommentOp, HistoryOp, Mark } from "./sync";
import { bindings, COMMANDS, withKey, type CommandId } from "./keys";
import { type IncludedEntry, type MoveOp } from "./chapters";
import { sessionTints, setTints } from "./sessiontints";
import { modExtensions, type Mod } from "./mods";
import { INCLUSION, inclusions, inclusionsOf, placeInclusion, readerText, setInclusions, type InclusionView } from "./inclusions";
import { wordsIn } from "./counts";

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
  /** Highlighted text was clicked: these Comments (by ID) are on it. */
  openComments(id: string, comments: string[]): void;
  /** A Work's Comments, as they are now (after anything changed them). */
  marks(id: string, marks: Mark[]): void;
  /** Its key (Ctrl+Shift+M at first), or "Add Comment" in the menu: a new Comment on the selection or the word at the cursor. */
  addComment(id: string): void;
  /** Right-click inside a selection: the app's own menu, at (x, y). */
  selectionMenu(id: string, x: number, y: number): void;
  /** A Work's History grew (typing recorded, something taken in, a Revert...). */
  historyGrew(id: string): void;
  /** Its key (Ctrl+Shift+H at first): the History of the selection, or the paragraph. */
  textHistory(id: string): void;
  /** How many Merge notices the Work in front has (0 if it isn't a Work): "History (n)". */
  notices(count: number): void;
  /** "Go to" on an Inclusion: open the Work it includes (its ID). */
  openWork(id: string, at: number): void;
  /** A Mod's command's key, while writing in a Work: it runs it, or offers to add the Mod. */
  modCommand(id: string, command: CommandId): boolean;
};

const historyCompartment = new Compartment();
/** The Formatted view, or the Raw view's Typst colouring. */
const viewCompartment = new Compartment();
/** Line numbers and the shaded cursor line (settings). */
const linesCompartment = new Compartment();
/** The Story elements the Work has (from its Preamble and the Style library). */
const elementsCompartment = new Compartment();
/** Which Comment highlights are painted (a setting, and "Show resolved"). */
const commentsCompartment = new Compartment();
/** The editor's commands on the keys the Author gave them. */
const keysCompartment = new Compartment();
/** What the Work's Mods add to its editor (those in use on this Device). */
const modsCompartment = new Compartment();

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
  /** Its Comments, as Rust last gave them. */
  marks: Mark[] = [];
  /** The Mods in use in it on this Device (their IDs), as last set. */
  mods: string[] = [];
  /** Does it (maybe) have Inclusions? Then they're asked for again now and then, to stay live. */
  hasInclusions = false;
  private fetchingInclusions = false;

  /**
   * Asks Rust for the Inclusions as they are now (the included Works may have changed in other
   * tabs, or on other Devices), once everything typed has reached it, so positions fit.
   */
  async refreshInclusions(events: TabEvents) {
    if (!this.isWork || !this.hasInclusions || this.fetchingInclusions) return;
    this.fetchingInclusions = true;
    try {
      await this.sync.idle();
      const views = await invoke<InclusionView[]>("get_inclusions", { id: this.sync.id });
      this.hasInclusions = views.length > 0 || this.view.state.doc.toString().includes(INCLUSION);
      // Typed meanwhile: they're asked for again next time rather than put in the wrong place.
      if (this.sync.isIdle()) {
        this.view.dispatch({ effects: setInclusions.of(views) });
        events.edited(this.sync.id); // the counts include them
      }
    } catch (e) {
      events.error(String(e));
    } finally {
      this.fetchingInclusions = false;
    }
  }

  /**
   * Puts an Inclusion of `work` (its ID in the Vault), or of its Chapter starting at `chapter`,
   * on a line of its own at the cursor.
   */
  include(work: string, chapter: number | null, events: TabEvents) {
    const { state } = this.view;
    const place = placeInclusion(state.doc, state.selection.main.head);
    this.view.dispatch({
      changes: { from: place.from, insert: place.insert },
      selection: { anchor: place.at + 1 },
      userEvent: "input.paste",
      scrollIntoView: true,
    });
    this.sync.include(place.at, work, chapter);
    this.hasInclusions = true;
    void this.refreshInclusions(events);
  }

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
    close.title = withKey("Close", "closeTab");
    close.textContent = "×";
    close.addEventListener("click", () => void tabs.close(this.id));
    this.label.append(name, close);
    tabs.bar.append(this.label);

    if ("path" in content) {
      // A library file: Typst in the Raw view, coloured whole, with the editor's own undo.
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
            typstColouring(true),
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

    this.view = new EditorView({
      parent: editor,
      state: EditorState.create({
        doc: work.text,
        extensions: [
          EditorView.lineWrapping,
          historyCompartment.of(history()),
          keysCompartment.of(this.editorKeys(tabs)),
          // Only the basics: no shortcuts that move lines, which would bypass Moves.
          keymap.of(standardKeymap),
          dropCursor(),
          dragToMove(),
          completions({ work: () => this.sync.id, settled: () => this.sync.idle() }),
          elementsCompartment.of(storyElements([])),
          viewCompartment.of(tabs.formatted ? formattedView() : typstColouring(false)),
          linesCompartment.of(lineExtensions(tabs.options)),
          commentHighlights(() => tabs.commentOptions, (ids) => tabs.events.openComments(this.id, ids)),
          commentsCompartment.of(commentClasses(tabs.commentOptions)),
          sessionTints(),
          inclusions(),
          modsCompartment.of([]),
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
      marks: (marks) => {
        this.marks = marks;
        this.view.dispatch({ effects: setMarks.of(marks) });
        tabs.events.marks(this.id, marks);
      },
      tints: (tints) => this.view.dispatch({ effects: setTints.of(tints) }),
      grew: () => {
        tabs.events.historyGrew(this.id);
        void this.refreshInclusions(tabs.events);
      },
      error: (message) => tabs.events.error(message),
    });
    this.hasInclusions = work.text.includes(INCLUSION);
    void this.refreshInclusions(tabs.events);
    this.pane.addEventListener("wrtier-open-work", (e) => {
      const { work, at } = (e as CustomEvent<{ work: string; at: number }>).detail;
      tabs.events.openWork(work, at);
    });
    this.rename(work.id, work.name);
    this.sync.requestMarks();
    this.sync.setTinting(tabs.tinting);

    // Right-click inside a selection: the app's menu (Add Comment, Cut, Copy, Paste). Anywhere
    // else the system's own, which has the spelling suggestions.
    this.view.contentDOM.addEventListener("contextmenu", (e) => {
      const selection = this.view.state.selection.main;
      const pos = this.view.posAtCoords({ x: e.clientX, y: e.clientY });
      if (selection.empty || pos === null || pos < selection.from || pos > selection.to) return;
      e.preventDefault();
      tabs.events.selectionMenu(this.id, e.clientX, e.clientY);
    });
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

  /**
   * What the editor's commands do in this tab: on their keys (keys.ts), and the phone's toolbar.
   * Undo and redo: the sync decides between the editor's own undo (inside a draft) and the History's.
   */
  commands(tabs: Tabs): Partial<Record<CommandId, () => boolean>> {
    const run: Partial<Record<CommandId, () => boolean>> = {
      undo: () => this.sync.undo(),
      redo: () => this.sync.redo(),
      redoAlt: () => this.sync.redo(),
      strong: () => toggleMarkup("strong")(this.view),
      emph: () => toggleMarkup("emph")(this.view),
      addComment: () => {
        tabs.events.addComment(this.id);
        return true;
      },
      textHistory: () => {
        tabs.events.textHistory(this.id);
        return true;
      },
    };
    // A Mod's commands are on their keys in every Work: one without the Mod offers to add it.
    for (const command of COMMANDS) {
      if (command.mod) run[command.id] = () => tabs.events.modCommand(this.id, command.id);
    }
    return run;
  }

  /** The editor's commands on the keys the Author gave them (keys.ts). */
  editorKeys(tabs: Tabs) {
    const run = this.commands(tabs);
    const keys = bindings();
    return keymap.of(Object.entries(run).flatMap(([command, action]) => {
      const key = keys[command as CommandId];
      return key ? [{ key, run: action, preventDefault: true }] : [];
    }));
  }

  rename(id: string, name: string) {
    this.sync.id = id;
    this.setLabel(id, name);
  }

  /** A library file's new title (the name shown; its path stays). */
  retitle(title: string) {
    if (this.file) this.setLabel(this.file.path, title);
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

  /** Which Comment highlights are painted, in every Work. */
  commentOptions: CommentOptions = { highlights: true, resolved: false };

  /** "Colour text by Session" (Device state, off by default): every open Work follows it. */
  tinting = false;

  setTinting(on: boolean) {
    this.tinting = on;
    for (const tab of this.open) if (tab.isWork) tab.sync.setTinting(on);
  }

  setCommentOptions(options: CommentOptions) {
    this.commentOptions = options;
    for (const tab of this.open) {
      if (tab.isWork) tab.view.dispatch({ effects: commentsCompartment.reconfigure(commentClasses(options)) });
    }
  }

  // ---- Comments ----

  /** A Work's Comments, as Rust last gave them. */
  marksOf(id: string): Mark[] {
    return this.find(id)?.marks ?? [];
  }

  /**
   * "History of this text": the selection in a Work (or the paragraph the cursor is in) is
   * what its History tab narrows to. A short quote of it, or null if there's no text there.
   */
  async stretch(id: string): Promise<string | null> {
    const tab = this.find(id);
    if (!tab?.isWork) return null;
    const { from, to } = tab.view.state.selection.main;
    return tab.sync.stretch(from, to);
  }

  /** Reverts, restores or returns to a Milestone in a Work, in order with its typing. */
  history(id: string, op: HistoryOp) {
    const tab = this.find(id);
    if (tab?.isWork) tab.sync.history(op);
  }

  /** Does something to a Work's Comments, in order with its typing. */
  comment(id: string, op: CommentOp) {
    const tab = this.find(id);
    if (tab?.isWork) tab.sync.comment(op);
  }

  /** What a new Comment would be on: the selection, or else the word at the cursor. */
  commentTarget(id: string): { from: number; to: number } | null {
    const tab = this.find(id);
    if (!tab?.isWork) return null;
    const { state } = tab.view;
    const selection = state.selection.main;
    if (!selection.empty) return { from: selection.from, to: selection.to };
    const word = state.wordAt(selection.head);
    return word ? { from: word.from, to: word.to } : null;
  }

  /** Outlines the text a new Comment will be on (null: no longer). */
  setPending(id: string, range: { from: number; to: number } | null) {
    const tab = this.find(id);
    if (tab?.isWork) tab.view.dispatch({ effects: setPending.of(range) });
  }

  /** Where the new Comment's text is now, if it's still there. */
  pendingRange(id: string): { from: number; to: number } | null {
    const tab = this.find(id);
    return tab?.isWork ? pendingRange(tab.view) : null;
  }

  /** Selects part `part` of a Comment's text (a Move may have split it). Returns how many parts it has. */
  showComment(id: string, comment: string, part: number): number {
    const tab = this.find(id);
    if (!tab?.isWork) return 0;
    const ranges = rangesOf(tab.view, comment);
    if (ranges.length === 0) return 0;
    const [from, to] = ranges[part % ranges.length];
    tab.view.dispatch({ selection: { anchor: from, head: to }, scrollIntoView: true });
    tab.view.focus(); // the selection only shows in a focused editor
    return ranges.length;
  }

  /**
   * Selects some characters of a Work by their identity (a History snippet was clicked):
   * where they are now, asked through its queue. False if they aren't shown any more.
   */
  async findText(id: string, first: string, last: string | null): Promise<boolean> {
    const tab = this.find(id);
    if (!tab?.isWork) return false;
    const range = await tab.sync.find(first, last);
    if (!range) return false;
    const length = tab.view.state.doc.length;
    tab.view.dispatch({ selection: { anchor: Math.min(range[0], length), head: Math.min(range[1], length) }, scrollIntoView: true });
    tab.view.focus(); // the selection only shows in a focused editor
    return true;
  }

  /** The editor of a Work (for the selection menu's Cut, Copy and Paste). */
  viewOf(id: string): EditorView | null {
    const tab = this.find(id);
    return tab?.isWork ? tab.view : null;
  }

  /** The keys changed (keys.ts): every editor and tab label follows. */
  keysChanged() {
    for (const tab of this.open) {
      (tab.label.querySelector(".tab-close") as HTMLElement).title = withKey("Close", "closeTab");
      if (tab.isWork) tab.view.dispatch({ effects: keysCompartment.reconfigure(tab.editorKeys(this)) });
    }
  }

  /** The text selected in a Work ("" if nothing is; null if it isn't open). */
  selectedText(id: string): string | null {
    const tab = this.find(id);
    if (!tab?.isWork) return null;
    const { from, to } = tab.view.state.selection.main;
    return tab.view.state.sliceDoc(from, to);
  }

  /**
   * A Work's text (or its selection) as a reader sees it, with its Inclusions' text in their
   * place, and the included part on its own: what the counts count. Null if it isn't open.
   */
  readerTextOf(id: string, selection = false): { text: string; included: string } | null {
    const tab = this.find(id);
    if (!tab?.isWork) return null;
    const { state } = tab.view;
    const { from, to } = selection ? state.selection.main : { from: 0, to: state.doc.length };
    return readerText(state, from, to);
  }

  /**
   * A Work's Inclusions as the Chapter list shows them: an included Chapter under its own title
   * (its words not counting its heading), a whole Work under its name.
   */
  includedOf(id: string): IncludedEntry[] {
    const tab = this.find(id);
    if (!tab?.isWork) return [];
    return inclusionsOf(tab.view.state).filter((v) => !v.missing).map((v) => {
      const body = v.whole ? v.text : v.text.slice(v.text.indexOf("\n") + 1 || v.text.length);
      const title = v.whole ? v.label : v.label.slice(v.label.indexOf(": ") + 2);
      return { at: v.at, title, words: wordsIn(body) };
    });
  }

  /** Puts an Inclusion of `work` (or its Chapter at `chapter`) at the cursor in the Work `id`. */
  include(id: string, work: string, chapter: number | null) {
    const tab = this.find(id);
    if (tab?.isWork) tab.include(work, chapter, this.events);
  }

  /** The Work in front asks for its Inclusions again, so they follow the Works they include. */
  refreshInclusions() {
    if (this.front?.isWork) void this.front.refreshInclusions(this.events);
  }

  /** A Work's text as its editor has it now (null if it isn't open). */
  textOf(id: string): string | null {
    const tab = this.find(id);
    return tab?.isWork ? tab.view.state.doc.toString() : null;
  }

  /**
   * Moves text in a Work, as Moves (each in the text the ones before it left), so it keeps its
   * History and Comments; then puts the cursor at `cursor` (in the text after them) and shows it.
   */
  moveText(id: string, moves: MoveOp[], cursor: number) {
    const tab = this.find(id);
    if (!tab?.isWork || moves.length === 0) return;
    const { view } = tab;
    for (const m of moves) {
      const changes = view.state.changes([{ from: m.from, to: m.to }, { from: m.at, insert: view.state.sliceDoc(m.from, m.to) }]);
      view.dispatch({ changes, userEvent: "move.drop" }); // one Move each, as when dragging text
    }
    view.dispatch({ selection: { anchor: Math.min(cursor, view.state.doc.length) }, scrollIntoView: true });
    view.focus();
  }

  /** The Mods in use in a Work on this Device changed: its editor follows. */
  setMods(id: string, mods: Mod[]) {
    const tab = this.find(id);
    if (!tab?.isWork) return;
    const ids = mods.map((m) => m.id);
    if (ids.join() === tab.mods.join()) return;
    tab.mods = ids;
    tab.view.dispatch({ effects: modsCompartment.reconfigure(modExtensions(mods)) });
  }

  /** Every open Work asks for its highlights again (a Reviewer's name or colour changed). */
  refreshMarks() {
    for (const tab of this.open) if (tab.isWork) tab.sync.requestMarks();
  }

  /** Switches every tab between the Formatted and the Raw view. */
  setFormatted(on: boolean) {
    this.formatted = on;
    for (const tab of this.open) {
      if (tab.isWork) tab.view.dispatch({ effects: viewCompartment.reconfigure(on ? formattedView() : typstColouring(false)) });
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

  /** Runs an editor command in the tab in front, as its key would (the phone's toolbar). */
  runCommand(command: CommandId): boolean {
    const run = this.front?.commands(this)[command];
    return run ? run() : false;
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
  /** A library file got a new title: its tab, if open, follows. */
  libraryRetitled(path: string, title: string) {
    this.find(libraryId(path))?.retitle(title);
  }

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
    this.events.notices(state?.notices ?? 0);
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
