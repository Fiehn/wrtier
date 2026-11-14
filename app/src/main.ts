import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type WorkState } from "./sync";
import { Tabs, type Layout, type OpenedWork } from "./tabs";
import { buildTree, folderOf, foldersOf, hiddenFolderAt, withoutHidden, type Folder } from "./tree";
import { destinations, movedFolders, whereTo, type Movable, type Taken } from "./vaultmoves";
import { showMenu, type MenuItem } from "./menu";
import { PagePreview, type Milestone } from "./preview";
import { LibraryPanel, type LibraryItem } from "./library";
import { libraryId, libraryPath } from "./libraryfile";
import { PreambleDialog, type Preamble } from "./preambledialog";
import { colourOf, COLOURS, openReviewers, type Reviewer } from "./reviewers";
import { SidePanel, type PanelTab } from "./sidepanel";
import { CommentsPanel } from "./commentspanel";
import { HistoryPanel, type ComparisonView, type HistoryView, type RestorePreview, type ReturnPreview, type RevertPreview, type SnippetsView } from "./historypanel";
import { bindingsFrom, COMMANDS, isCommand, keyLabel, onBindingsChanged, setBindings, withKey, type CommandId } from "./keys";
import { openShortcuts } from "./shortcutsdialog";
import { openLicenses } from "./licensesdialog";
import { browseForVault, ensureFilesAccess, onAndroid } from "./android";
import { PhoneLayout } from "./phone";
import { PanelSizes } from "./panelsizes";
import { fillNewWorkMods, fillVaultMods, openMods } from "./modsdialog";
import { inUse, modNamed, vaultModInUse, type ModsState } from "./mods";
import { ChapterList } from "./chapterlist";
import { CountLine } from "./countline";
import { endWorkDrag, startWorkDrag, type ChapterChoice } from "./inclusions";
import { ThemeApplier, themeChoices, VAULT_PREFIX } from "./themes";
import { SyncPanel } from "./syncpanel";
import { UpdateDialog } from "./updates";
import { UninstallDialog } from "./uninstall";
import { FindBar } from "./findbar";
import { FindInVault, type TextForFinding, type VaultWork } from "./findinvault";

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
/** `newer`: made by a newer version of the app, so updating this Device is the fix. */
type SyncProblem = { path: string; reason: string; newer?: boolean };
/** `hidden`: the Hidden folders on this Device (vault/hidden.rs), ones gone from the Vault too. */
type Listing = { entries: Entry[]; problems: SyncProblem[]; folders: string[]; hidden: string[]; colours: Colours };
/** The colours of Works (by ID) and folders (by path), each a Reviewer colour slot (vault/colours.rs). */
type Colours = { works: Record<string, string>; folders: Record<string, string> };
type TrashedWork = { id: string; name: string };
type Settings = {
  /** "system", "light", "dark", or "vault:<file>" for a theme in the Vault's `themes/`. */
  theme: string;
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
  /** This Device keeps its own list of Hidden folders. */
  hiddenFoldersHere: boolean;
};

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

// ---- Messages and dialogs ----

function showError(e: unknown) {
  const el = $("error");
  el.textContent = String(e);
  // A Vault or Work set up by a newer version of the app (vault.rs): updating is the fix, and
  // with the Vault shut, Settings can't be reached.
  if (String(e).includes("newer version of the app")) el.append(" ", updateButton());
  el.hidden = false;
}

/** "Check for updates…", for a problem only a newer version of the app can fix. */
function updateButton() {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "link";
  b.textContent = "Check for updates…";
  b.addEventListener("click", () => act(() => updates.open()));
  return b;
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

/** A line offering something, with a button that does it (and closes the line). */
function offer(text: string, label: string, run: () => Promise<unknown>) {
  const el = $("info");
  const button = document.createElement("button");
  button.type = "button";
  button.className = "link";
  button.textContent = label;
  button.addEventListener("click", () => {
    el.hidden = true;
    act(run);
  });
  el.replaceChildren(`${text} `, button);
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
  if (problems.some((p) => p.newer)) {
    const line = document.createElement("p");
    line.append("Another device has a newer version of the app. ", updateButton());
    el.append(line);
  }
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

/** How the Vault syncs through Syncthing: the line at the top of the sidebar and its dialog. */
const syncPanel = new SyncPanel((message) => showError(message), (text, yes) => confirmAsk(text, yes));

/** Runs something the Author asked for, showing any error. */
function act(run: () => Promise<unknown>) {
  clearError();
  run().catch(showError);
}

// ---- Find and replace (in the tab in front) ----

const findBar = new FindBar({
  target: () => tabs.frontEditor(),
  openWork: (work, at) => act(async () => {
    await openWork(work, at);
    await tabs.reveal(work, at, at); // also when it was open already
  }),
});
$("find-open").addEventListener("click", () => findBar.open(false));

// ---- Tabs ----

const tabs = new Tabs($("tabbar"), $("panes"), { notice: $("notice"), status: $("status"), moved: $("moved"), style: $("style-notice") }, {
  changed: () => {
    $("empty").hidden = tabs.active !== null;
    // A library file is plain Typst: no Formatted view, and no pages of its own.
    $("view-mode").hidden = !tabs.activeIsWork;
    $("panel-toggle").hidden = !tabs.activeIsWork;
    $("find-open").hidden = tabs.active === null;
    findBar.attach();
    showToolbar();
    preview.show(tabs.activeIsWork ? tabs.active : null);
    commentsPanel.render();
    historyPanel.workChanged();
    showModViews();
    rememberSoon();
    highlightOpen();
  },
  error: showError,
  askName: (question, initial) => askText(question, initial),
  edited: (id) => {
    preview.edited(id);
    chapterList.edited(id);
    countLine.edited(id);
    findInVault.edited(id);
  },
  committed: (id) => preview.committed(id),
  cursor: (id, at) => {
    preview.cursor(id);
    chapterList.cursor(id, at);
    countLine.selected(id);
  },
  closed: (id) => {
    preview.forget(id);
    modsOf.delete(id);
  },
  addElement: (id, name) => act(() => addElement(id, name)),
  openComments: (_id, ids) => {
    openTab("comments");
    commentsPanel.focus(ids);
  },
  marks: (id) => {
    if (id === tabs.active) commentsPanel.render();
  },
  addComment: (id) => startComment(id),
  selectionMenu: (id, x, y) => showMenu(x, y, selectionMenu(id)),
  historyGrew: (id) => historyPanel.grew(id),
  textHistory: (id) => textHistory(id),
  notices: (count) => {
    $("panel-tab-history").textContent = count > 0 ? `History (${count})` : "History";
  },
  modCommand: (id, command) => modCommand(id, command),
  dropWork: (id, work, pos) => act(() => includeChecked(id, work, null, pos)),
  inclusionStyles: (id, at, styles, x, y) => {
    const all = library.items.filter((i) => i.kind === "style");
    if (all.length === 0) {
      showError("The Style library has no Styles yet.");
      return;
    }
    const set = (next: string[]) => act(async () => {
      await tabs.settled(id);
      await invoke("set_inclusion_styles", { id, at, styles: next });
      tabs.refreshInclusionsOf(id);
    });
    showMenu(x, y, [
      ...all.map((s): MenuItem => {
        const on = styles.includes(s.name);
        return { label: `${on ? "✓ " : "    "}${s.title}`, run: () => set(on ? styles.filter((n) => n !== s.name) : [...styles, s.name]) };
      }),
      ...(styles.length > 0 ? ["separator" as const, { label: "No Styles of its own", run: () => set([]) }] : []),
    ]);
  },
  openWork: (id, at) => act(async () => {
    await openWork(id, at);
    await tabs.reveal(id, at, at); // also when it was open already
  }),
});

// ---- The side panel (Ctrl+Shift+P): the Page preview and the Comments ----

const preview = new PagePreview($("preview-bar"), $("preview-pages"), $<HTMLSelectElement>("preview-look"), $<HTMLSelectElement>("preview-version"), $<HTMLSelectElement>("preview-zoom"), {
  settled: (id) => tabs.settled(id),
  reveal: (id, from, to) => {
    phone.toWrite();
    void tabs.reveal(id, from, to);
  },
  cursorOf: (id) => tabs.cursorOf(id),
  looksOf: async (id) => (await invoke<{ preamble: Preamble }>("get_preamble", { id })).preamble.looks,
  milestonesOf: (id) => invoke<Milestone[]>("get_milestones", { id }),
  error: showError,
});

// ---- The History ----

const historyPanel = new HistoryPanel($("history-list"), {
  work: () => (tabs.activeIsWork ? tabs.active : null),
  load: (work, limit, filtered) => invoke<HistoryView>("get_history", { id: work, limit, filtered }),
  snippets: (work, changes, filtered) => invoke<SnippetsView>("get_snippets", { id: work, changes, filtered }),
  compare: (work, from, to) => invoke<ComparisonView>("compare_versions", { id: work, from, to }),
  previewRevert: (work, changes) => invoke<RevertPreview>("preview_revert", { id: work, changes }),
  previewRestore: (work, changes) => invoke<RestorePreview>("preview_restore", { id: work, changes }),
  previewReturn: (work, milestone) => invoke<ReturnPreview>("preview_return", { id: work, milestone }),
  act: (work, op) => {
    clearError();
    tabs.history(work, op);
  },
  locate: (work, first, last) => {
    phone.toWrite();
    return tabs.locateText(work, first, last);
  },
  nameMilestone: (work) => nameMilestone(work),
  renameMilestone: async (work, milestone, name) => {
    tabs.status(work, await invoke<WorkState>("rename_milestone", { id: work, milestone, name }));
    await preview.milestonesChanged(work);
  },
  setMilestoneDeleted: async (work, milestone, deleted) => {
    tabs.status(work, await invoke<WorkState>("set_milestone_deleted", { id: work, milestone, deleted }));
    await preview.milestonesChanged(work);
  },
  dismissNotice: async (work, key) => {
    tabs.status(work, await invoke<WorkState>("dismiss_notice", { id: work, key }));
  },
  openPreamble: (work) => act(() => editPreamble(work)),
  error: showError,
});
$("history-name-milestone").addEventListener("click", () => historyPanel.nameMilestone());
$("history-compare").addEventListener("click", () => historyPanel.startCompare());

// "Colour text by Session": on or off on this Device (off at first). While it's off, nothing
// is worked out for it.
const tintSwitch = $<HTMLInputElement>("history-tint");
try {
  tintSwitch.checked = localStorage.getItem("session-colours") === "on";
} catch {
  // not remembered: off
}
tabs.setTinting(tintSwitch.checked);
tintSwitch.addEventListener("change", () => {
  tabs.setTinting(tintSwitch.checked);
  try {
    localStorage.setItem("session-colours", tintSwitch.checked ? "on" : "off");
  } catch {
    // not remembered, then: no harm done
  }
});

/** Its key (Ctrl+Shift+H at first) or "History of this text": the History tab narrows to the
 *  selection, or to the paragraph the cursor is in. */
function textHistory(id: string) {
  act(async () => {
    const quote = await tabs.stretch(id);
    if (quote === null) throw new Error("Select some text, or put the cursor in a paragraph, to see its History.");
    openTab("history");
    historyPanel.showTextHistory(id, quote);
  });
}

// Shown or not, and which tab, are remembered on this Device. The pages only compile while
// their tab is showing, and the History only loads while its tab is.
const panel = new SidePanel(
  $("side-panel"),
  [$("panel-tab-pages"), $("panel-tab-comments"), $("panel-tab-history")],
  { pages: $("preview"), comments: $("comments-pane"), history: $("history-pane") },
  $("panel-toggle"),
  (pagesInView, inView) => {
    preview.setOpen(pagesInView);
    historyPanel.setVisible(inView === "history");
  },
);

// On a phone (or any narrow window): one screen at a time, with a bar at the bottom.
const phone = new PhoneLayout(panel);

/** Opens one of the panel's tabs: on the phone, that tab's screen. */
function openTab(tab: PanelTab) {
  if (phone.active) phone.show(tab);
  else panel.setTab(tab);
}

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
  includedMarks: (work) => tabs.includedMarksOf(work),
  // A Comment may belong to a Work that isn't open (one shown on an Inclusion): then it's
  // changed without opening a tab, and the Inclusions in front follow.
  act: (work, op) => {
    if (tabs.has(work)) {
      tabs.comment(work, op);
      return;
    }
    act(async () => {
      await invoke("comment_in", { work, ops: [op] });
      if (tabs.active) tabs.refreshInclusionsOf(tabs.active);
    });
  },
  addIncluded: async (work, target, sender, body) => {
    await invoke("comment_on_inclusion", { id: work, at: target.at, from: target.from, to: target.to, sender, body });
    tabs.refreshInclusionsOf(work);
  },
  show: (work, comment, part) => {
    phone.toWrite();
    if (tabs.has(work)) return tabs.showComment(work, comment, part);
    // One on an Inclusion: its own Work opens, at it.
    act(async () => {
      await openWork(work);
      tabs.showComment(work, comment, part);
    });
    return 1;
  },
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
function startComment(id: string, included = tabs.includedSelectionOf(id)) {
  // Text selected in an Inclusion: the Comment goes to the Work it includes (ADR 0015).
  if (included) {
    openTab("comments");
    act(() => commentsPanel.startAddingIncluded(id, included));
    return;
  }
  const target = tabs.commentTarget(id);
  if (!target) {
    showError("Select some text to comment on, or put the cursor in a word.");
    return;
  }
  openTab("comments");
  act(() => commentsPanel.startAdding(id, target));
}

/** Right-click inside a selection: Add Comment, and the editing the system menu would have. */
function selectionMenu(id: string): MenuItem[] {
  const view = tabs.viewOf(id);
  if (!view) return [];
  // In an Inclusion's text, which can't be changed here: Add Comment and Copy. (Read now: the
  // selection may not survive clicking the menu.)
  const included = tabs.includedSelectionOf(id);
  if (included) {
    return [
      { label: withKey("Add Comment", "addComment"), run: () => startComment(id, included) },
      { label: "Copy", run: () => act(() => navigator.clipboard.writeText(included.quote)) },
    ];
  }
  // Cut and Copy go through the editor, so a cut pasted back is still a Move.
  const command = (name: "cut" | "copy") => () => {
    view.focus();
    document.execCommand(name);
  };
  return [
    { label: withKey("Add Comment", "addComment"), run: () => startComment(id) },
    { label: withKey("History of this text", "textHistory"), run: () => textHistory(id) },
    { label: withKey("Find and replace…", "find"), run: () => findBar.open(false) },
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
  menu: (item, x, y) => libraryMenu(item, x, y),
  error: showError,
});

const KIND_NAMES: Record<LibraryItem["kind"], string> = { style: "Style", look: "Look", storyElement: "Story element" };

/** Right-click on a Style, Look or Story element: open, rename or remove it. */
function libraryMenu(item: LibraryItem, x: number, y: number) {
  const run = (f: () => Promise<unknown>) => () => act(f);
  const items: MenuItem[] = [
    { label: "Open", run: run(() => openLibraryFile(item.path, item.title)) },
    "separator",
    { label: "Rename…", run: run(() => renameLibraryItem(item)) },
  ];
  // The built-in Story elements are part of every Work, so they stay.
  if (!(item.kind === "storyElement" && library.builtIn.includes(item.name))) {
    items.push({ label: "Remove…", run: run(() => removeLibraryItem(item)) });
  }
  showMenu(x, y, items);
}

/**
 * Renaming changes the name shown (the title in the file's first line). The file keeps its
 * name, which is what Works and Looks find it by, so nothing using it loses it.
 */
async function renameLibraryItem(item: LibraryItem) {
  const what = KIND_NAMES[item.kind];
  const hint = item.kind === "storyElement"
    ? `It's still written as #${item.name} in the text; only the name shown changes.`
    : `Only the name shown changes: its file stays ${item.path.split("/").pop()}, so the Works and Looks using it still find it.`;
  const title = await askText(`Rename the ${what} ${item.title} to:`, item.title, "Rename", hint);
  if (title === null || title.trim() === item.title) return;
  await tabs.settled(libraryId(item.path)); // anything typed in its tab is saved first
  await invoke("retitle_library", { path: item.path, title });
  await library.refresh();
  tabs.libraryRetitled(item.path, library.titleOf(item.path));
  await tabs.libraryChanged();
}

/** Removing moves the file to the Vault's trash folder; Works that recorded it keep their copy. */
async function removeLibraryItem(item: LibraryItem) {
  const what = KIND_NAMES[item.kind];
  const lines = [`Remove the ${what} ${item.title} from the Style library?`];
  if (item.kind === "style") {
    const looks = library.items.filter((i) => i.kind === "look" && i.styles.includes(item.name)).map((i) => i.title);
    if (looks.length) lines.push(`${looks.length === 1 ? "The Look" : "The Looks"} ${looks.join(", ")} ${looks.length === 1 ? "uses" : "use"} it and will say it's missing.`);
  }
  lines.push(`Works that use it keep the copy they recorded of it, so their pages don't change. The file goes to the Vault's .trash folder.`);
  if (!(await confirmAsk(lines.join(" "), "Remove"))) return;
  await tabs.close(libraryId(item.path)); // saved first, then closed
  await invoke("trash_library", { path: item.path });
  await library.refresh();
  await refreshElements();
}

async function openLibraryFile(path: string, title: string = library.titleOf(path)): Promise<void> {
  const text = await invoke<string>("read_library", { path });
  await tabs.addLibrary({ path, title, text });
}

// The library changed: in a tab here, in another editor, or from another Device. The Page
// preview uses the library's versions, so it's compiled again too.
void listen("library-changed", () => {
  library.refresh().then(() => refreshElements()).then(() => findInVault.libraryChanged()).catch(showError);
  void tabs.libraryChanged();
  void preview.preambleChanged(tabs.activeIsWork ? tabs.active : null);
});

// ---- A Work's Mods ----

/** Each open Work's Mods, as last asked (its own, and which this Device hides). */
const modsOf = new Map<string, ModsState>();

/** Is this Mod at work in this Work on this Device? */
const modInUse = (id: string, mod: string) => inUse(modsOf.get(id) ?? { work: [], newWorks: [], vault: [], hidden: [] }).some((m) => m.id === mod);

function setModsOf(id: string, state: ModsState) {
  modsOf.set(id, state);
  tabs.setMods(id, inUse(state));
  if (id === tabs.active) showModViews();
}

/** Tells each open Work's editor (or just `only`) which Mods are in use in it on this Device. */
async function refreshMods(only?: string) {
  for (const id of tabs.works) {
    if (only && id !== only) continue;
    try {
      setModsOf(id, await invoke<ModsState>("get_mods", { id }));
    } catch {
      // renamed or gone meanwhile: its tab says so, and the next refresh finds it
    }
  }
}

async function editMods(id: string) {
  await openMods(id, id.split("/").pop() ?? id, (state) => setModsOf(id, state));
  await refreshMods(); // hiding a Mod on this Device changes every Work that has it
}

/**
 * A Mod's command's key, pressed while writing in a Work: if the Work has the Mod (and this
 * Device doesn't hide it), it runs; otherwise a line offers to add it, or to show it here.
 */
function modCommand(id: string, command: CommandId): boolean {
  const mod = modNamed(COMMANDS.find((c) => c.id === command)?.mod ?? "");
  if (!mod) return false;
  if (modInUse(id, mod.id)) {
    switch (command) {
      case "nextChapter": return chapterList.step(id, 1);
      case "previousChapter": return chapterList.step(id, -1);
      case "chapterUp": return chapterList.shift(id, -1);
      case "chapterDown": return chapterList.shift(id, 1);
      default: return false;
    }
  }
  const name = id.split("/").pop() ?? id;
  if (modsOf.get(id)?.work.includes(mod.id)) {
    offer(`The ${mod.name} is hidden on this device.`, "Show it", async () => {
      await invoke("set_mod_hidden", { name: mod.id, hidden: false });
      await refreshMods();
    });
  } else {
    offer(`The ${mod.name} isn't on ${name}.`, "Add it", async () => {
      setModsOf(id, await invoke<ModsState>("set_mod", { id, name: mod.id, on: true }));
    });
  }
  return true;
}

// ---- The Chapter list (a Mod) ----

const chapterList = new ChapterList($("chapters"), $("chapters-list"), $("chapters-none"), {
  textOf: (id) => tabs.textOf(id),
  cursorOf: (id) => tabs.cursorOf(id),
  includedOf: (id) => tabs.includedOf(id),
  goTo: (id, at) => {
    phone.toWrite();
    void tabs.reveal(id, at, at);
  },
  move: (id, moves, cursor) => tabs.moveText(id, moves, cursor),
});

// ---- The word and character count (a Mod) ----

const countLine = new CountLine($("counts"), {
  textOf: (id) => tabs.readerTextOf(id),
  selectedText: (id) => tabs.readerTextOf(id, true),
});

/** The Mods that show the Work in front (the Chapter list, the counts) follow it. */
function showModViews() {
  const id = tabs.activeIsWork ? tabs.active : null;
  chapterList.show(id && modInUse(id, "chapters") ? id : null);
  countLine.show(id && modInUse(id, "counts") ? id : null);
}

// ---- Inclusions (ADR 0015) ----

/**
 * Includes the Work `work` at the cursor in the Work `into`: the whole of it, or (asked with a
 * menu at `at` if it has Chapters) one of its Chapters.
 */
async function includeIn(into: string, work: string, at: { x: number; y: number }) {
  // Refused before the menu, if it would make a Work include itself.
  await invoke("check_inclusion", { id: into, work });
  const chapters = await invoke<ChapterChoice[]>("chapters_of", { work });
  if (chapters.length === 0) {
    tabs.include(into, work, null);
    return;
  }
  showMenu(at.x, at.y, [
    { label: "The whole Work", run: () => tabs.include(into, work, null) },
    "separator",
    ...chapters.map((c): MenuItem => ({ label: `Chapter: ${c.title}`, run: () => tabs.include(into, work, c.at) })),
  ]);
}

/**
 * Includes `work` (or its Chapter at `chapter`) in the Work `into`, at the cursor or near `near`,
 * once Rust has said it can be: not if that would make a Work include itself.
 */
async function includeChecked(into: string, work: string, chapter: number | null, near?: number) {
  await invoke("check_inclusion", { id: into, work });
  tabs.include(into, work, chapter, near);
}

// ---- Collections: Works selected in the Vault browser (Ctrl+click, or "Select" in its menu) ----

/** The Works selected for a new Collection, in the order they were picked. */
let picked: string[] = [];

function togglePicked(id: string) {
  picked = picked.includes(id) ? picked.filter((p) => p !== id) : [...picked, id];
  showPicked();
}

/** Marks the selected Works in the Vault browser, numbered in the order they'll be in. */
function showPicked() {
  picked = picked.filter((id) => entries.some((e) => e.id === id && e.kind === "work"));
  for (const item of $("works").querySelectorAll<HTMLElement>("li.work-entry")) {
    const n = picked.indexOf(item.dataset.id ?? "");
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
async function newCollection(works: string[], folder?: string) {
  const id = await askText(`A new Collection of ${works.length === 1 ? "1 Work" : `${works.length} Works`}. What's it called?`, folder ? `${folder}/` : "", "Create",
    `It includes ${works.map((w) => w.split("/").pop()).join(", ")}, in that order. ${FOLDER_HINT}`);
  if (id === null) return;
  const created = await invoke<OpenedWork>("create_collection", { id, works });
  picked = [];
  await tabs.add(created);
  await refreshElements(created.id);
  await refreshList();
}

// The Work in front follows the Works it includes as they're written (in other tabs, or arriving
// from other Devices).
window.setInterval(() => tabs.refreshInclusions(), 2000);

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
/**
 * Before an Export: if some of the Work's Inclusions can't be shown (the Work they include is in
 * the trash, not synced yet…), says which and asks. True to go ahead (they become a line
 * saying so in the Export).
 */
async function inclusionsReady(id: string): Promise<boolean> {
  const missing = await invoke<string[]>("missing_inclusions", { id });
  if (missing.length === 0) return true;
  const shown = missing.slice(0, 3).join(" ");
  const more = missing.length > 3 ? ` And ${missing.length - 3} more.` : "";
  const what = missing.length === 1 ? "One Inclusion can't be shown" : `${missing.length} Inclusions can't be shown`;
  return confirmAsk(`${what}: ${shown}${more} The Export would have a line saying so in ${missing.length === 1 ? "its" : "their"} place.`, "Export anyway");
}

/** `milestone`: export the Work as it was then (what the Page preview shows), not as it is now. */
async function exportPdf(id: string, milestone: { id: string; name: string } | null = null) {
  await tabs.settled(id);
  if (!milestone && !(await inclusionsReady(id))) return;
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
  const done = await invoke<Exported | null>("export_pdf", { id, look, milestone: milestone?.id ?? null });
  if (!done) return; // cancelled
  const pages = done.pages === 1 ? "1 page" : `${done.pages} pages`;
  const fonts = done.deviceFonts.length
    ? ` ${done.deviceFonts.join(", ")} came from this Device, not the Vault, so a PDF from another Device would differ.`
    : "";
  const what = milestone ? ` “${milestone.name}”` : "";
  showInfo(`Exported${what} ${pages} to ${done.path}.${fonts}`);
}

$("preview-pdf").addEventListener("click", () => act(async () => {
  if (preview.shown) await exportPdf(preview.shown, preview.shownMilestone);
}));

/** Exports a Work as a manuscript .docx, and says what it couldn't carry. */
async function exportDocx(id: string, milestone: { id: string; name: string } | null = null) {
  await tabs.settled(id);
  if (!milestone && !(await inclusionsReady(id))) return;
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
  const done = await invoke<Exported | null>("export_docx", { id, font: select.value, milestone: milestone?.id ?? null });
  if (!done) return; // cancelled
  let line = `Exported${milestone ? ` “${milestone.name}”` : ""} ${done.paragraphs} paragraphs to ${done.path}.`;
  if (done.leftOut.length > 0) {
    const shown = done.leftOut.slice(0, 3).map((l) => `line ${l.line}: ${l.what}`).join("; ");
    const more = done.leftOut.length > 3 ? `, and ${done.leftOut.length - 3} more` : "";
    line += ` Left out (a .docx can't carry it): ${shown}${more}.`;
  }
  showInfo(line);
}

$("preview-docx").addEventListener("click", () => act(async () => {
  if (preview.shown) await exportDocx(preview.shown, preview.shownMilestone);
}));

/** Names the current point in a Work's History, to come back to or compare with later. */
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

async function openWork(id: string, cursor?: number, restoring = false) {
  if (!restoring) phone.show("write");
  const path = libraryPath(id);
  if (path) return openLibraryFile(path);
  if (tabs.has(id)) return tabs.activate(id);
  const work = await invoke<OpenedWork>("open_work", { id });
  await tabs.add(work, cursor);
  await refreshElements(work.id);
  await refreshMods(work.id);
  if (work.problems.length > 0) showProblems(work.problems);
  await refreshList(); // a Text file may have become a Work
}

// The Rust side's timer closed a burst, saved, or noticed a change from outside.
void listen<{ id: string; state: WorkState }>("work-status", (event) => tabs.status(event.payload.id, event.payload.state));

// ---- The Vault ----

let vaultOpen = false;
let entries: Entry[] = [];
let folders: string[] = [];
/** The Hidden folders on this Device: they and everything in them are left out of the Vault browser. */
let hiddenFolders: string[] = [];
let colours: Colours = { works: {}, folders: {} };
/** The Vault's Mods as last read: which Vault Mods it has, and which this Device hides. */
let vaultMods: ModsState | null = null;

/** Is the Colours Vault Mod at work here? Its colours are kept either way. */
const coloursOn = () => vaultMods !== null && vaultModInUse(vaultMods, "colours");

/** A Work's or folder's colour as CSS, if it has one and Colours is on. */
function shownColour(slot: string | undefined): string | null {
  return slot && coloursOn() ? colourOf(slot) : null;
}

/** Gives a list item its colour (on its name; for a folder, also its chevron and line). */
function paint(item: HTMLElement, slot: string | undefined) {
  const css = shownColour(slot);
  if (!css) return;
  item.classList.add("coloured");
  item.style.setProperty("--item-colour", css);
}

/** Colour… on a Work or folder: a row of the colours, "no colour" first, at (x, y). */
function pickColour(x: number, y: number, current: string | undefined, save: (colour: string) => Promise<unknown>) {
  const choose = (colour: string) => () => act(async () => {
    await save(colour);
    await refreshList();
    void findInVault.refresh();
  });
  showMenu(x, y, [{
    swatches: [
      { label: "No colour", css: null, chosen: !current, run: choose("") },
      ...COLOURS.map((c) => ({ label: c[0].toUpperCase() + c.slice(1), css: colourOf(c), chosen: current === c, run: choose(c) })),
    ],
  }]);
}

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
  findInVault.reset();
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
  syncPanel.start();
  if (opened.deviceName === null) await askDeviceName(opened.suggestedName);
  applySettings(await invoke<Settings>("get_settings"));
  await refreshList();
  $("library").hidden = false;
  await library.refresh();
  // The tabs that were open here last time, each with its cursor where it was.
  const layout = await invoke<Layout>("get_layout");
  for (const id of layout.tabs) {
    try {
      await openWork(id, layout.cursors[id], true);
    } catch {
      // renamed or gone since: just not reopened
    }
  }
  if (layout.active && tabs.has(layout.active)) await tabs.activate(layout.active);
  // On the phone: writing, if a Work was open last time.
  phone.show(tabs.active ? "write" : "works");
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
  paint(item, colours.works[entry.id]);
  const pill = document.createElement("div");
  pill.className = "entry";
  const name = document.createElement("button");
  name.type = "button";
  name.className = "entry-name";
  name.textContent = entry.name;
  name.title = entry.id;
  name.disabled = disabled(entry);
  name.addEventListener("click", (e) => {
    // Ctrl+click (⌘ on a Mac) picks it for a new Collection instead.
    if ((e.ctrlKey || e.metaKey) && entry.kind === "work") togglePicked(entry.id);
    else act(() => openWork(entry.id));
  });
  pill.append(name);
  if (entry.kind === "work") {
    // Dragged into a Work's text, it's included there; onto a folder, it moves there.
    pill.draggable = true;
    pill.addEventListener("dragstart", (e) => {
      startWorkDrag(e, entry.id);
      startMove({ kind: "work", id: entry.id });
    });
    pill.addEventListener("dragend", () => {
      endWorkDrag();
      endMove();
    });
  }
  const actions = document.createElement("span");
  actions.className = "entry-actions";
  if (entry.kind === "work") {
    actions.append(
      actionButton("Preamble…", withKey(`Looks, Styles, Story elements and settings of ${entry.id}`, "preamble"), () => editPreamble(entry.id)),
      actionButton("Mods…", withKey(`Optional features of ${entry.id}, such as spellcheck`, "mods"), () => editMods(entry.id)),
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
  paint(item, colours.folders[folder.path]);
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
    startMove({ kind: "folder", path: folder.path });
  });
  item.addEventListener("dragend", (e) => {
    if (e.target === item) endMove();
  });
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

/** The tree the Vault browser shows: everything but what's in Hidden folders. */
function shownTree(): Folder<Entry> {
  const shown = withoutHidden(entries, folders, hiddenFolders);
  return buildTree(shown.items, shown.folders);
}

async function refreshList() {
  const listing = await invoke<Listing>("list_works");
  entries = listing.entries;
  folders = listing.folders;
  hiddenFolders = listing.hidden ?? [];
  colours = listing.colours ?? { works: {}, folders: {} };
  vaultMods = await invoke<ModsState>("get_mods", { id: null });
  showProblems(listing.problems);
  const list = $("works");
  list.replaceChildren();
  fillList(list, shownTree());
  showPicked();
  highlightOpen();
  await refreshTrash();
}

/**
 * Marks the Work in front, and opens the folders it's in so it can be seen. One in a Hidden
 * folder can't be: a line above the list says where it is instead.
 */
function highlightOpen() {
  const active = tabs.active;
  const shutIn = active && tabs.activeIsWork ? hiddenFolderAt(folderOf(active), hiddenFolders) : null;
  showHiddenNote(active, shutIn);
  if (active && !shutIn) {
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

/** "Rain v1 is in a hidden folder (Old drafts) · Unhide Old drafts", or nothing. */
function showHiddenNote(active: string | null, shutIn: string | null) {
  const note = $("hidden-note");
  note.hidden = !active || !shutIn;
  if (!active || !shutIn) return;
  const text = document.createElement("span");
  text.textContent = `${active.split("/").pop()} is in a hidden folder (${shutIn}).`;
  note.replaceChildren(text, document.createTextNode(" "),
    actionButton(`Unhide ${shutIn.split("/").pop()}`, `Show ${shutIn} in the Vault browser again, on every device`, () => setFolderHidden(shutIn, false)));
}

/** Hides a folder in the Vault browser (on every Device, unless this one keeps its own list), or shows it again. */
async function setFolderHidden(path: string, hidden: boolean) {
  await invoke<string[]>("set_folder_hidden", { path, hidden });
  await refreshList();
  void findInVault.refresh(); // its Works are searched again, or no longer
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
  await moveFolder(path, name); // folders that were open stay open under their new name
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
// (Only in the Vault's part of the sidebar: the Chapters below aren't Works.)
$("sidebar-vault").addEventListener("contextmenu", (e) => {
  if (!vaultOpen) return;
  e.preventDefault();
  const target = e.target as HTMLElement;
  const workEl = target.closest<HTMLElement>("li.work-entry");
  const folderEl = target.closest<HTMLElement>("li.folder");
  const run = (f: () => Promise<unknown>) => () => act(f);
  const folderHere = workEl ? foldersOf(workEl.dataset.id ?? "").pop() : folderEl?.dataset.path;
  // With Works selected (Ctrl+click, or "Select" here): a Collection of them.
  const collecting: MenuItem[] = picked.length === 0 ? [] : [
    "separator",
    { label: `New Collection from selected Works (${picked.length})…`, run: run(() => newCollection([...picked], folderHere)) },
    { label: "Clear the selection", run: () => { picked = []; showPicked(); } },
  ];
  let items: MenuItem[];
  if (workEl) {
    const entry = entries.find((x) => x.id === workEl.dataset.id);
    if (!entry) return;
    const folder = foldersOf(entry.id).pop();
    // Included at the cursor in the Work in front (not in itself).
    const into = tabs.activeIsWork && tabs.active !== entry.id ? tabs.active : null;
    const at = { x: e.clientX, y: e.clientY };
    items =
      entry.kind === "work" ? [
        { label: "Open", run: run(() => openWork(entry.id)) },
        ...(into ? [{ label: `Include in ${into.split("/").pop()}…`, run: run(() => includeIn(into, entry.id, at)) }] : []),
        {
          label: picked.includes(entry.id) ? "Deselect" : "Select for a Collection",
          run: () => togglePicked(entry.id),
        },
        { label: "Preamble…", run: run(() => editPreamble(entry.id)) },
        { label: "Mods…", run: run(() => editMods(entry.id)) },
        ...(coloursOn() ? [{ label: "Colour…", run: () => pickColour(at.x, at.y, colours.works[entry.id], (colour) => invoke("set_work_colour", { id: entry.id, colour })) }] : []),
        { label: "Name a Milestone…", run: run(() => nameMilestone(entry.id)) },
        { label: "Export PDF…", run: run(() => exportPdf(entry.id)) },
        { label: "Export .docx…", run: run(() => exportDocx(entry.id)) },
        "separator",
        { label: "Rename…", run: run(() => renameWork(entry.id)) },
        ...moveToItem({ kind: "work", id: entry.id }, at),
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
      ...moveToItem({ kind: "folder", path }, { x: e.clientX, y: e.clientY }),
      { label: "Hide folder", run: run(() => setFolderHidden(path, true)) },
      ...(coloursOn() ? [{ label: "Colour…", run: () => pickColour(e.clientX, e.clientY, colours.folders[path], (colour) => invoke("set_folder_colour", { path, colour })) }] : []),
      { label: "Move folder to trash", run: run(() => trashFolder(path)) },
    ];
  } else {
    items = [
      { label: "New Work…", run: run(() => newWork()) },
      { label: "New folder…", run: run(() => newFolder()) },
    ];
  }
  showMenu(e.clientX, e.clientY, items.length > 0 ? [...items, ...collecting] : collecting.slice(1));
});

async function renameWork(id: string) {
  const name = await askText(`Rename ${id} to:`, id, "Rename");
  if (name === null || name === id) return;
  await moveWork(id, name);
  await refreshList();
}

/** Renames a Work, maybe into another folder: its tab, and its place in a Collection being picked, go along. */
async function moveWork(id: string, name: string): Promise<string> {
  const renamed = await invoke<string>("rename_work", { id, name });
  tabs.renamed(id, renamed);
  picked = picked.map((p) => (p === id ? renamed : p));
  return renamed;
}

/** Renames a folder, maybe into another: the tabs of Works in it, and the folders open in it, go along. */
async function moveFolder(path: string, name: string): Promise<string> {
  const moved = await invoke<[string, string][]>("rename_folder", { path, name });
  for (const [from, to] of moved) tabs.renamed(from, to);
  const renamed = name.split("/").map((p) => p.trim()).join("/");
  openFolders = new Set(movedFolders(openFolders, path, renamed));
  saveOpenFolders();
  picked = picked.map((p) => (p.startsWith(`${path}/`) ? renamed + p.slice(path.length) : p));
  return renamed;
}

// ---- Moving Works and folders: dragged onto a folder in the Vault browser, or Move to… ----

/** What a folder dragged in the Vault browser carries: its path. */
const FOLDER_DRAG = "application/x-wrtier-folder";

/**
 * What's being dragged to move, while it is: kept a moment past the drag's end, since WebKitGTK
 * reports the end before the drop (see dragging.ts).
 */
let moving: Movable | null = null;
let forgetMoving: number | undefined;
/** A closed folder dragged over opens after a moment, to reach the folders in it. */
let springOpen: { path: string; timer: number } | null = null;
/** Why the move can't go where it's being dragged, as last said (said once, not on every move of the mouse). */
let refusedSaid: string | null = null;

function startMove(item: Movable) {
  window.clearTimeout(forgetMoving);
  moving = item;
}

function endMove() {
  window.clearTimeout(forgetMoving);
  forgetMoving = window.setTimeout(() => (moving = null), 1000);
  showDropTarget(null);
  springOpenSoon(null);
  refusedSaid = null;
}

const taken = (): Taken => ({ works: entries.map((e) => e.id), folders });

/**
 * The folder a drop at `target` goes into: a folder's row (or anything inside it) is that
 * folder, a Work's row is its folder, and the space around the list is the top of the Vault.
 */
function dropFolder(target: EventTarget | null): string {
  const el = target instanceof Element ? target : null;
  const work = el?.closest<HTMLElement>("#works li.work-entry");
  if (work) return folderOf(work.dataset.id ?? "");
  return el?.closest<HTMLElement>("#works li.folder")?.dataset.path ?? "";
}

/** Shades where a drop would go: a folder, or (for "") the whole list. */
function showDropTarget(into: string | null) {
  for (const el of document.querySelectorAll(".drop-target")) el.classList.remove("drop-target");
  if (into === null) return;
  const el = into === "" ? $("works") : document.querySelector(`#works li.folder[data-path="${CSS.escape(into)}"]`);
  el?.classList.add("drop-target");
}

/** Opens a closed folder whose row has been dragged over for a moment. */
function springOpenSoon(target: EventTarget | null) {
  const row = target instanceof Element ? target.closest<HTMLElement>("#works .folder-toggle") : null;
  const folderEl = row?.parentElement ?? null;
  const path = folderEl && !folderEl.classList.contains("expanded") ? folderEl.dataset.path ?? null : null;
  if (springOpen?.path === path) return;
  if (springOpen) window.clearTimeout(springOpen.timer);
  springOpen = path === null ? null : {
    path,
    timer: window.setTimeout(() => {
      springOpen = null;
      if (!folderEl!.classList.contains("expanded")) row!.click();
    }, 750),
  };
}

const vaultSide = $("sidebar-vault");
vaultSide.addEventListener("dragover", (e) => {
  if (!moving || vaultSide.classList.contains("finding")) return;
  const into = dropFolder(e.target);
  const result = whereTo(moving, into, taken());
  springOpenSoon(e.target);
  e.preventDefault(); // it's ours, wherever it is over the Vault browser
  // Where it can't go, it can't be dropped (the cursor says so), and the info line says why.
  if (e.dataTransfer) e.dataTransfer.dropEffect = "to" in result ? "move" : "none";
  showDropTarget("to" in result ? into : null);
  if ("refused" in result && refusedSaid !== result.refused) showInfo(result.refused);
  refusedSaid = "refused" in result ? result.refused : null;
});
vaultSide.addEventListener("dragleave", (e) => {
  // Out of the Vault browser altogether (not just from one row to the next).
  if (!(e.relatedTarget instanceof Node && vaultSide.contains(e.relatedTarget))) {
    showDropTarget(null);
    springOpenSoon(null);
  }
});
vaultSide.addEventListener("drop", (e) => {
  const item = moving;
  if (!item || vaultSide.classList.contains("finding")) return;
  e.preventDefault();
  moving = null;
  showDropTarget(null);
  springOpenSoon(null);
  refusedSaid = null;
  const result = whereTo(item, dropFolder(e.target), taken());
  if ("to" in result) act(() => moveTo(item, result.to));
});

/** Moves a Work or folder to `to`, then shows where it went: its folder opened, its row flashing. */
async function moveTo(item: Movable, to: string) {
  const moved = item.kind === "work" ? await moveWork(item.id, to) : await moveFolder(item.path, to);
  foldersOf(moved).forEach((f) => openFolders.add(f));
  saveOpenFolders();
  await refreshList();
  void findInVault.refresh();
  const row = item.kind === "work"
    ? document.querySelector(`#works li.work-entry[data-id="${CSS.escape(moved)}"]`)
    : document.querySelector(`#works li.folder[data-path="${CSS.escape(moved)}"]`);
  if (!row) return;
  row.scrollIntoView({ block: "nearest" });
  row.classList.add("just-moved");
  window.setTimeout(() => row.classList.remove("just-moved"), 1600);
}

/** Move to…: the folders it can go to (no Hidden ones), "Top of the Vault" first. None if there are none. */
function moveToItem(item: Movable, at: { x: number; y: number }): MenuItem[] {
  const choices = destinations(item, withoutHidden(entries, folders, hiddenFolders).folders);
  if (choices.length === 0) return [];
  return [{
    label: "Move to…",
    run: () => showMenu(at.x, at.y, choices.map((f) => ({
      label: f === "" ? "Top of the Vault" : f,
      run: () => act(async () => {
        const result = whereTo(item, f, taken());
        if ("refused" in result) showInfo(result.refused);
        else if ("to" in result) await moveTo(item, result.to);
      }),
    }))),
  }];
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
  // Android's own picker gives no file path, so the app has its own folder browser there.
  if (onAndroid && !(await ensureFilesAccess())) return;
  const path = onAndroid ? await browseForVault() : await invoke<string | null>("pick_folder");
  if (path) await openVault(path);
}));


// Going to the background on Android (another app, the screen off): Android may end the app
// there without warning, so everything typed goes to disk at once (the open burst closes, a cut
// waiting for its paste becomes a deletion) and Syncthing can send it. Nothing compiles meanwhile.
// Coming back, the Vault is checked for changes from outside at once.
document.addEventListener("visibilitychange", () => {
  const hidden = document.visibilityState === "hidden";
  preview.setOpen(!hidden && panel.pagesInView);
  if (!vaultOpen) return;
  if (hidden) {
    if (!onAndroid) return;
    act(async () => {
      await tabs.idle();
      for (const { id, state } of await invoke<{ id: string; state: WorkState }[]>("app_hidden")) tabs.status(id, state);
    });
  } else {
    void refreshQuietly();
  }
});

// The Vault list follows what Syncthing brings (new Works, Conflict copies of Works that
// aren't open): every 10 seconds, and whenever the window gets focus. The tabs and cursors are
// remembered then too.
async function refreshQuietly() {
  if (!vaultOpen || document.querySelector("dialog[open]")) return;
  try {
    await refreshList();
    await refreshMods(); // another Device may have changed them
    void findInVault.refresh(); // Works synced in, renamed or trashed: its list catches up
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

// ---- Find in Vault (Ctrl+Shift+F): every Work's text, in place of the Works list ----

/** The Works to find in, in the Works list's order (folders first, as it shows them). */
function vaultWorks(): VaultWork[] {
  const out: VaultWork[] = [];
  const walk = (folder: Folder<Entry>) => {
    for (const sub of folder.folders) walk(sub);
    for (const e of folder.items) {
      if (e.kind === "work" || e.kind === "newWork") out.push({ id: e.id, name: e.name, problem: null, colour: shownColour(colours.works[e.id]) });
      else if (e.kind === "newer") out.push({ id: e.id, name: e.name, problem: "made by a newer version of the app" });
      else if (e.kind === "unreadable") out.push({ id: e.id, name: e.name, problem: e.reason });
      // A leftover or blocked Text file isn't a Work.
    }
  };
  walk(shownTree());
  return out;
}

/** Shows a match found: its Work in a passing tab if it isn't open, the match selected. */
async function showFound(id: string, from: number, to: number, focus: boolean) {
  if (!tabs.has(id)) {
    const work = await invoke<OpenedWork>("open_work", { id });
    await tabs.addPassing(work, focus);
    await refreshElements(work.id);
    await refreshMods(work.id);
    if (work.problems.length > 0) showProblems(work.problems);
    if (entries.some((e) => e.id === id && e.kind === "newWork")) await refreshList(); // it has a History now
  }
  await tabs.reveal(id, from, to, focus);
  if (focus) phone.toWrite();
}

const findInVault = new FindInVault({
  works: vaultWorks,
  hiddenFolders: () => withoutHidden(entries, folders, hiddenFolders).shut.length,
  openText: (id) => tabs.docOf(id),
  read: (id, known) => invoke<TextForFinding>("text_for_finding", { id, known }),
  elements: () => library.items.filter((i) => i.kind === "storyElement").map((i) => ({ name: i.name, title: i.title, shownAs: i.shownAs ?? "block", inWork: true })),
  show: showFound,
  openWithFind: async (id, query, match) => {
    await openWork(id);
    tabs.keep(id);
    if (match) await tabs.reveal(id, match.from, match.to);
    findBar.open(true, query);
  },
  shown: (open) => $("sidebar-vault").classList.toggle("finding", open),
  error: showError,
});

/** Ctrl+Shift+F, or the button: the Vault browser shows (the Works screen on the phone), finding. */
function openFindInVault() {
  if (!vaultOpen) return;
  setSidebar(true);
  phone.show("works");
  // A selection within one line in the editor is what to find, as with Ctrl+F.
  const front = tabs.frontEditor();
  let text: string | undefined;
  if (front?.view.hasFocus) {
    const { state } = front.view;
    const s = state.selection.main;
    if (!s.empty && state.doc.lineAt(s.from).number === state.doc.lineAt(s.to).number) text = state.sliceDoc(s.from, s.to);
  }
  findInVault.show(text);
}
$("open-find-in-vault").addEventListener("click", openFindInVault);

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

// ---- The panels' widths: dragged by their edges, remembered on this Device ----

new PanelSizes({
  vault: {
    element: $("sidebar"),
    handle: $("sidebar-resizer"),
    shown: () => !$("app").classList.contains("sidebar-hidden"),
    setShown: setSidebar,
  },
  side: {
    element: $("side-panel"),
    handle: $("panel-resizer"),
    shown: () => panel.shown,
    setShown: (shown) => panel.setShown(shown),
  },
});


// ---- Formatted and Raw view (Ctrl+E), remembered on this Device ----

function setFormatted(on: boolean) {
  tabs.setFormatted(on);
  $("view-mode").textContent = on ? "Raw view" : "Formatted view";
  $("toolbar-view").textContent = on ? "Raw" : "Formatted";
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

// ---- The phone's toolbar under the text: each button does what its key does ----

const toolbar = $("write-toolbar");
for (const button of toolbar.querySelectorAll<HTMLButtonElement>("button")) {
  // The editor keeps the focus, so the keyboard stays open.
  button.addEventListener("pointerdown", (e) => e.preventDefault());
  button.addEventListener("mousedown", (e) => e.preventDefault());
}
for (const button of toolbar.querySelectorAll<HTMLButtonElement>("button[data-command]")) {
  button.addEventListener("click", () => tabs.runCommand(button.dataset.command as CommandId));
}
$("toolbar-view").addEventListener("click", () => setFormatted(!tabs.formatted));

/** Shown with a tab in front; Comments, the History and the views are a Work's only. */
function showToolbar() {
  toolbar.hidden = tabs.active === null;
  for (const id of ["addComment", "textHistory"]) {
    toolbar.querySelector<HTMLButtonElement>(`[data-command="${id}"]`)!.disabled = !tabs.activeIsWork;
  }
  $<HTMLButtonElement>("toolbar-view").disabled = !tabs.activeIsWork;
}

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
  mods: () => {
    if (!tabs.activeIsWork || !tabs.active) return false;
    act(() => editMods(tabs.active!));
    return true;
  },
  // Always the app's own, never the web engine's page search (which only sees the lines drawn).
  find: () => {
    findBar.open(false);
    return true;
  },
  replace: () => {
    findBar.open(true);
    return true;
  },
  findInVault: () => {
    if (!vaultOpen) return false;
    openFindInVault();
    return true;
  },
  // Only while the Page preview has the focus: in the editor these keys do nothing.
  zoomIn: () => inPages(() => preview.zoom.zoomIn()),
  zoomInAlt: () => inPages(() => preview.zoom.zoomIn()),
  zoomOut: () => inPages(() => preview.zoom.zoomOut()),
  zoomReset: () => inPages(() => preview.zoom.reset()),
};

/** Runs a Page preview command if the focus is in the Page preview (its pages or its header). */
function inPages(run: () => void): boolean {
  if (!$("preview").contains(document.activeElement)) return false;
  run();
  return true;
}

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
  $("open-switcher").title = withKey("Open a Work", "quickSwitcher");
  $("find-open").title = withKey("Find and replace", "find");
  findBar.showKeys();
  $("open-switcher-key").textContent = keyLabel("quickSwitcher");
  $("open-switcher-key").hidden = !keyLabel("quickSwitcher");
  $("open-find-in-vault").title = withKey("Find some text in every Work", "findInVault");
  $("find-in-vault-key").textContent = keyLabel("findInVault");
  $("find-in-vault-key").hidden = !keyLabel("findInVault");
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

/** The theme setting as last applied, so a change to the Vault's themes can apply it again. */
let themeSetting = "system";

/** Shows the theme picker's choices and lines for the theme setting as it was last applied. */
function showThemes() {
  const { list, resolved } = themes;
  const select = $<HTMLSelectElement>("theme");
  select.replaceChildren();
  const vaultGroup = document.createElement("optgroup");
  vaultGroup.label = "This Vault's themes";
  for (const choice of themeChoices(list, resolved.missing)) {
    const option = new Option(choice.label, choice.value);
    (choice.group === "vault" ? vaultGroup : select).append(option);
  }
  if (vaultGroup.children.length) select.append(vaultGroup);
  select.value = themeSetting;
  const missing = $("theme-missing");
  missing.hidden = !resolved.missing;
  missing.textContent = resolved.missing
    ? `The theme ${resolved.missing} isn't in this Vault's themes folder, so the app follows the system. Choose another theme, or import it again.`
    : "";
  const problems = $("theme-problems");
  problems.hidden = list.problems.length === 0;
  problems.textContent = list.problems.map((p) => `${p.file} in the themes folder isn't a theme: ${p.reason}`).join("\n");
}

const themes = new ThemeApplier(showThemes);

function applySettings(settings: Settings) {
  const root = document.documentElement;
  themeSetting = settings.theme;
  void themes.apply(settings.theme);
  root.style.setProperty("--editor-font-size", `${settings.fontSize}px`);
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
  $<HTMLInputElement>("hidden-folders-here").checked = settings.hiddenFoldersHere;
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
// Importing copies a theme's file into the Vault's themes folder, then uses it.
$("import-theme").addEventListener("click", () => act(async () => {
  type Imported = { status: "imported"; file: string; name: string } | { status: "exists"; name: string };
  const path = await invoke<string | null>("pick_theme_file");
  if (path === null) return;
  let done = await invoke<Imported>("import_theme", { path, replace: false });
  if (done.status === "exists") {
    if (!(await confirmAsk(`This Vault already has a theme called “${done.name}”. Replace it with the one you picked?`, "Replace"))) return;
    done = await invoke<Imported>("import_theme", { path, replace: true });
  }
  if (done.status !== "imported") return;
  setting("theme", VAULT_PREFIX + done.file, $<HTMLInputElement>("theme-here").checked);
}));

// A theme was added, changed or removed (here, in another editor, or from another Device).
void listen("themes-changed", () => void themes.apply(themeSetting));

$("font-size").addEventListener("change", () => setting("fontSize", $<HTMLInputElement>("font-size").value, $<HTMLInputElement>("font-size-here").checked));
$("font-size-here").addEventListener("change", () => setting("fontSize", $<HTMLInputElement>("font-size").value, $<HTMLInputElement>("font-size-here").checked));
// Afterwards the highlights take any new names and colours.
$("open-reviewers").addEventListener("click", () => act(async () => {
  await openReviewers(confirmAsk);
  tabs.refreshMarks();
}));

/** Settings' list of Hidden folders, each with Unhide (or Remove, for one that's gone from the Vault). */
function fillHiddenFolders() {
  const list = $("hidden-folders");
  list.replaceChildren();
  if (hiddenFolders.length === 0) {
    const none = document.createElement("li");
    none.className = "muted";
    none.textContent = "None. Right-click a folder in the Vault browser to hide it.";
    list.append(none);
  }
  for (const path of hiddenFolders) {
    const item = document.createElement("li");
    const name = document.createElement("span");
    name.textContent = path;
    item.append(name);
    if (folders.includes(path)) {
      item.append(actionButton("Unhide", `Show ${path} in the Vault browser again`, async () => {
        await setFolderHidden(path, false);
        fillHiddenFolders();
      }));
    } else {
      item.append(small("not in the Vault now"), actionButton("Remove", `Take ${path} off the list`, async () => {
        await setFolderHidden(path, false);
        fillHiddenFolders();
      }));
    }
    list.append(item);
  }
}

$("hidden-folders-here").addEventListener("change", () => act(async () => {
  applySettings(await invoke<Settings>("set_hidden_folders_here", { here: $<HTMLInputElement>("hidden-folders-here").checked }));
  await refreshList();
  void findInVault.refresh();
  fillHiddenFolders();
}));

$("open-settings").addEventListener("click", () => act(async () => {
  applySettings(await invoke<Settings>("get_settings")); // another Device may have changed them
  await refreshList(); // and the Hidden folders with them
  fillHiddenFolders();
  await fillVaultMods($("vault-mods"), (state) => {
    vaultMods = state;
    act(async () => {
      await refreshList(); // the colours come or go
      void findInVault.refresh();
    });
  });
  await fillNewWorkMods($("new-work-mods"));
  await ask($<HTMLDialogElement>("settings"));
}));

$("open-shortcuts").addEventListener("click", () => act(async () => {
  const settings = await invoke<Settings>("get_settings");
  applySettings(settings);
  await openShortcuts(settings, { apply: applySettings, confirm: confirmAsk });
}));

$("open-licenses").addEventListener("click", () => act(openLicenses));

// ---- App updates (ADR 0014) ----

/** Everything typed goes to disk before an update replaces the app. */
async function saveEverything() {
  await tabs.idle();
  if (!vaultOpen) return;
  for (const { id, state } of await invoke<{ id: string; state: WorkState }[]>("app_hidden")) tabs.status(id, state);
}

const updates = new UpdateDialog(saveEverything);
$("check-updates").addEventListener("click", () => act(() => updates.open()));
const uninstaller = new UninstallDialog(saveEverything);
$("open-uninstall").addEventListener("click", () => act(() => uninstaller.open()));
invoke<{ version: string; canUninstall: boolean }>("app_info").then(
  (info) => {
    $("app-version").textContent = `Wrtier ${info.version}`;
    $("open-uninstall").hidden = !info.canUninstall;
  },
  () => {},
);

// ---- Start ----

window.addEventListener("DOMContentLoaded", () => act(async () => {
  const { lastVault } = await invoke<{ lastVault: string | null }>("startup");
  // On Android the access can be turned off in Settings while the app is closed.
  if (lastVault && (await ensureFilesAccess())) await openVault(lastVault);
}));
