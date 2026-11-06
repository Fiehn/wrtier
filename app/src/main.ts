import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type WorkState } from "./sync";
import { Tabs, type Layout, type OpenedWork } from "./tabs";

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
type Listing = { entries: Entry[]; problems: SyncProblem[] };
type TrashedWork = { id: string; name: string };
type Settings = { theme: "system" | "light" | "dark"; fontSize: number; themeHere: boolean; fontSizeHere: boolean };

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

/** Asks for a name (of a Work); null if cancelled. */
async function askText(question: string, initial = "", ok = "OK"): Promise<string | null> {
  $("ask-text-question").textContent = question;
  $("ask-text-ok").textContent = ok;
  const input = $<HTMLInputElement>("ask-text-input");
  input.value = initial;
  input.select();
  return (await ask($<HTMLDialogElement>("ask-text"))) === "ok" ? input.value : null;
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

const tabs = new Tabs($("tabbar"), $("panes"), { notice: $("notice"), status: $("status"), moved: $("moved") }, {
  changed: () => {
    $("empty").hidden = tabs.active !== null;
    $("view-mode").hidden = tabs.active === null;
    rememberSoon();
    highlightOpen();
  },
  error: showError,
  askName: (question, initial) => askText(question, initial),
});

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

async function openWork(id: string, cursor?: number) {
  if (tabs.has(id)) return tabs.activate(id);
  const work = await invoke<OpenedWork>("open_work", { id });
  await tabs.add(work, cursor);
  if (work.problems.length > 0) showProblems(work.problems);
  await refreshList(); // a Text file may have become a Work
}

// The Rust side's timer closed a burst, saved, or noticed a change from outside.
void listen<{ id: string; state: WorkState }>("work-status", (event) => tabs.status(event.payload.id, event.payload.state));

// ---- The Vault ----

let vaultOpen = false;
let entries: Entry[] = [];

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
  $("vault-actions").hidden = false;
  $("open-settings").hidden = false;
  if (opened.deviceName === null) await askDeviceName(opened.suggestedName);
  applySettings(await invoke<Settings>("get_settings"));
  await refreshList();
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

async function refreshList() {
  const listing = await invoke<Listing>("list_works");
  entries = listing.entries;
  showProblems(listing.problems);
  const list = $("works");
  list.replaceChildren();
  for (const entry of entries) {
    const item = document.createElement("li");
    item.className = entry.kind;
    item.dataset.id = entry.id;
    const name = document.createElement("button");
    name.type = "button";
    name.className = "entry-name";
    name.textContent = entry.id;
    name.disabled = entry.kind === "newer" || entry.kind === "unreadable" || entry.kind === "leftover" || entry.kind === "blocked";
    name.addEventListener("click", () => act(() => openWork(entry.id)));
    item.append(name);
    const note =
      entry.kind === "leftover" ? `Text file left over from ${entry.of}`
      : entry.kind === "blocked" ? entry.reason
      : labels[entry.kind];
    if (note) item.append(small(note));
    const actions = document.createElement("span");
    actions.className = "entry-actions";
    if (entry.kind === "work") {
      actions.append(
        actionButton("Rename", `Rename ${entry.id}`, () => renameWork(entry.id)),
        actionButton("Trash", `Move ${entry.id} to the trash`, () => trashWork(entry.id)),
      );
    } else if (entry.kind === "leftover") {
      actions.append(actionButton("Delete it", "Delete the leftover Text file", async () => {
        await invoke("delete_leftover", { id: entry.id });
        await refreshList();
      }));
    }
    item.append(actions);
    list.append(item);
  }
  highlightOpen();
  await refreshTrash();
}

function highlightOpen() {
  for (const item of $("works").querySelectorAll<HTMLElement>("li")) {
    item.classList.toggle("open", item.dataset.id === tabs.active);
  }
}

async function newWork(name?: string) {
  const id = name ?? (await askText("What's the new Work called?", "", "Create"));
  if (id === null) return;
  await tabs.add(await invoke<OpenedWork>("create_work", { id }));
  await refreshList();
}

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

$("new-work").addEventListener("click", () => act(() => newWork()));

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

// App-wide keys: Ctrl+O finds a Work, Ctrl+W closes the tab in front.
window.addEventListener("keydown", (e) => {
  const mod = e.ctrlKey || e.metaKey;
  if (mod && !e.shiftKey && e.key.toLowerCase() === "o") {
    e.preventDefault();
    void openSwitcher();
  } else if (mod && !e.shiftKey && e.key.toLowerCase() === "w" && tabs.active) {
    e.preventDefault();
    act(() => tabs.close(tabs.active!));
  }
}, { capture: true });

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
window.addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "e" && tabs.active) {
    e.preventDefault();
    setFormatted(!tabs.formatted);
  }
}, { capture: true });

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
}

function setting(name: string, value: string, here: boolean) {
  act(async () => applySettings(await invoke<Settings>("set_setting", { name, value, here })));
}

$("theme").addEventListener("change", () => setting("theme", $<HTMLSelectElement>("theme").value, $<HTMLInputElement>("theme-here").checked));
$("theme-here").addEventListener("change", () => setting("theme", $<HTMLSelectElement>("theme").value, $<HTMLInputElement>("theme-here").checked));
$("font-size").addEventListener("change", () => setting("fontSize", $<HTMLInputElement>("font-size").value, $<HTMLInputElement>("font-size-here").checked));
$("font-size-here").addEventListener("change", () => setting("fontSize", $<HTMLInputElement>("font-size").value, $<HTMLInputElement>("font-size-here").checked));
$("open-settings").addEventListener("click", () => act(async () => {
  applySettings(await invoke<Settings>("get_settings")); // another Device may have changed them
  await ask($<HTMLDialogElement>("settings"));
}));

// ---- Start ----

window.addEventListener("DOMContentLoaded", () => act(async () => {
  const { lastVault } = await invoke<{ lastVault: string | null }>("startup");
  if (lastVault) await openVault(lastVault);
}));
