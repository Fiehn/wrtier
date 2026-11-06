// Tabs: the Works open in the app. Each has its own editor (so a tab in the background still
// takes in changes from other Devices) and its own undo; one is shown at a time.

import { invoke } from "@tauri-apps/api/core";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, dropCursor, keymap } from "@codemirror/view";
import { history, standardKeymap } from "@codemirror/commands";
import { WorkSync, type WorkState } from "./sync";

export type OpenedWork = { id: string; name: string; text: string; problems: { path: string; reason: string }[]; notice: string | null };

export type Layout = { tabs: string[]; active: string | null; cursors: Record<string, number> };

/** What the rest of the page does for the tabs. */
export type TabEvents = {
  /** The tabs changed (opened, closed, renamed, another in front): worth remembering. */
  changed(): void;
  error(message: string): void;
  /** Asks the Author for a name; null if they cancelled. */
  askName(question: string, initial: string): Promise<string | null>;
};

const historyCompartment = new Compartment();

/** One open Work. */
class Tab {
  readonly view: EditorView;
  readonly sync: WorkSync;
  readonly pane: HTMLElement;
  readonly label: HTMLElement;
  /** The last word from Rust about it, and a line to show at its top (recovered, took in…). */
  state: WorkState | null = null;
  notice: string | null;

  constructor(work: OpenedWork, tabs: Tabs) {
    this.notice = work.notice;
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
    name.addEventListener("click", () => void tabs.activate(this.sync.id));
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tab-close";
    close.title = "Close (Ctrl+W)";
    close.textContent = "×";
    close.addEventListener("click", () => void tabs.close(this.sync.id));
    this.label.append(name, close);
    tabs.bar.append(this.label);

    // Ctrl+Z and redo: the sync decides between the editor's own undo (inside a draft) and the History's.
    const undoKeys = keymap.of([
      { key: "Mod-z", run: () => this.sync.undo(), preventDefault: true },
      { key: "Mod-Shift-z", run: () => this.sync.redo(), preventDefault: true },
      { key: "Mod-y", run: () => this.sync.redo(), preventDefault: true },
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
          EditorView.updateListener.of((update) => {
            for (const tr of update.transactions) this.sync.track(tr);
          }),
        ],
      }),
    });
    this.sync = new WorkSync(work.id, this.view, {
      state: (state) => {
        this.state = state;
        if (state.tookIn) this.notice = state.tookIn;
        tabs.heard(this, state);
      },
      burstClosed: () => {
        this.view.dispatch({ effects: historyCompartment.reconfigure([]) });
        this.view.dispatch({ effects: historyCompartment.reconfigure(history()) });
      },
      error: (message) => tabs.events.error(message),
    });
    this.rename(work.id, work.name);
  }

  rename(id: string, name: string) {
    this.sync.id = id;
    const button = this.label.querySelector(".tab-name") as HTMLElement;
    button.textContent = name;
    button.title = id;
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

  constructor(
    readonly bar: HTMLElement,
    readonly panes: HTMLElement,
    /** Where the active tab's lines go: what was recovered or taken in, and problems. */
    private lines: { notice: HTMLElement; status: HTMLElement; moved: HTMLElement },
    readonly events: TabEvents,
  ) {}

  get active(): string | null {
    return this.front?.sync.id ?? null;
  }

  has(id: string) {
    return this.open.some((tab) => tab.sync.id === id);
  }

  private find(id: string) {
    return this.open.find((tab) => tab.sync.id === id);
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
    await tab.sync.idle();
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
    await tab.sync.idle();
    try {
      await invoke(how === "close" ? "close_work" : "discard_work", { id: tab.sync.id });
    } catch (e) {
      this.events.error(String(e));
    }
    const index = this.open.indexOf(tab);
    this.open.splice(index, 1);
    tab.destroy();
    if (tab === this.front) {
      this.front = null;
      const next = this.open[Math.min(index, this.open.length - 1)];
      if (next) await this.activate(next.sync.id);
      else this.show(null);
    }
    this.events.changed();
  }

  async closeAll() {
    for (const tab of [...this.open]) await this.close(tab.sync.id);
  }

  /** A Work was renamed (here, or on another Device and followed). */
  renamed(id: string, to: string) {
    const tab = this.find(id);
    if (!tab) return;
    tab.rename(to, to.split("/").pop() ?? to);
    this.events.changed();
  }

  /** What the timer said about a Work (Rust names it by the ID the tab had). */
  status(id: string, state: WorkState) {
    this.find(id)?.sync.status(state);
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
    for (const tab of this.open) cursors[tab.sync.id] = tab.view.state.selection.main.head;
    return { tabs: this.open.map((t) => t.sync.id), active: this.active, cursors };
  }

  /** Waits until everything typed in every tab has reached Rust. */
  async idle() {
    for (const tab of this.open) await tab.sync.idle();
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
    this.showMoved(tab);
  }

  /** A banner for a tab whose Work went to the trash, or vanished, on another Device. */
  private showMoved(tab: Tab | null) {
    const el = this.lines.moved;
    el.replaceChildren();
    const kind = tab?.state?.moved?.kind;
    el.hidden = !tab || (kind !== "trashed" && kind !== "gone");
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
      text.textContent = `${tab.name} is no longer in the Vault (deleted outside the app). Nothing is saved until you choose.`;
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
