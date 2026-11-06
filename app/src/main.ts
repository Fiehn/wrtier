import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, dropCursor, keymap } from "@codemirror/view";
import { history, standardKeymap } from "@codemirror/commands";
import { WorkSync, type WorkState } from "./sync";

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
type OpenedWork = { id: string; name: string; text: string; problems: SyncProblem[]; notice: string | null };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

// ---- The editor ----

/** The Work in the editor, if any. */
let current: WorkSync | null = null;
const historyCompartment = new Compartment();

/** Ctrl+Z and redo: the sync decides between the editor's own undo (inside a draft) and the History's. */
const undoKeys = keymap.of([
  { key: "Mod-z", run: () => current?.undo() ?? true, preventDefault: true },
  { key: "Mod-Shift-z", run: () => current?.redo() ?? true, preventDefault: true },
  { key: "Mod-y", run: () => current?.redo() ?? true, preventDefault: true },
]);

function editorState(text: string, editable: boolean) {
  return EditorState.create({
    doc: text,
    extensions: [
      EditorView.lineWrapping,
      EditorState.readOnly.of(!editable),
      EditorView.editable.of(editable),
      historyCompartment.of(history()),
      undoKeys,
      // Only the basics: no shortcuts that move lines, which would bypass Moves.
      keymap.of(standardKeymap),
      dropCursor(),
      EditorView.updateListener.of((update) => {
        for (const tr of update.transactions) current?.track(tr);
      }),
    ],
  });
}

const editor = new EditorView({ parent: $("editor"), state: editorState("", false) });

/** The burst closed: its undo steps are gone (it's a Change now; Ctrl+Z reverts it). */
function clearUndoSteps() {
  editor.dispatch({ effects: historyCompartment.reconfigure([]) });
  editor.dispatch({ effects: historyCompartment.reconfigure(history()) });
}

function showWorkState(state: WorkState) {
  const el = $("status");
  const lines = [state.saveError, state.textFileProblem, state.recoveryProblem, state.syncProblem]
    .filter((line): line is string => line !== null);
  el.textContent = lines.join("\n");
  el.hidden = lines.length === 0;
  if (state.tookIn) {
    $("notice").textContent = state.tookIn;
    $("notice").hidden = false;
  }
}

// ---- Messages ----

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

// ---- The Vault ----

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
  await closeCurrent();
  let opened = await invoke<VaultOpened>("open_vault", { path, create: false });
  if (opened.status === "notAVault") {
    $("ask-create-path").textContent = path;
    if ((await ask($<HTMLDialogElement>("ask-create"))) !== "yes") return;
    opened = await invoke<VaultOpened>("open_vault", { path, create: true });
  }
  if (opened.status !== "opened") return;
  $("vault-root").textContent = opened.root;
  $("new-work").hidden = false;
  showWork(null);
  if (opened.deviceName === null) await askDeviceName(opened.suggestedName);
  await refreshList();
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

async function refreshList() {
  const listing = await invoke<Listing>("list_works");
  showProblems(listing.problems);
  const list = $("works");
  list.replaceChildren();
  for (const entry of listing.entries) {
    const item = document.createElement("li");
    item.className = entry.kind + (entry.id === current?.id ? " open" : "");
    const name = document.createElement("button");
    name.type = "button";
    name.textContent = entry.id;
    name.disabled = entry.kind === "newer" || entry.kind === "unreadable" || entry.kind === "leftover" || entry.kind === "blocked";
    name.addEventListener("click", () => openWork(entry.id));
    item.append(name);
    const note =
      entry.kind === "leftover" ? `Text file left over from ${entry.of}`
      : entry.kind === "blocked" ? entry.reason
      : labels[entry.kind];
    if (note) {
      const small = document.createElement("small");
      small.textContent = note;
      item.append(small);
    }
    if (entry.kind === "leftover") {
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "link";
      remove.textContent = "Delete it";
      remove.addEventListener("click", async () => {
        try {
          await invoke("delete_leftover", { id: entry.id });
          await refreshList();
        } catch (e) {
          showError(e);
        }
      });
      item.append(remove);
    }
    list.append(item);
  }
}

// ---- Works ----

function showWork(work: OpenedWork | null) {
  $("work-title").textContent = work?.name ?? "";
  $("empty").hidden = work !== null;
  $("editor").hidden = work === null;
  $("status").hidden = true;
  $("notice").textContent = work?.notice ?? "";
  $("notice").hidden = !work?.notice;
  current = work === null ? null : new WorkSync(work.id, editor, {
    state: showWorkState,
    burstClosed: clearUndoSteps,
    error: showError,
  });
  editor.setState(editorState(work?.text ?? "", work !== null));
  if (work !== null) editor.focus();
}

/** Closes the Work in the editor: everything typed reaches Rust, is saved, and the Text file written. */
async function closeCurrent() {
  if (current === null) return;
  const closing = current;
  await closing.idle();
  current = null;
  try {
    await invoke("close_work", { id: closing.id });
  } catch (e) {
    showError(e);
  }
}

async function openWork(id: string) {
  if (id === current?.id) return;
  clearError();
  try {
    await closeCurrent();
    const work = await invoke<OpenedWork>("open_work", { id });
    showWork(work);
    if (work.problems.length > 0) showProblems(work.problems);
    await refreshList(); // a Text file may have become a Work
  } catch (e) {
    showError(e);
  }
}

$("new-work").addEventListener("click", async () => {
  const input = $<HTMLInputElement>("new-work-name");
  input.value = "";
  if ((await ask($<HTMLDialogElement>("ask-new-work"))) !== "create") return;
  clearError();
  try {
    await closeCurrent();
    showWork(await invoke<OpenedWork>("create_work", { id: input.value }));
    await refreshList();
  } catch (e) {
    showWork(null);
    showError(e);
  }
});

// The Vault list follows what Syncthing brings (new Works, Conflict copies of Works that
// aren't open): every 10 seconds, and whenever the window gets focus.
let vaultOpen = false;
async function refreshQuietly() {
  if (!vaultOpen || document.querySelector("dialog[open]")) return;
  try {
    await refreshList();
  } catch (e) {
    showError(e);
  }
}
setInterval(refreshQuietly, 10_000);
window.addEventListener("focus", refreshQuietly);

// Leaving the window: the Text file is brought up to date for other programs.
window.addEventListener("blur", async () => {
  const work = current;
  if (work === null) return;
  await work.idle();
  try {
    showWorkState(await invoke<WorkState>("leave_work", { id: work.id }));
  } catch (e) {
    showError(e);
  }
});

// The Rust side's timer closed a burst or saved something.
void listen<{ id: string; state: WorkState }>("work-status", (event) => {
  if (event.payload.id === current?.id) current.status(event.payload.state);
});

$("choose-vault").addEventListener("click", async () => {
  try {
    const path = await invoke<string | null>("pick_folder");
    if (path) await openVault(path);
  } catch (e) {
    showError(e);
  }
});

window.addEventListener("DOMContentLoaded", async () => {
  showWork(null);
  try {
    const { lastVault } = await invoke<{ lastVault: string | null }>("startup");
    if (lastVault) await openVault(lastVault);
  } catch (e) {
    showError(e);
  }
});
