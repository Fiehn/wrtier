import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type WorkState } from "./sync";
import { Tabs, type Layout, type OpenedWork } from "./tabs";
import { buildTree, foldersOf, type Folder } from "./tree";
import { showMenu, type MenuItem } from "./menu";
import { PagePreview, type Milestone } from "./preview";
import { LibraryPanel } from "./library";
import { libraryPath } from "./libraryfile";
import { PreambleDialog, type Preamble } from "./preambledialog";
import { openReviewers, type Reviewer } from "./reviewers";
import { SidePanel } from "./sidepanel";
import { CommentsPanel } from "./commentspanel";
import { bindingsFrom, isCommand, keyLabel, onBindingsChanged, setBindings, withKey, type CommandId } from "./keys";
import { openShortcuts } from "./shortcutsdialog";

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

type Entry = { id: string; name: string } & EntryKind;
type SyncProblem = { path: string; reason: string };
type Listing = { entries: Entry[]; problems: SyncProblem[]; folders: string[] };
type TrashedWork = { id: string; name: string };
type Settings = {
  theme: "system" | "light" | "dark";
  fontSize: number;
  themeHere: boolean;
  fontSizeHere: boolean;
  lineNumbers: boolean;
  lineNumbersHere: boolean;
  highlightLine: boolean;
  highlightLineHere: boolean;
  previewFollows: boolean;
  previewFollowsHere: boolean;
  commentHighlights: boolean;
  commentHighlightsHere: boolean;
  /** Shortcuts changed from the defaults: command → key, or "none". */
  shortcuts: Record<string, string>;
  shortcutsHere: boolean;
};

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

// ---- Messages and dialogs ----

function showError(e: unknown) {
  const el = $("error");
  el.textContent = String(e);
  el.hidden = false;
}

function clearError() {
  $("error").hidden = true;
}

/** A line saying something went well (an Export), for a few seconds. */
let infoTimer: number | undefined;
function showInfo(text: string) {
  const el = $("info");
  el.textContent = text;
  el.hidden = false;
  clearTimeout(infoTimer);
  infoTimer = window.setTimeout(() => (el.hidden = true), 10_000);
}

function showProblems(problems: SyncProblem[]) {
  const el = $("problems");
  el.replaceChildren();
  el.hidden = problems.length === 0;
  if (problems.length === 0) return;
  const title = document.createElement("strong");
  title.textContent = problems.length === 1 ? "1 Sync problem" : `${problems.length} Sync problems`;
  const list = document.createElement("ul");
  for (const p of problems) {
    const item = document.createElement("li");
    item.textContent = `${p.path}: ${p.reason}`;
    list.append(item);
  }
  el.append(title, list);
}

// Cancel buttons are plain buttons, not submit buttons: pressing Enter in a dialog submits with
// its first submit button, which must be the one that says yes.
for (const button of document.querySelectorAll<HTMLButtonElement>("dialog button[data-close]")) {
  button.addEventListener("click", () => button.closest("dialog")?.close(button.dataset.close));
}

/** Shows a <dialog> and resolves with the button that closed it ("" if Escape). */
function ask(dialog: HTMLDialogElement): Promise<string> {
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue), { once: true });
    dialog.returnValue = "";
    dialog.showModal();
  });
}

const FOLDER_HINT = "Put it in a folder with a slash, e.g. Short stories/Rain.";

/**
 * Asks for a name (of a Work or folder); null if cancelled. An `initial` ending with a slash
 * (a folder to put it in) leaves the cursor after it; otherwise it's all selected, to replace.
 */
async function askText(question: string, initial = "", ok = "OK", hint = FOLDER_HINT): Promise<string | null> {
  $("ask-text-question").textContent = question;
  $("ask-text-ok").textContent = ok;
  $("ask-text-hint").textContent = hint;
  const input = $<HTMLInputElement>("ask-text-input");
  input.value = initial;
  const dialog = $<HTMLDialogElement>("ask-text");
  const answer = ask(dialog);
  if (initial.endsWith("/")) input.setSelectionRange(initial.length, initial.length);
  else input.select();
  return (await answer) === "ok" ? input.value : null;
}

async function confirmAsk(text: string, yes: string): Promise<boolean> {
  $("ask-confirm-text").textContent = text;
  $("ask-confirm-yes").textContent = yes;
  return (await ask($<HTMLDialogElement>("ask-confirm"))) === "yes";
}

/** Runs something the Author asked for, showing any error. */
function act(run: () => Promise<unknown>) {
  clearError();
  run().catch(showError);
}

// ---- Tabs ----

const tabs = new Tabs($("tabbar"), $("panes"), { notice: $("notice"), status: $("status"), moved: $("moved"), style: $("style-notice") }, {
  changed: () => {
    $("empty").hidden = tabs.active !== null;
    // A library file is plain Typst: no Formatted view, and no pages of its own.
    $("view-mode").hidden = !tabs.activeIsWork;
    $("panel-toggle").hidden = !tabs.activeIsWork;
    preview.show(tabs.activeIsWork ? tabs.active : null);
    commentsPanel.render();
    rememberSoon();
    highlightOpen();
  },
  error: showError,
  askName: (question, initial) => askText(question, initial),
  edited: (id) => preview.edited(id),
  committed: (id) => preview.committed(id),
  cursor: (id) => preview.cursor(id),
  closed: (id) => preview.forget(id),
  addElement: (id, name) => act(() => addElement(id, name)),
  openComments: (_id, ids) => {
    panel.setTab("comments");
    commentsPanel.focus(ids);
  },
  marks: (id) => {
    if (id === tabs.active) commentsPanel.render();
  },
  addComment: (id) => startComment(id),
  selectionMenu: (id, x, y) => showMenu(x, y, selectionMenu(id)),
});

// ---- The side panel (Ctrl+Shift+P): the Page preview and the Comments ----

const preview = new PagePreview($("preview-bar"), $("preview-pages"), $<HTMLSelectElement>("preview-look"), $<HTMLSelectElement>("preview-version"), {
  settled: (id) => tabs.settled(id),
  reveal: (id, from, to) => void tabs.reveal(id, from, to),
  cursorOf: (id) => tabs.cursorOf(id),
  looksOf: async (id) => (await invoke<{ preamble: Preamble }>("get_preamble", { id })).preamble.looks,
  milestonesOf: (id) => invoke<Milestone[]>("get_milestones", { id }),
  error: showError,
});

// Shown or not, and which tab, are remembered on this Device. The pages only compile while
// their tab is showing.
const panel = new SidePanel(
  $("side-panel"),
  [$("panel-tab-pages"), $("panel-tab-comments")],
  { pages: $("preview"), comments: $("comments-pane") },
  $("panel-toggle"),
  (pagesInView) => preview.setOpen(pagesInView),
);

// Resolved Comments' highlights: shown or not on this Device.
const showResolved = $<HTMLInputElement>("comments-show-resolved");
try {
  showResolved.checked = localStorage.getItem("comments-resolved") === "shown";
} catch {
  // not remembered: hidden
}
tabs.setCommentOptions({ ...tabs.commentOptions, resolved: showResolved.checked });
showResolved.addEventListener("change", () => {
  tabs.setCommentOptions({ ...tabs.commentOptions, resolved: showResolved.checked });
  commentsPanel.render();
  try {
    localStorage.setItem("comments-resolved", showResolved.checked ? "shown" : "hidden");
  } catch {
    // not remembered, then: no harm done
  }
});

// ---- Comments ----

const commentsPanel = new CommentsPanel($("comments-list"), $("comments-new"), {
  work: () => (tabs.activeIsWork ? tabs.active : null),
  marks: (work) => tabs.marksOf(work),
  act: (work, op) => tabs.comment(work, op),
  show: (work, comment, part) => tabs.showComment(work, comment, part),
  setPending: (work, range) => tabs.setPending(work, range),
  pendingRange: (work) => tabs.pendingRange(work),
  reviewers: () => invoke<Reviewer[]>("get_reviewers"),
  authorName: async (work) => {
    try {
      const author = (await invoke<{ preamble: Preamble }>("get_preamble", { id: work })).preamble.author.trim();
      return author || "Author";
    } catch {
      return "Author";
    }
  },
  newReviewer: async () => {
    const name = await askText("Who is the new Reviewer?", "", "Add", "They're added to the Vault, so they can comment on any Work. Their role and colour can be changed under Reviewers.");
    if (name === null || !name.trim()) return null;
    return invoke<Reviewer>("add_reviewer", { name, role: "" });
  },
  showResolved: () => showResolved.checked,
  counted: (open) => {
    $("panel-tab-comments").textContent = open > 0 ? `Comments (${open})` : "Comments";
  },
  error: showError,
});

/** Its key (Ctrl+Shift+M at first) or "Add Comment": a new Comment on the selection, or the word at the cursor. */
function startComment(id: string) {
  const target = tabs.commentTarget(id);
  if (!target) {
    showError("Select some text to comment on, or put the cursor in a word.");
    return;
  }
  panel.setTab("comments");
  act(() => commentsPanel.startAdding(id, target));
}

/** Right-click inside a selection: Add Comment, and the editing the system menu would have. */
function selectionMenu(id: string): MenuItem[] {
  const view = tabs.viewOf(id);
  if (!view) return [];
  // Cut and Copy go through the editor, so a cut pasted back is still a Move.
  const command = (name: "cut" | "copy") => () => {
    view.focus();
    document.execCommand(name);
  };
  return [
    { label: withKey("Add Comment", "addComment"), run: () => startComment(id) },
    "separator",
    { label: "Cut", run: command("cut") },
    { label: "Copy", run: command("copy") },
    {
      label: "Paste",
      run: () => act(async () => {
        let text: string;
        try {
          text = await navigator.clipboard.readText();
        } catch {
          throw new Error("The clipboard can't be read from this menu here. Use Ctrl+V.");
        }
        const { from, to } = view.state.selection.main;
        view.focus();
        view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length }, userEvent: "input.paste", scrollIntoView: true });
      }),
    },
  ];
}

$("panel-toggle").addEventListener("click", () => panel.toggle());
$("panel-close").addEventListener("click", () => panel.setShown(false));

/** The tabs (and cursors) are remembered for next time, a moment after they change. */
let rememberTimer: number | undefined;
function rememberSoon() {
  clearTimeout(rememberTimer);
  rememberTimer = window.setTimeout(remember, 500);
}
function remember() {
  if (!vaultOpen) return;
  invoke("save_layout", { layout: tabs.layout() }).catch(showError);
}

// ---- The Style library ----

const library: LibraryPanel = new LibraryPanel($("library-list"), $("library-conflicts"), {
  open: (path, title) => openLibraryFile(path, title),
  error: showError,
});

async function openLibraryFile(path: string, title: string = library.titleOf(path)): Promise<void> {
  const text = await invoke<string>("read_library", { path });
  await tabs.addLibrary({ path, title, text });
}

// The library changed: in a tab here, in another editor, or from another Device. The Page
// preview uses the library's versions, so it's compiled again too.
void listen("library-changed", () => {
  library.refresh().then(() => refreshElements()).catch(showError);
  void tabs.libraryChanged();
  void preview.preambleChanged(tabs.activeIsWork ? tabs.active : null);
});

// ---- A Work's Preamble ----

const preambleDialog = new PreambleDialog(ask);

async function editPreamble(id: string) {
  await library.refresh(); // the dialog offers what the library has now
  const name = id.split("/").pop() ?? id;
  const state = await preambleDialog.open(id, name, library.items, library.builtIn);
  if (state === undefined) return; // cancelled
  if (state) tabs.status(id, state);
  await preambleChanged(id);
}

/** Exports a Work as a PDF, in a Look picked here if it has more than one. */
async function exportPdf(id: string) {
  await tabs.settled(id);
  const { preamble } = await invoke<{ preamble: Preamble }>("get_preamble", { id });
  let look: string | null = preamble.looks[0] ?? null;
  if (preamble.looks.length > 1) {
    const select = $<HTMLSelectElement>("ask-export-look");
    select.replaceChildren(...preamble.looks.map((name, i) => {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = i === 0 ? `${name} (the Work's own)` : name;
      return option;
    }));
    select.value = preview.lookFor(id) ?? preamble.looks[0];
    $("ask-export-heading").textContent = `Export ${id.split("/").pop()} as PDF`;
    if ((await ask($<HTMLDialogElement>("ask-export"))) !== "export") return;
    look = select.value;
  }
  type Exported = { path: string; pages: number; deviceFonts: string[] };
  const done = await invoke<Exported | null>("export_pdf", { id, look });
  if (!done) return; // cancelled
  const pages = done.pages === 1 ? "1 page" : `${done.pages} pages`;
  const fonts = done.deviceFonts.length
    ? ` ${done.deviceFonts.join(", ")} came from this Device, not the Vault, so a PDF from another Device would differ.`
    : "";
  showInfo(`Exported ${pages} to ${done.path}.${fonts}`);
}

$("preview-pdf").addEventListener("click", () => act(async () => {
  if (preview.shown) await exportPdf(preview.shown);
}));

/** Exports a Work as a manuscript .docx, and says what it couldn't carry. */
async function exportDocx(id: string) {
  await tabs.settled(id);
  $("ask-docx-heading").textContent = `Export ${id.split("/").pop()} as .docx`;
  const select = $<HTMLSelectElement>("ask-docx-font");
  try {
    select.value = localStorage.getItem("docx-font") ?? "Times New Roman";
  } catch {
    // nothing remembered
  }
  if ((await ask($<HTMLDialogElement>("ask-docx"))) !== "export") return;
  try {
    localStorage.setItem("docx-font", select.value);
  } catch {
    // not remembered, then: no harm done
  }
  type LeftOut = { line: number; what: string };
  type Exported = { path: string; paragraphs: number; leftOut: LeftOut[] };
  const done = await invoke<Exported | null>("export_docx", { id, font: select.value });
  if (!done) return; // cancelled
  let line = `Exported ${done.paragraphs} paragraphs to ${done.path}.`;
  if (done.leftOut.length > 0) {
    const shown = done.leftOut.slice(0, 3).map((l) => `line ${l.line}: ${l.what}`).join("; ");
    const more = done.leftOut.length > 3 ? `, and ${done.leftOut.length - 3} more` : "";
    line += ` Left out (a .docx can't carry it): ${shown}${more}.`;
  }
  showInfo(line);
}

$("preview-docx").addEventListener("click", () => act(async () => {
  if (preview.shown) await exportDocx(preview.shown);
}));

/** Names the current point in a Work's History, to look back at later. (A stopgap until the
 *  History view in Phase 5, which is where Milestones will be made and compared.) */
async function nameMilestone(id: string) {
  const name = await askText(`Name this point in ${id}'s History:`, "", "Name it",
    "E.g. Draft 2, sent to editor. The Page preview can show the Work as it was then, at any time.");
  if (name === null) return;
  const state = await invoke<WorkState | null>("make_milestone", { id, name });
  if (state) tabs.status(id, state);
  await preview.milestonesChanged(id);
}

/** A Work's Preamble changed: its editor's Story elements and its pages follow. */
async function preambleChanged(id: string) {
  await refreshElements(id);
  await preview.preambleChanged(id);
}

/** Tells each open Work's editor (or just `only`) which Story elements it has. */
async function refreshElements(only?: string) {
  for (const id of tabs.works) {
    if (only && id !== only) continue;
    try {
      const { preamble } = await invoke<{ preamble: Preamble }>("get_preamble", { id });
      tabs.setElements(id, library.items.filter((i) => i.kind === "storyElement").map((i) => ({
        name: i.name,
        title: i.title,
        shownAs: i.shownAs ?? "block",
        inWork: library.builtIn.includes(i.name) || preamble.elements.includes(i.name),
      })));
    } catch {
      // gone or unreadable right now: it keeps what it had
    }
  }
}

/** "Add it?": a Story element from the library goes into the Work's Preamble. */
async function addElement(id: string, name: string) {
  const { preamble } = await invoke<{ preamble: Preamble }>("get_preamble", { id });
  if (!preamble.elements.includes(name)) {
    const state = await invoke<WorkState | null>("set_preamble", { id, preamble: { ...preamble, elements: [...preamble.elements, name] } });
    if (state) tabs.status(id, state);
  }
  await preambleChanged(id);
}

$("new-style").addEventListener("click", () => act(async () => {
  const name = await askText("What's the new Style called?", "", "Create", "One small thing it does, e.g. Drop caps or Large print.");
  if (name !== null) await library.create("style", name);
}));
$("new-look").addEventListener("click", () => act(async () => {
  const name = await askText("What's the new Look called?", "", "Create", "What it's for, e.g. Hardback or Large print.");
  if (name !== null) await library.create("look", name);
}));
$("new-element").addEventListener("click", () => act(async () => {
  const field = (id: string) => $<HTMLInputElement>(id);
  for (const id of ["element-name", "element-title", "element-description"]) field(id).value = "";
  const answer = ask($<HTMLDialogElement>("ask-element"));
  field("element-name").focus();
  if ((await answer) !== "ok") return;
  await library.create("storyElement", field("element-name").value.trim(), {
    title: field("element-title").value,
    shownAs: $<HTMLSelectElement>("element-shown").value,
    docx: $<HTMLSelectElement>("element-docx").value,
    description: field("element-description").value,
  });
}));

async function openWork(id: string, cursor?: number) {
  const path = libraryPath(id);
  if (path) return openLibraryFile(path);
  if (tabs.has(id)) return tabs.activate(id);
  const work = await invoke<OpenedWork>("open_work", { id });
  await tabs.add(work, cursor);
  await refreshElements(work.id);
  if (work.problems.length > 0) showProblems(work.problems);
  await refreshList(); // a Text file may have become a Work
}

// The Rust side's timer closed a burst, saved, or noticed a change from outside.
void listen<{ id: string; state: WorkState }>("work-status", (event) => tabs.status(event.payload.id, event.payload.state));

// ---- The Vault ----

let vaultOpen = false;
let entries: Entry[] = [];
let folders: string[] = [];

async function askDeviceName(suggested: string) {
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

async function openVault(path: string) {
  clearError();
  await tabs.closeAll();
  vaultOpen = false;
  let opened = await invoke<VaultOpened>("open_vault", { path, create: false });
  if (opened.status === "notAVault") {
    $("ask-create-path").textContent = path;
    if ((await ask($<HTMLDialogElement>("ask-create"))) !== "yes") return;
    opened = await invoke<VaultOpened>("open_vault", { path, create: true });
  }
  if (opened.status !== "opened") return;
  $("vault-root").textContent = opened.root;
  vaultRoot = opened.root;
  loadOpenFolders();
  $("vault-actions").hidden = false;
  $("open-settings").hidden = false;
  $("open-reviewers").hidden = false;
  if (opened.deviceName === null) await askDeviceName(opened.suggestedName);
  applySettings(await invoke<Settings>("get_settings"));
  await refreshList();
  $("library").hidden = false;
  await library.refresh();
  // The tabs that were open here last time, each with its cursor where it was.
  const layout = await invoke<Layout>("get_layout");
  for (const id of layout.tabs) {
    try {
      await openWork(id, layout.cursors[id]);
    } catch {
      // renamed or gone since: just not reopened
    }
  }
  if (layout.active && tabs.has(layout.active)) await tabs.activate(layout.active);
  vaultOpen = true;
}

const labels: Record<Entry["kind"], string> = {
  work: "",
  newWork: "Text file only: opening it starts its History",
  leftover: "",
  newer: "Made by a newer version of the app",
  unreadable: "Can't be read right now",
  blocked: "",
};

function small(text: string) {
  const el = document.createElement("small");
  el.textContent = text;
  return el;
}

function actionButton(label: string, title: string, run: () => Promise<unknown>) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "link";
  b.textContent = label;
  b.title = title;
  b.addEventListener("click", () => act(run));
  return b;
}

// The Vault browser: folders fold in and out (which are open is remembered on this Device,
// per Vault), Works are shaded pills whose Rename and Trash show on hover, and a right-click
// gives everything else.

let vaultRoot = "";
let openFolders = new Set<string>();
const foldersKey = () => `open-folders:${vaultRoot}`;

function loadOpenFolders() {
  try {
    openFolders = new Set(JSON.parse(localStorage.getItem(foldersKey()) ?? "[]"));
  } catch {
    openFolders = new Set();
  }
}

function saveOpenFolders() {
  try {
    localStorage.setItem(foldersKey(), JSON.stringify([...openFolders]));
  } catch {
    // not remembered, then: no harm done
  }
}

const disabled = (entry: Entry) => entry.kind === "newer" || entry.kind === "unreadable" || entry.kind === "leftover" || entry.kind === "blocked";

/** One Work (or Text file) in the list. */
function workItem(entry: Entry): HTMLElement {
  const item = document.createElement("li");
  item.className = `work-entry ${entry.kind}`;
  item.dataset.id = entry.id;
  const pill = document.createElement("div");
  pill.className = "entry";
  const name = document.createElement("button");
  name.type = "button";
  name.className = "entry-name";
  name.textContent = entry.name;
  name.title = entry.id;
  name.disabled = disabled(entry);
  name.addEventListener("click", () => act(() => openWork(entry.id)));
  pill.append(name);
  const actions = document.createElement("span");
  actions.className = "entry-actions";
  if (entry.kind === "work") {
    actions.append(
      actionButton("Preamble…", withKey(`Looks, Styles, Story elements and settings of ${entry.id}`, "preamble"), () => editPreamble(entry.id)),
      actionButton("Rename", `Rename ${entry.id}`, () => renameWork(entry.id)),
      actionButton("Trash", `Move ${entry.id} to the trash`, () => trashWork(entry.id)),
    );
  } else if (entry.kind === "leftover") {
    actions.append(actionButton("Delete it", "Delete the leftover Text file", () => deleteLeftover(entry.id)));
  }
  if (actions.childElementCount > 0) pill.append(actions);
  item.append(pill);
  const note =
    entry.kind === "leftover" ? `Text file left over from ${entry.of}`
    : entry.kind === "blocked" ? entry.reason
    : labels[entry.kind];
  if (note) item.append(small(note));
  return item;
}

/** A folder, with what's in it (shown if it's open). */
function folderItem(folder: Folder<Entry>): HTMLElement {
  const item = document.createElement("li");
  item.className = "folder";
  item.dataset.path = folder.path;
  const isOpen = openFolders.has(folder.path);
  item.classList.toggle("expanded", isOpen);
  const toggle = document.createElement("button");
  toggle.type = "button";
  toggle.className = "folder-toggle";
  toggle.setAttribute("aria-expanded", String(isOpen));
  const chevron = document.createElement("span");
  chevron.className = "chevron";
  chevron.textContent = "▸";
  toggle.append(chevron, document.createTextNode(folder.name));
  toggle.addEventListener("click", () => {
    if (openFolders.has(folder.path)) openFolders.delete(folder.path);
    else openFolders.add(folder.path);
    saveOpenFolders();
    const open = openFolders.has(folder.path);
    item.classList.toggle("expanded", open);
    toggle.setAttribute("aria-expanded", String(open));
    children.hidden = !open;
  });
  const children = document.createElement("ul");
  children.hidden = !isOpen;
  fillList(children, folder);
  item.append(toggle, children);
  return item;
}

function fillList(list: HTMLElement, folder: Folder<Entry>) {
  for (const sub of folder.folders) list.append(folderItem(sub));
  for (const entry of folder.items) list.append(workItem(entry));
  if (folder.path && folder.folders.length === 0 && folder.items.length === 0) {
    const empty = document.createElement("li");
    empty.className = "folder-empty muted";
    empty.textContent = "Empty";
    list.append(empty);
  }
}

async function refreshList() {
  const listing = await invoke<Listing>("list_works");
  entries = listing.entries;
  folders = listing.folders;
  showProblems(listing.problems);
  const list = $("works");
  list.replaceChildren();
  fillList(list, buildTree(entries, folders));
  highlightOpen();
  await refreshTrash();
}

/** Marks the Work in front, and opens the folders it's in so it can be seen. */
function highlightOpen() {
  const active = tabs.active;
  if (active) {
    const hidden = foldersOf(active).filter((f) => !openFolders.has(f));
    if (hidden.length > 0) {
      hidden.forEach((f) => openFolders.add(f));
      saveOpenFolders();
      void refreshList();
      return;
    }
  }
  for (const item of $("works").querySelectorAll<HTMLElement>("li.work-entry")) {
    item.classList.toggle("open", item.dataset.id === active);
  }
}

async function newWork(name?: string, folder?: string) {
  const id = name ?? (await askText("What's the new Work called?", folder ? `${folder}/` : "", "Create"));
  if (id === null) return;
  const created = await invoke<OpenedWork>("create_work", { id });
  await tabs.add(created);
  await refreshElements(created.id);
  await refreshList();
}

async function newFolder(parent?: string) {
  const path = await askText("What's the new folder called?", parent ? `${parent}/` : "", "Create",
    "Put it inside another folder with a slash, e.g. Short stories/Old.");
  if (path === null) return;
  const made = await invoke<string>("create_folder", { path });
  foldersOf(`${made}/x`).forEach((f) => openFolders.add(f)); // it and the folders it's in
  saveOpenFolders();
  await refreshList();
}

async function renameFolder(path: string) {
  const name = await askText(`Rename the folder ${path} to:`, path, "Rename",
    "Everything in it goes along. Move it into another folder with a slash.");
  if (name === null || name === path) return;
  const moved = await invoke<[string, string][]>("rename_folder", { path, name });
  for (const [from, to] of moved) tabs.renamed(from, to);
  // Folders that were open stay open under their new name.
  const renamed = name.split("/").map((p) => p.trim()).join("/");
  openFolders = new Set([...openFolders].map((f) => (f === path || f.startsWith(`${path}/`) ? renamed + f.slice(path.length) : f)));
  saveOpenFolders();
  await refreshList();
}

async function trashFolder(path: string) {
  for (const id of tabs.layout().tabs) {
    if (id.startsWith(`${path}/`)) await tabs.close(id); // saved first
  }
  await invoke("trash_folder", { path });
  await refreshList();
}

async function deleteLeftover(id: string) {
  await invoke("delete_leftover", { id });
  await refreshList();
}

// Right-click: what can be done with a Work, a folder, or the Vault.
$("sidebar").addEventListener("contextmenu", (e) => {
  if (!vaultOpen) return;
  e.preventDefault();
  const target = e.target as HTMLElement;
  const workEl = target.closest<HTMLElement>("li.work-entry");
  const folderEl = target.closest<HTMLElement>("li.folder");
  const run = (f: () => Promise<unknown>) => () => act(f);
  let items: MenuItem[];
  if (workEl) {
    const entry = entries.find((x) => x.id === workEl.dataset.id);
    if (!entry) return;
    const folder = foldersOf(entry.id).pop();
    items =
      entry.kind === "work" ? [
        { label: "Open", run: run(() => openWork(entry.id)) },
        { label: "Preamble…", run: run(() => editPreamble(entry.id)) },
        { label: "Name a Milestone…", run: run(() => nameMilestone(entry.id)) },
        { label: "Export PDF…", run: run(() => exportPdf(entry.id)) },
        { label: "Export .docx…", run: run(() => exportDocx(entry.id)) },
        "separator",
        { label: "Rename…", run: run(() => renameWork(entry.id)) },
        { label: "Move to trash", run: run(() => trashWork(entry.id)) },
        "separator",
        { label: folder ? "New Work in this folder…" : "New Work…", run: run(() => newWork(undefined, folder)) },
      ]
      : entry.kind === "newWork" ? [{ label: "Open (starts its History)", run: run(() => openWork(entry.id)) }]
      : entry.kind === "leftover" ? [{ label: "Delete the leftover Text file", run: run(() => deleteLeftover(entry.id)) }]
      : [];
  } else if (folderEl) {
    const path = folderEl.dataset.path!;
    items = [
      { label: "New Work here…", run: run(() => newWork(undefined, path)) },
      { label: "New folder here…", run: run(() => newFolder(path)) },
      "separator",
      { label: "Rename folder…", run: run(() => renameFolder(path)) },
      { label: "Move folder to trash", run: run(() => trashFolder(path)) },
    ];
  } else {
    items = [
      { label: "New Work…", run: run(() => newWork()) },
      { label: "New folder…", run: run(() => newFolder()) },
    ];
  }
  showMenu(e.clientX, e.clientY, items);
});

async function renameWork(id: string) {
  const name = await askText(`Rename ${id} to:`, id, "Rename");
  if (name === null || name === id) return;
  const renamed = await invoke<string>("rename_work", { id, name });
  tabs.renamed(id, renamed);
  await refreshList();
}

async function trashWork(id: string) {
  await tabs.close(id); // saved first
  await invoke("trash_work", { id });
  await refreshList();
}

async function refreshTrash() {
  const trashed = await invoke<TrashedWork[]>("list_trash");
  $("trash").hidden = trashed.length === 0;
  const list = $("trash-list");
  list.replaceChildren();
  for (const work of trashed) {
    const item = document.createElement("li");
    item.append(document.createTextNode(work.id + " "));
    item.append(actionButton("Restore", `Put ${work.id} back`, async () => {
      await invoke("restore_work", { id: work.id });
      await refreshList();
    }));
    list.append(item);
  }
}

$("empty-trash").addEventListener("click", () => act(async () => {
  if (!(await confirmAsk("Delete everything in the trash for good, on every Device? This can't be undone.", "Empty the trash"))) return;
  await invoke("empty_trash");
  await refreshList();
}));

$("choose-vault").addEventListener("click", () => act(async () => {
  const path = await invoke<string | null>("pick_folder");
  if (path) await openVault(path);
}));


// The Vault list follows what Syncthing brings (new Works, Conflict copies of Works that
// aren't open): every 10 seconds, and whenever the window gets focus. The tabs and cursors are
// remembered then too.
async function refreshQuietly() {
  if (!vaultOpen || document.querySelector("dialog[open]")) return;
  try {
    await refreshList();
    remember();
  } catch (e) {
    showError(e);
  }
}
setInterval(refreshQuietly, 10_000);
window.addEventListener("focus", refreshQuietly);

// Leaving the window: the Text file is brought up to date for other programs.
window.addEventListener("blur", () => {
  remember();
  void tabs.leave();
});

// ---- Quick switcher (Ctrl+O) ----

/** Does `query` appear in `text`, in order (not necessarily together)? */
function matches(text: string, query: string) {
  let i = 0;
  const t = text.toLowerCase();
  for (const c of query.toLowerCase()) {
    i = t.indexOf(c, i);
    if (i === -1) return false;
    i++;
  }
  return true;
}

let switcherChoice = 0;
function fillSwitcher() {
  const query = $<HTMLInputElement>("switcher-input").value.trim();
  const found = entries.filter((e) => (e.kind === "work" || e.kind === "newWork") && matches(e.id, query)).slice(0, 12);
  const list = $("switcher-list");
  list.replaceChildren();
  const choices: { label: string; run: () => Promise<unknown> }[] = found.map((e) => ({ label: e.id, run: () => openWork(e.id) }));
  if (query && !found.some((e) => e.id.toLowerCase() === query.toLowerCase())) {
    choices.push({ label: `Create “${query}”`, run: () => newWork(query) });
  }
  switcherChoice = Math.min(switcherChoice, Math.max(choices.length - 1, 0));
  choices.forEach((choice, i) => {
    const item = document.createElement("li");
    item.textContent = choice.label;
    item.classList.toggle("chosen", i === switcherChoice);
    item.addEventListener("click", () => {
      $<HTMLDialogElement>("switcher").close();
      act(choice.run);
    });
    list.append(item);
  });
  return choices;
}

async function openSwitcher() {
  if (!vaultOpen) return;
  const dialog = $<HTMLDialogElement>("switcher");
  const input = $<HTMLInputElement>("switcher-input");
  input.value = "";
  switcherChoice = 0;
  fillSwitcher();
  dialog.showModal();
  input.focus();
}

$("switcher-input").addEventListener("input", () => {
  switcherChoice = 0;
  fillSwitcher();
});
$("switcher-input").addEventListener("keydown", (e) => {
  const choices = fillSwitcher();
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    switcherChoice = (switcherChoice + (e.key === "ArrowDown" ? 1 : choices.length - 1)) % Math.max(choices.length, 1);
    fillSwitcher();
  } else if (e.key === "Enter") {
    e.preventDefault();
    $<HTMLDialogElement>("switcher").close();
    const choice = choices[switcherChoice];
    if (choice) act(choice.run);
  }
});
$("open-switcher").addEventListener("click", () => void openSwitcher());

// ---- Hiding the Vault browser, for writing with nothing else on screen (remembered here) ----

function setSidebar(shown: boolean) {
  $("app").classList.toggle("sidebar-hidden", !shown);
  $("sidebar-toggle").title = withKey(shown ? "Hide the Works" : "Show the Works", "toggleWorks");
  $("sidebar-toggle").setAttribute("aria-expanded", String(shown));
  try {
    localStorage.setItem("sidebar", shown ? "shown" : "hidden");
  } catch {
    // not remembered, then: no harm done
  }
}

try {
  setSidebar(localStorage.getItem("sidebar") !== "hidden");
} catch {
  setSidebar(true);
}
$("sidebar-toggle").addEventListener("click", () => setSidebar($("app").classList.contains("sidebar-hidden")));


// ---- Formatted and Raw view (Ctrl+E), remembered on this Device ----

function setFormatted(on: boolean) {
  tabs.setFormatted(on);
  $("view-mode").textContent = on ? "Raw view" : "Formatted view";
  try {
    localStorage.setItem("view", on ? "formatted" : "raw");
  } catch {
    // not remembered, then: no harm done
  }
}

try {
  setFormatted(localStorage.getItem("view") !== "raw");
} catch {
  setFormatted(true);
}
$("view-mode").addEventListener("click", () => setFormatted(!tabs.formatted));

// ---- Keyboard shortcuts: the app-wide ones (the editor's are in each editor's keymap) ----

/** What each app-wide command does; false: it doesn't apply right now (the key is left alone). */
const appCommands: Partial<Record<CommandId, () => boolean>> = {
  toggleWorks: () => {
    setSidebar($("app").classList.contains("sidebar-hidden"));
    return true;
  },
  quickSwitcher: () => {
    void openSwitcher();
    return true;
  },
  closeTab: () => {
    if (!tabs.active) return false;
    act(() => tabs.close(tabs.active!));
    return true;
  },
  toggleView: () => {
    if (!tabs.active) return false;
    setFormatted(!tabs.formatted);
    return true;
  },
  togglePanel: () => {
    panel.toggle();
    return true;
  },
  preamble: () => {
    if (!tabs.activeIsWork || !tabs.active) return false;
    act(() => editPreamble(tabs.active!));
    return true;
  },
};

window.addEventListener("keydown", (e) => {
  for (const [command, run] of Object.entries(appCommands)) {
    if (isCommand(e, command as CommandId) && run()) {
      e.preventDefault();
      return;
    }
  }
}, { capture: true });

/** Everything that names a key says the current one. */
function showKeys() {
  $("open-switcher").title = withKey("Find a Work", "quickSwitcher");
  $("open-switcher-key").textContent = keyLabel("quickSwitcher");
  $("open-switcher-key").hidden = !keyLabel("quickSwitcher");
  $("view-mode").title = withKey("Switch between the Formatted and the Raw view", "toggleView");
  $("panel-toggle").title = withKey("The Page preview and the Comments", "togglePanel");
  $("panel-close").title = withKey("Hide the panel", "togglePanel");
  const switcher = keyLabel("quickSwitcher");
  $("empty").textContent = `Open a Vault, then choose a Work on the left${switcher ? `, or press ${switcher}` : ""}.`;
  $("sidebar-toggle").title = withKey($("app").classList.contains("sidebar-hidden") ? "Show the Works" : "Hide the Works", "toggleWorks");
  tabs.keysChanged();
  commentsPanel.render();
}
onBindingsChanged(showKeys);
showKeys(); // the defaults, until a Vault's settings say otherwise

// ---- Settings ----

function applySettings(settings: Settings) {
  const root = document.documentElement;
  if (settings.theme === "system") delete root.dataset.theme;
  else root.dataset.theme = settings.theme;
  root.style.setProperty("--editor-font-size", `${settings.fontSize}px`);
  $<HTMLSelectElement>("theme").value = settings.theme;
  $<HTMLInputElement>("theme-here").checked = settings.themeHere;
  $<HTMLInputElement>("font-size").value = String(settings.fontSize);
  $<HTMLInputElement>("font-size-here").checked = settings.fontSizeHere;
  $<HTMLInputElement>("line-numbers").checked = settings.lineNumbers;
  $<HTMLInputElement>("line-numbers-here").checked = settings.lineNumbersHere;
  $<HTMLInputElement>("highlight-line").checked = settings.highlightLine;
  $<HTMLInputElement>("highlight-line-here").checked = settings.highlightLineHere;
  $<HTMLInputElement>("preview-follows").checked = settings.previewFollows;
  $<HTMLInputElement>("preview-follows-here").checked = settings.previewFollowsHere;
  $<HTMLInputElement>("comment-highlights").checked = settings.commentHighlights;
  $<HTMLInputElement>("comment-highlights-here").checked = settings.commentHighlightsHere;
  tabs.setOptions({ lineNumbers: settings.lineNumbers, highlightLine: settings.highlightLine });
  tabs.setCommentOptions({ ...tabs.commentOptions, highlights: settings.commentHighlights });
  setBindings(bindingsFrom(settings.shortcuts));
  preview.setFollows(settings.previewFollows);
}

/** An on/off setting's two checkboxes: the setting itself, and "only on this device". */
function onOff(name: string, box: string, here: string) {
  const send = () => setting(name, $<HTMLInputElement>(box).checked ? "on" : "off", $<HTMLInputElement>(here).checked);
  $(box).addEventListener("change", send);
  $(here).addEventListener("change", send);
}
onOff("lineNumbers", "line-numbers", "line-numbers-here");
onOff("highlightLine", "highlight-line", "highlight-line-here");
onOff("previewFollows", "preview-follows", "preview-follows-here");
onOff("commentHighlights", "comment-highlights", "comment-highlights-here");

function setting(name: string, value: string, here: boolean) {
  act(async () => applySettings(await invoke<Settings>("set_setting", { name, value, here })));
}

$("theme").addEventListener("change", () => setting("theme", $<HTMLSelectElement>("theme").value, $<HTMLInputElement>("theme-here").checked));
$("theme-here").addEventListener("change", () => setting("theme", $<HTMLSelectElement>("theme").value, $<HTMLInputElement>("theme-here").checked));
$("font-size").addEventListener("change", () => setting("fontSize", $<HTMLInputElement>("font-size").value, $<HTMLInputElement>("font-size-here").checked));
$("font-size-here").addEventListener("change", () => setting("fontSize", $<HTMLInputElement>("font-size").value, $<HTMLInputElement>("font-size-here").checked));
// Afterwards the highlights take any new names and colours.
$("open-reviewers").addEventListener("click", () => act(async () => {
  await openReviewers(confirmAsk);
  tabs.refreshMarks();
}));

$("open-settings").addEventListener("click", () => act(async () => {
  applySettings(await invoke<Settings>("get_settings")); // another Device may have changed them
  await ask($<HTMLDialogElement>("settings"));
}));

$("open-shortcuts").addEventListener("click", () => act(async () => {
  const settings = await invoke<Settings>("get_settings");
  applySettings(settings);
  await openShortcuts(settings, { apply: applySettings, confirm: confirmAsk });
}));

// ---- Start ----

window.addEventListener("DOMContentLoaded", () => act(async () => {
  const { lastVault } = await invoke<{ lastVault: string | null }>("startup");
  if (lastVault) await openVault(lastVault);
}));
