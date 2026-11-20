// The Vault browser: the Vault's Works and folders in the sidebar, what can be done with them,
// and the trash under them. Folders fold in and out (which are open is remembered on this
// Device, per Vault), Works are shaded pills, and a right-click gives what can be done with one
// (its Preamble… and Mods… are also in the tab row, for the Work in front). Works selected with Ctrl+click become a Collection.

import { invoke } from "@tauri-apps/api/core";
import type { Tabs, Layout, OpenedWork } from "./tabs";
import { buildTree, folderOf, foldersOf, hiddenFolderAt, withoutHidden, type Folder } from "./tree";
import { movedFolders, type Movable } from "./vaultmoves";
import { showMenu, type MenuItem } from "./menu";
import { colourOf, COLOURS } from "./reviewers";
import { vaultModInUse, type ModsState } from "./mods";
import { endWorkDrag, startWorkDrag } from "./inclusions";
import { browseForVault, ensureFilesAccess, onAndroid } from "./android";
import type { ReadingPanel, ReadingPlace } from "./reading";
import type { LibraryPanel } from "./library";
import type { PhoneLayout } from "./phone";
import type { SyncPanel } from "./syncpanel";
import type { PreambleActions } from "./preambleactions";
import { FOLDER_DRAG } from "./movingworks";
import { $, act, actionButton, ask, askText, clearError, confirmAsk, FOLDER_HINT, showError, showProblems, small, type SyncProblem } from "./ui";

// The same shapes as the Rust side's (vault.rs, lib.rs).
type VaultOpened =
  | { status: "notAVault" }
  | { status: "opened"; root: string; deviceName: string | null; suggestedName: string };

type EntryKind =
  | { kind: "work" }
  | { kind: "newWork" }
  | { kind: "leftover"; of: string }
  | { kind: "newer"; version: number }
  | { kind: "unreadable"; reason: string }
  | { kind: "blocked"; reason: string };

export type Entry = { id: string; name: string } & EntryKind;
/** `hidden`: the Hidden folders on this Device (vault/hidden.rs), ones gone from the Vault too. */
type Listing = { entries: Entry[]; problems: SyncProblem[]; folders: string[]; hidden: string[]; colours: Colours };
/** The colours of Works (by ID) and folders (by path), each a Reviewer colour slot (vault/colours.rs). */
export type Colours = { works: Record<string, string>; folders: Record<string, string> };
type TrashedWork = { id: string; name: string };

/** What the Vault browser needs from the rest of the window. */
export type VaultBrowserDeps = {
  tabs: Tabs;
  readingPanel: ReadingPanel;
  library: LibraryPanel;
  phone: PhoneLayout;
  syncPanel: SyncPanel;
  preamble: PreambleActions;
  findInVault: { reset(): void; refresh(): Promise<void> };
  /** Reads the Vault's settings and applies them. */
  loadSettings(): Promise<void>;
  openWork(id: string, cursor?: number, restoring?: boolean): Promise<void>;
  readBeside(id: string): void;
  includeIn(into: string, work: string, at: { x: number; y: number }): Promise<void>;
  editMods(id: string): Promise<void>;
  /** Moving by dragging, or with Move to… (movingworks.ts). */
  startMove(item: Movable): void;
  endMove(): void;
  moveToItem(item: Movable, at: { x: number; y: number }): MenuItem[];
};

/** What's said under a Work that can't simply be opened. */
const LABELS: Record<Entry["kind"], string> = {
  work: "",
  newWork: "Text file only: opening it starts its History",
  leftover: "",
  newer: "Made by a newer version of the app",
  unreadable: "Can't be read right now",
  blocked: "",
};

/** The line under an entry in the list, if any. */
export function entryNote(entry: Entry): string {
  return entry.kind === "leftover" ? `Text file left over from ${entry.of}`
    : entry.kind === "blocked" ? entry.reason
    : LABELS[entry.kind];
}

/** Can't be opened from the list (only seen, or deleted). */
export const isDisabled = (entry: Entry) => entry.kind === "newer" || entry.kind === "unreadable" || entry.kind === "leftover" || entry.kind === "blocked";

/** A folder's new path as the Vault gives it, from what the Author typed. */
export const folderPath = (name: string) => name.split("/").map((p) => p.trim()).join("/");

/** The Works picked for a Collection, after the Work `from` became `to`. */
export const pickedAfterRename = (picked: string[], from: string, to: string) => picked.map((p) => (p === from ? to : p));

/** The Works picked for a Collection, after the folder `path` became `to`. */
export const pickedAfterFolderMove = (picked: string[], path: string, to: string) =>
  picked.map((p) => (p.startsWith(`${path}/`) ? to + p.slice(path.length) : p));

export class VaultBrowser {
  /** A Vault is open, and done opening. */
  isOpen = false;
  entries: Entry[] = [];
  folders: string[] = [];
  /** The Hidden folders on this Device: they and everything in them are left out of the Vault browser. */
  hiddenFolders: string[] = [];
  colours: Colours = { works: {}, folders: {} };
  /** The Vault's Mods as last read: which Vault Mods it has, and which this Device hides. */
  vaultMods: ModsState | null = null;
  private root = "";
  private openFolders = new Set<string>();
  /** The Works selected for a new Collection, in the order they were picked. */
  private picked: string[] = [];

  constructor(private deps: VaultBrowserDeps) {
    // Right-click: what can be done with a Work, a folder, or the Vault.
    // (Only in the Vault's part of the sidebar: the Chapters below aren't Works.)
    $("sidebar-vault").addEventListener("contextmenu", (e) => this.contextMenu(e));
    $("empty-trash").addEventListener("click", () => act(async () => {
      if (!(await confirmAsk("Delete everything in the trash for good, on every Device? This can't be undone.", "Empty the trash"))) return;
      await invoke("empty_trash");
      await this.refresh();
    }));
    $("choose-vault").addEventListener("click", () => act(async () => {
      // Android's own picker gives no file path, so the app has its own folder browser there.
      if (onAndroid && !(await ensureFilesAccess())) return;
      const path = onAndroid ? await browseForVault() : await invoke<string | null>("pick_folder");
      if (path) await this.open(path);
    }));
  }

  /** Is the Colours Vault Mod at work here? Its colours are kept either way. */
  coloursOn() {
    return this.vaultMods !== null && vaultModInUse(this.vaultMods, "colours");
  }

  /** A Work's or folder's colour as CSS, if it has one and Colours is on. */
  shownColour(slot: string | undefined): string | null {
    return slot && this.coloursOn() ? colourOf(slot) : null;
  }

  /** Gives a list item its colour (on its name; for a folder, also its chevron and line). */
  private paint(item: HTMLElement, slot: string | undefined) {
    const css = this.shownColour(slot);
    if (!css) return;
    item.classList.add("coloured");
    item.style.setProperty("--item-colour", css);
  }

  /** Colour… on a Work or folder: a row of the colours, "no colour" first, at (x, y). */
  private pickColour(x: number, y: number, current: string | undefined, save: (colour: string) => Promise<unknown>) {
    const choose = (colour: string) => () => act(async () => {
      await save(colour);
      await this.refresh();
      void this.deps.findInVault.refresh();
    });
    showMenu(x, y, [{
      swatches: [
        { label: "No colour", css: null, chosen: !current, run: choose("") },
        ...COLOURS.map((c) => ({ label: c[0].toUpperCase() + c.slice(1), css: colourOf(c), chosen: current === c, run: choose(c) })),
      ],
    }]);
  }

  private async askDeviceName(suggested: string) {
    const input = $<HTMLInputElement>("name-input");
    input.value = suggested;
    // A Device needs a name before it writes anything; keep asking until it has one.
    for (;;) {
      await ask($<HTMLDialogElement>("ask-name"));
      try {
        await invoke("set_device_name", { name: input.value });
        return;
      } catch (e) {
        showError(e);
      }
    }
  }

  /** Opens the Vault at `path` (asking first if it isn't one yet), with the tabs it had open last time. */
  async open(path: string) {
    const { tabs, readingPanel, findInVault, syncPanel, library, phone } = this.deps;
    clearError();
    await tabs.closeAll();
    readingPanel.close(false);
    findInVault.reset();
    this.isOpen = false;
    let opened = await invoke<VaultOpened>("open_vault", { path, create: false });
    if (opened.status === "notAVault") {
      $("ask-create-path").textContent = path;
      if ((await ask($<HTMLDialogElement>("ask-create"))) !== "yes") return;
      opened = await invoke<VaultOpened>("open_vault", { path, create: true });
    }
    if (opened.status !== "opened") return;
    showVault(opened.root);
    this.root = opened.root;
    this.loadOpenFolders();
    $("vault-actions").hidden = false;
    syncPanel.start();
    if (opened.deviceName === null) await this.askDeviceName(opened.suggestedName);
    await this.deps.loadSettings();
    await this.refresh();
    $("library").hidden = false;
    await library.refresh();
    // The tabs that were open here last time, each with its cursor where it was.
    const layout = await invoke<Layout & { reading?: ReadingPlace | null }>("get_layout");
    for (const id of layout.tabs) {
      try {
        await this.deps.openWork(id, layout.cursors[id], true);
      } catch {
        // renamed or gone since: just not reopened
      }
    }
    if (layout.active && tabs.has(layout.active)) await tabs.activate(layout.active);
    if (layout.reading) await readingPanel.open(layout.reading.work, layout.reading.identity, layout.reading.top);
    // On the phone: writing, if a Work was open last time.
    phone.show(tabs.active ? "write" : "works");
    this.isOpen = true;
  }

  private foldersKey() {
    return `open-folders:${this.root}`;
  }

  private loadOpenFolders() {
    try {
      this.openFolders = new Set(JSON.parse(localStorage.getItem(this.foldersKey()) ?? "[]"));
    } catch {
      this.openFolders = new Set();
    }
  }

  private saveOpenFolders() {
    try {
      localStorage.setItem(this.foldersKey(), JSON.stringify([...this.openFolders]));
    } catch {
      // not remembered, then: no harm done
    }
  }

  /** Opens these folders in the list (remembered), e.g. the ones a moved Work is in now. */
  openFoldersOf(paths: string[]) {
    paths.forEach((f) => this.openFolders.add(f));
    this.saveOpenFolders();
  }

  /** One Work (or Text file) in the list. */
  private workItem(entry: Entry): HTMLElement {
    const item = document.createElement("li");
    item.className = `work-entry ${entry.kind}`;
    item.dataset.id = entry.id;
    this.paint(item, this.colours.works[entry.id]);
    const pill = document.createElement("div");
    pill.className = "entry";
    const name = document.createElement("button");
    name.type = "button";
    name.className = "entry-name";
    name.textContent = entry.name;
    name.title = entry.id;
    name.disabled = isDisabled(entry);
    name.addEventListener("click", (e) => {
      // Ctrl+click (⌘ on a Mac) picks it for a new Collection instead.
      if ((e.ctrlKey || e.metaKey) && entry.kind === "work") this.togglePicked(entry.id);
      else act(() => this.deps.openWork(entry.id));
    });
    pill.append(name);
    if (entry.kind === "work") {
      // Dragged into a Work's text, it's included there; onto a folder, it moves there.
      pill.draggable = true;
      pill.addEventListener("dragstart", (e) => {
        startWorkDrag(e, entry.id);
        this.deps.startMove({ kind: "work", id: entry.id });
      });
      pill.addEventListener("dragend", () => {
        endWorkDrag();
        this.deps.endMove();
      });
    }
    const actions = document.createElement("span");
    actions.className = "entry-actions";
    if (entry.kind === "leftover") {
      actions.append(actionButton("Delete it", "Delete the leftover Text file", () => this.deleteLeftover(entry.id)));
    }
    if (actions.childElementCount > 0) pill.append(actions);
    item.append(pill);
    const note = entryNote(entry);
    if (note) item.append(small(note));
    return item;
  }

  /** A folder, with what's in it (shown if it's open). */
  private folderItem(folder: Folder<Entry>): HTMLElement {
    const item = document.createElement("li");
    item.className = "folder";
    item.dataset.path = folder.path;
    this.paint(item, this.colours.folders[folder.path]);
    const isOpen = this.openFolders.has(folder.path);
    item.classList.toggle("expanded", isOpen);
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "folder-toggle";
    toggle.setAttribute("aria-expanded", String(isOpen));
    const chevron = document.createElement("span");
    chevron.className = "chevron";
    chevron.textContent = "▸";
    toggle.append(chevron, document.createTextNode(folder.name));
    // Dragged by its row onto another folder (or the space around the list), it moves there.
    // The whole item is what's dragged, as WebKit won't drag a button itself; a press anywhere
    // else in it (on "Empty") doesn't drag, and a drag of something inside it isn't this one's.
    item.draggable = true;
    let pressedRow = false;
    // (mousedown, not pointerdown: WebKit sends no pointerdown for the press after a drag.)
    item.addEventListener("mousedown", (e) => (pressedRow = toggle.contains(e.target as Node)));
    item.addEventListener("dragstart", (e) => {
      if (e.target !== item) return;
      if (!pressedRow) return e.preventDefault();
      e.dataTransfer?.setData(FOLDER_DRAG, folder.path); // some engines only drag with data
      if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
      this.deps.startMove({ kind: "folder", path: folder.path });
    });
    item.addEventListener("dragend", (e) => {
      if (e.target === item) this.deps.endMove();
    });
    toggle.addEventListener("click", () => {
      if (this.openFolders.has(folder.path)) this.openFolders.delete(folder.path);
      else this.openFolders.add(folder.path);
      this.saveOpenFolders();
      const open = this.openFolders.has(folder.path);
      item.classList.toggle("expanded", open);
      toggle.setAttribute("aria-expanded", String(open));
      children.hidden = !open;
    });
    const children = document.createElement("ul");
    children.hidden = !isOpen;
    this.fillList(children, folder);
    item.append(toggle, children);
    return item;
  }

  private fillList(list: HTMLElement, folder: Folder<Entry>) {
    for (const sub of folder.folders) list.append(this.folderItem(sub));
    for (const entry of folder.items) list.append(this.workItem(entry));
    if (folder.path && folder.folders.length === 0 && folder.items.length === 0) {
      const empty = document.createElement("li");
      empty.className = "folder-empty muted";
      empty.textContent = "Empty";
      list.append(empty);
    }
  }

  /** The tree the Vault browser shows: everything but what's in Hidden folders. */
  shownTree(): Folder<Entry> {
    const shown = withoutHidden(this.entries, this.folders, this.hiddenFolders);
    return buildTree(shown.items, shown.folders);
  }

  /** The folders the Vault browser shows (no Hidden ones). */
  shownFolders(): string[] {
    return withoutHidden(this.entries, this.folders, this.hiddenFolders).folders;
  }

  /** How many folders are hidden. */
  hiddenCount(): number {
    return withoutHidden(this.entries, this.folders, this.hiddenFolders).shut.length;
  }

  /** Lists the Vault again (as Rust finds it now), and the trash. */
  async refresh() {
    const listing = await invoke<Listing>("list_works");
    this.entries = listing.entries;
    this.folders = listing.folders;
    this.hiddenFolders = listing.hidden ?? [];
    this.colours = listing.colours ?? { works: {}, folders: {} };
    this.vaultMods = await invoke<ModsState>("get_mods", { id: null });
    showProblems(listing.problems);
    const list = $("works");
    list.replaceChildren();
    this.fillList(list, this.shownTree());
    this.showPicked();
    this.highlightOpen();
    await this.refreshTrash();
    void this.deps.readingPanel.refresh(); // synced in, renamed, trashed or restored
  }

  /**
   * Marks the Work in front, and opens the folders it's in so it can be seen. One in a Hidden
   * folder can't be: a line above the list says where it is instead.
   */
  highlightOpen() {
    const { tabs } = this.deps;
    const active = tabs.active;
    const shutIn = active && tabs.activeIsWork ? hiddenFolderAt(folderOf(active), this.hiddenFolders) : null;
    this.showHiddenNote(active, shutIn);
    if (active && !shutIn) {
      const hidden = foldersOf(active).filter((f) => !this.openFolders.has(f));
      if (hidden.length > 0) {
        this.openFoldersOf(hidden);
        void this.refresh();
        return;
      }
    }
    for (const item of $("works").querySelectorAll<HTMLElement>("li.work-entry")) {
      item.classList.toggle("open", item.dataset.id === active);
    }
  }

  /** "Rain v1 is in a hidden folder (Old drafts) · Unhide Old drafts", or nothing. */
  private showHiddenNote(active: string | null, shutIn: string | null) {
    const note = $("hidden-note");
    note.hidden = !active || !shutIn;
    if (!active || !shutIn) return;
    const text = document.createElement("span");
    text.textContent = `${active.split("/").pop()} is in a hidden folder (${shutIn}).`;
    note.replaceChildren(text, document.createTextNode(" "),
      actionButton(`Unhide ${shutIn.split("/").pop()}`, `Show ${shutIn} in the Vault browser again, on every device`, () => this.setFolderHidden(shutIn, false)));
  }

  /** Hides a folder in the Vault browser (on every Device, unless this one keeps its own list), or shows it again. */
  async setFolderHidden(path: string, hidden: boolean) {
    await invoke<string[]>("set_folder_hidden", { path, hidden });
    await this.refresh();
    void this.deps.findInVault.refresh(); // its Works are searched again, or no longer
  }

  async newWork(name?: string, folder?: string) {
    const id = name ?? (await askText("What's the new Work called?", folder ? `${folder}/` : "", "Create"));
    if (id === null) return;
    const created = await invoke<OpenedWork>("create_work", { id });
    await this.deps.tabs.add(created);
    await this.deps.preamble.refreshElements(created.id);
    await this.refresh();
  }

  private async newFolder(parent?: string) {
    const path = await askText("What's the new folder called?", parent ? `${parent}/` : "", "Create",
      "Put it inside another folder with a slash, e.g. Short stories/Old.");
    if (path === null) return;
    const made = await invoke<string>("create_folder", { path });
    this.openFoldersOf(foldersOf(`${made}/x`)); // it and the folders it's in
    await this.refresh();
  }

  private async renameFolder(path: string) {
    const name = await askText(`Rename the folder ${path} to:`, path, "Rename",
      "Everything in it goes along. Move it into another folder with a slash.");
    if (name === null || name === path) return;
    await this.moveFolder(path, name); // folders that were open stay open under their new name
    await this.refresh();
  }

  private async trashFolder(path: string) {
    const { tabs } = this.deps;
    for (const id of tabs.layout().tabs) {
      if (id.startsWith(`${path}/`)) await tabs.close(id); // saved first
    }
    await invoke("trash_folder", { path });
    await this.refresh();
  }

  private async deleteLeftover(id: string) {
    await invoke("delete_leftover", { id });
    await this.refresh();
  }

  private contextMenu(e: MouseEvent) {
    if (!this.isOpen) return;
    e.preventDefault();
    const { tabs, preamble, readBeside, editMods, includeIn, moveToItem } = this.deps;
    const target = e.target as HTMLElement;
    const workEl = target.closest<HTMLElement>("li.work-entry");
    const folderEl = target.closest<HTMLElement>("li.folder");
    const run = (f: () => Promise<unknown>) => () => act(f);
    const folderHere = workEl ? foldersOf(workEl.dataset.id ?? "").pop() : folderEl?.dataset.path;
    // With Works selected (Ctrl+click, or "Select" here): a Collection of them.
    const collecting: MenuItem[] = this.picked.length === 0 ? [] : [
      "separator",
      { label: `New Collection from selected Works (${this.picked.length})…`, run: run(() => this.newCollection([...this.picked], folderHere)) },
      { label: "Clear the selection", run: () => { this.picked = []; this.showPicked(); } },
    ];
    let items: MenuItem[];
    if (workEl) {
      const entry = this.entries.find((x) => x.id === workEl.dataset.id);
      if (!entry) return;
      const folder = foldersOf(entry.id).pop();
      // Included at the cursor in the Work in front (not in itself).
      const into = tabs.activeIsWork && tabs.active !== entry.id ? tabs.active : null;
      const at = { x: e.clientX, y: e.clientY };
      items =
        entry.kind === "work" ? [
          { label: "Open", run: run(() => this.deps.openWork(entry.id)) },
          { label: "Read beside", run: () => readBeside(entry.id) },
          ...(into ? [{ label: `Include in ${into.split("/").pop()}…`, run: run(() => includeIn(into, entry.id, at)) }] : []),
          {
            label: this.picked.includes(entry.id) ? "Deselect" : "Select for a Collection",
            run: () => this.togglePicked(entry.id),
          },
          { label: "Preamble…", run: run(() => preamble.editPreamble(entry.id)) },
          { label: "Mods…", run: run(() => editMods(entry.id)) },
          ...(this.coloursOn() ? [{ label: "Colour…", run: () => this.pickColour(at.x, at.y, this.colours.works[entry.id], (colour) => invoke("set_work_colour", { id: entry.id, colour })) }] : []),
          { label: "Name a Milestone…", run: run(() => preamble.nameMilestone(entry.id)) },
          { label: "Export PDF…", run: run(() => preamble.exportPdf(entry.id)) },
          { label: "Export .docx…", run: run(() => preamble.exportDocx(entry.id)) },
          "separator",
          { label: "Rename…", run: run(() => this.renameWork(entry.id)) },
          ...moveToItem({ kind: "work", id: entry.id }, at),
          { label: "Move to trash", run: run(() => this.trashWork(entry.id)) },
          "separator",
          { label: folder ? "New Work in this folder…" : "New Work…", run: run(() => this.newWork(undefined, folder)) },
        ]
        : entry.kind === "newWork" ? [
          { label: "Open (starts its History)", run: run(() => this.deps.openWork(entry.id)) },
          { label: "Read beside", run: () => readBeside(entry.id) },
        ]
        : entry.kind === "leftover" ? [{ label: "Delete the leftover Text file", run: run(() => this.deleteLeftover(entry.id)) }]
        : [];
    } else if (folderEl) {
      const path = folderEl.dataset.path!;
      items = [
        { label: "New Work here…", run: run(() => this.newWork(undefined, path)) },
        { label: "New folder here…", run: run(() => this.newFolder(path)) },
        "separator",
        { label: "Rename folder…", run: run(() => this.renameFolder(path)) },
        ...moveToItem({ kind: "folder", path }, { x: e.clientX, y: e.clientY }),
        { label: "Hide folder", run: run(() => this.setFolderHidden(path, true)) },
        ...(this.coloursOn() ? [{ label: "Colour…", run: () => this.pickColour(e.clientX, e.clientY, this.colours.folders[path], (colour) => invoke("set_folder_colour", { path, colour })) }] : []),
        { label: "Move folder to trash", run: run(() => this.trashFolder(path)) },
      ];
    } else {
      items = [
        { label: "New Work…", run: run(() => this.newWork()) },
        { label: "New folder…", run: run(() => this.newFolder()) },
      ];
    }
    showMenu(e.clientX, e.clientY, items.length > 0 ? [...items, ...collecting] : collecting.slice(1));
  }

  private async renameWork(id: string) {
    const name = await askText(`Rename ${id} to:`, id, "Rename");
    if (name === null || name === id) return;
    await this.moveWork(id, name);
    await this.refresh();
  }

  /** Renames a Work, maybe into another folder: its tab, and its place in a Collection being picked, go along. */
  async moveWork(id: string, name: string): Promise<string> {
    const renamed = await invoke<string>("rename_work", { id, name });
    this.deps.tabs.renamed(id, renamed);
    this.picked = pickedAfterRename(this.picked, id, renamed);
    return renamed;
  }

  /** Renames a folder, maybe into another: the tabs of Works in it, and the folders open in it, go along. */
  async moveFolder(path: string, name: string): Promise<string> {
    const moved = await invoke<[string, string][]>("rename_folder", { path, name });
    for (const [from, to] of moved) this.deps.tabs.renamed(from, to);
    const renamed = folderPath(name);
    this.openFolders = new Set(movedFolders(this.openFolders, path, renamed));
    this.saveOpenFolders();
    this.picked = pickedAfterFolderMove(this.picked, path, renamed);
    return renamed;
  }

  private async trashWork(id: string) {
    await this.deps.tabs.close(id); // saved first
    await invoke("trash_work", { id });
    await this.refresh();
  }

  private async refreshTrash() {
    const trashed = await invoke<TrashedWork[]>("list_trash");
    $("trash").hidden = trashed.length === 0;
    const list = $("trash-list");
    list.replaceChildren();
    for (const work of trashed) {
      const item = document.createElement("li");
      item.append(document.createTextNode(work.id + " "));
      item.append(actionButton("Restore", `Put ${work.id} back`, async () => {
        await invoke("restore_work", { id: work.id });
        await this.refresh();
      }));
      list.append(item);
    }
  }

  // ---- Collections: Works selected in the Vault browser (Ctrl+click, or "Select" in its menu) ----

  private togglePicked(id: string) {
    this.picked = this.picked.includes(id) ? this.picked.filter((p) => p !== id) : [...this.picked, id];
    this.showPicked();
  }

  /** Marks the selected Works in the Vault browser, numbered in the order they'll be in. */
  private showPicked() {
    this.picked = this.picked.filter((id) => this.entries.some((e) => e.id === id && e.kind === "work"));
    for (const item of $("works").querySelectorAll<HTMLElement>("li.work-entry")) {
      const n = this.picked.indexOf(item.dataset.id ?? "");
      item.classList.toggle("picked", n !== -1);
      item.querySelector(".entry-pick")?.remove();
      if (n !== -1) {
        const badge = document.createElement("span");
        badge.className = "entry-pick";
        badge.textContent = String(n + 1);
        badge.title = "Selected for a new Collection, in this place";
        item.querySelector(".entry")?.prepend(badge);
      }
    }
  }

  /** "New Collection from selected Works…": a new Work holding an Inclusion of each, in that order. */
  private async newCollection(works: string[], folder?: string) {
    const id = await askText(`A new Collection of ${works.length === 1 ? "1 Work" : `${works.length} Works`}. What's it called?`, folder ? `${folder}/` : "", "Create",
      `It includes ${works.map((w) => w.split("/").pop()).join(", ")}, in that order. ${FOLDER_HINT}`);
    if (id === null) return;
    const created = await invoke<OpenedWork>("create_collection", { id, works });
    this.picked = [];
    await this.deps.tabs.add(created);
    await this.deps.preamble.refreshElements(created.id);
    await this.refresh();
  }
}

/** The Vault button at the top of the sidebar: the Vault's name (its folder's), its path in the tooltip. */
export function vaultName(root: string): string {
  return root.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || root;
}

function showVault(root: string) {
  $("vault-name").textContent = vaultName(root);
  $("choose-vault").title = `${root}\nClick to open another Vault`;
}
