import { invoke } from "@tauri-apps/api/core";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

// The same shapes as the Rust side's (vault.rs, lib.rs).
type VaultOpened =
  | { status: "notAVault" }
  | { status: "opened"; root: string; deviceName: string | null; suggestedName: string };

type EntryKind =
  | { kind: "work" }
  | { kind: "newWork" }
  | { kind: "leftover"; of: string }
  | { kind: "newer"; version: number }
  | { kind: "unreadable"; reason: string };

type Entry = { id: string; name: string } & EntryKind;
type SyncProblem = { path: string; reason: string };
type Listing = { entries: Entry[]; problems: SyncProblem[] };
type OpenedWork = { id: string; name: string; text: string; problems: SyncProblem[] };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

// Step 1 only shows the text; typing comes with the History connection (step 2).
const editor = new EditorView({
  parent: $("editor"),
  state: EditorState.create({ doc: "", extensions: [EditorView.lineWrapping, EditorState.readOnly.of(true)] }),
});

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

/** Shows a <dialog> and resolves with the button that closed it ("" if Escape). */
function ask(dialog: HTMLDialogElement): Promise<string> {
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue), { once: true });
    dialog.returnValue = "";
    dialog.showModal();
  });
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
  let opened = await invoke<VaultOpened>("open_vault", { path, create: false });
  if (opened.status === "notAVault") {
    $("ask-create-path").textContent = path;
    if ((await ask($<HTMLDialogElement>("ask-create"))) !== "yes") return;
    opened = await invoke<VaultOpened>("open_vault", { path, create: true });
  }
  if (opened.status !== "opened") return;
  $("vault-root").textContent = opened.root;
  showWork(null);
  if (opened.deviceName === null) await askDeviceName(opened.suggestedName);
  await refreshList();
}

const labels: Record<Entry["kind"], string> = {
  work: "",
  newWork: "Text file only: opening it starts its History",
  leftover: "",
  newer: "Made by a newer version of the app",
  unreadable: "Can't be read right now",
};

async function refreshList() {
  const listing = await invoke<Listing>("list_works");
  showProblems(listing.problems);
  const list = $("works");
  list.replaceChildren();
  for (const entry of listing.entries) {
    const item = document.createElement("li");
    item.className = entry.kind;
    const name = document.createElement("button");
    name.type = "button";
    name.textContent = entry.id;
    name.disabled = entry.kind === "newer" || entry.kind === "unreadable" || entry.kind === "leftover";
    name.addEventListener("click", () => openWork(entry.id));
    item.append(name);
    const note = entry.kind === "leftover" ? `Text file left over from ${entry.of}` : labels[entry.kind];
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

function showWork(work: OpenedWork | null) {
  $("work-title").textContent = work?.name ?? "";
  $("empty").hidden = work !== null;
  $("editor").hidden = work === null;
  editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: work?.text ?? "" } });
}

async function openWork(id: string) {
  clearError();
  try {
    const work = await invoke<OpenedWork>("open_work", { id });
    showWork(work);
    if (work.problems.length > 0) showProblems(work.problems);
    await refreshList(); // a Text file may have become a Work
  } catch (e) {
    showError(e);
  }
}

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
