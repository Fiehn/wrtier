// What every part of the window uses to talk to the Author: the error line, the info line, the
// Sync problems, and the small dialogs that ask a question.

/** An element of the page by its ID. */
export const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

/** A file the app couldn't take in (vault.rs `SyncProblem`). `newer`: made by a newer version of the app, so updating this Device is the fix. */
export type SyncProblem = { path: string; reason: string; newer?: boolean };

/** Opens the update dialog (set by main.ts once it exists). */
let openUpdates: () => Promise<unknown> = async () => {};

/** Where "Check for updates…" goes, for a problem only a newer version of the app can fix. */
export function onUpdatesAsked(open: () => Promise<unknown>) {
  openUpdates = open;
}

export function showError(e: unknown) {
  const el = $("error");
  el.textContent = String(e);
  // A Vault or Work set up by a newer version of the app (vault.rs): updating is the fix, so
  // it's offered right here as well as in Settings' App tab.
  if (String(e).includes("newer version of the app")) el.append(" ", updateButton());
  el.hidden = false;
}

/** "Check for updates…", for a problem only a newer version of the app can fix. */
function updateButton() {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "link";
  b.textContent = "Check for updates…";
  b.addEventListener("click", () => act(() => openUpdates()));
  return b;
}

export function clearError() {
  $("error").hidden = true;
}

/** Runs something the Author asked for, showing any error. */
export function act(run: () => Promise<unknown>) {
  clearError();
  run().catch(showError);
}

/** A line saying something went well (an Export), for a few seconds. */
let infoTimer: number | undefined;
export function showInfo(text: string) {
  const el = $("info");
  el.textContent = text;
  el.hidden = false;
  clearTimeout(infoTimer);
  infoTimer = window.setTimeout(() => (el.hidden = true), 10_000);
}

/** A line offering something, with a button that does it (and closes the line). */
export function offer(text: string, label: string, run: () => Promise<unknown>) {
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

/** "1 Sync problem", "3 Sync problems". */
export const problemsTitle = (n: number) => (n === 1 ? "1 Sync problem" : `${n} Sync problems`);

export function showProblems(problems: SyncProblem[]) {
  const el = $("problems");
  el.replaceChildren();
  el.hidden = problems.length === 0;
  if (problems.length === 0) return;
  const title = document.createElement("strong");
  title.textContent = problemsTitle(problems.length);
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

/**
 * Cancel buttons are plain buttons, not submit buttons: pressing Enter in a dialog submits with
 * its first submit button, which must be the one that says yes. Each closes its dialog with
 * its `data-close` value.
 */
export function wireCloseButtons() {
  for (const button of document.querySelectorAll<HTMLButtonElement>("dialog button[data-close]")) {
    button.addEventListener("click", () => button.closest("dialog")?.close(button.dataset.close));
  }
}

/** Shows a <dialog> and resolves with the button that closed it ("" if Escape). */
export function ask(dialog: HTMLDialogElement): Promise<string> {
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue), { once: true });
    dialog.returnValue = "";
    dialog.showModal();
  });
}

export const FOLDER_HINT = "Put it in a folder with a slash, e.g. Short stories/Rain.";

/**
 * Asks for a name (of a Work or folder); null if cancelled. An `initial` ending with a slash
 * (a folder to put it in) leaves the cursor after it; otherwise it's all selected, to replace.
 */
export async function askText(question: string, initial = "", ok = "OK", hint = FOLDER_HINT): Promise<string | null> {
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

export async function confirmAsk(text: string, yes: string): Promise<boolean> {
  $("ask-confirm-text").textContent = text;
  $("ask-confirm-yes").textContent = yes;
  return (await ask($<HTMLDialogElement>("ask-confirm"))) === "yes";
}

/** A small note under a list item. */
export function small(text: string) {
  const el = document.createElement("small");
  el.textContent = text;
  return el;
}

/** A link-like button that does something the Author asked for. */
export function actionButton(label: string, title: string, run: () => Promise<unknown>) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "link";
  b.textContent = label;
  b.title = title;
  b.addEventListener("click", () => act(run));
  return b;
}

/** This Device's choice, remembered in local storage (absent in a private window). */
export function deviceChoice(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function keepDeviceChoice(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // not remembered, then: no harm done
  }
}
