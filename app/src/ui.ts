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

/**
 * A click outside an open dialog (on its backdrop) closes it as Esc does: the dialog's "cancel"
 * comes first, so one that's busy (installing an update…) stays open. Only a click that both
 * starts and ends outside counts: selecting text in a field and letting go outside doesn't.
 */
export function closeDialogsOnClickOutside() {
  const outside = (e: MouseEvent): HTMLDialogElement | null => {
    const dialog = e.target;
    if (!(dialog instanceof HTMLDialogElement) || !dialog.open) return null;
    // On the dialog itself but outside its box: the backdrop (a modal dialog's backdrop is its own).
    const r = dialog.getBoundingClientRect();
    const inBox = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
    return inBox ? null : dialog;
  };
  let pressed: HTMLDialogElement | null = null;
  document.addEventListener("pointerdown", (e) => (pressed = outside(e)), true);
  document.addEventListener("click", (e) => {
    const dialog = outside(e);
    const started = pressed;
    pressed = null;
    if (!dialog || dialog !== started) return;
    if (dialog.dispatchEvent(new Event("cancel", { cancelable: true }))) dialog.close();
  }, true);
}

/** Shows a <dialog> and resolves with the button that closed it ("" if Escape or a click outside). */
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

/** A symbol drawn as Lucide's are (index.html has the fixed ones): its paths' `d`, on a 24×24 grid. */
export function symbol(paths: string[], size = 16): SVGSVGElement {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  for (const [name, value] of Object.entries({
    viewBox: "0 0 24 24", width: String(size), height: String(size), fill: "none", stroke: "currentColor",
    "stroke-width": "1.75", "stroke-linecap": "round", "stroke-linejoin": "round", "aria-hidden": "true",
  })) svg.setAttribute(name, value);
  for (const d of paths) {
    const path = document.createElementNS(ns, "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}

/** Lucide's symbols for what buttons made in code do, the same one for the same act everywhere
 *  (lines and circles written as paths). */
export const SYMBOLS = {
  plus: ["M5 12h14", "M12 5v14"],
  restore: ["M9 14 4 9l5-5", "M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"], // undo-2
  delete: ["M3 6h18", "M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6", "M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2", "M10 11v6", "M14 11v6"], // trash-2
  edit: ["M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z", "m15 5 4 4"], // pencil
  goTo: ["M2 12h3", "M19 12h3", "M12 2v3", "M12 19v3", "M5 12a7 7 0 1 0 14 0a7 7 0 1 0-14 0"], // locate
  reply: ["M9 17 4 12l5-5", "M20 18v-2a4 4 0 0 0-4-4H4"],
  resolve: ["M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0", "m9 12 2 2 4-4"], // circle-check
  insertImage: ["M16 5h6", "M19 2v6", "M21 11.5V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7.5", "m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21", "M7 9a2 2 0 1 0 4 0a2 2 0 1 0-4 0"], // image-plus
  replaceImage: ["M10.3 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v10l-3.1-3.1a2 2 0 0 0-2.814.014L6 21", "m14 19.5 3-3 3 3", "M17 22v-5.5", "M7 9a2 2 0 1 0 4 0a2 2 0 1 0-4 0"], // image-up
  openFolder: ["m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"], // folder-open
  openElsewhere: ["M15 3h6v6", "M10 14 21 3", "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"], // external-link
  removePerson: ["M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2", "M5 7a4 4 0 1 0 8 0a4 4 0 1 0-8 0", "M22 11h-6"], // user-minus
  show: ["M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0", "M9 12a3 3 0 1 0 6 0a3 3 0 1 0-6 0"], // eye
  takeOff: ["M18 6 6 18", "m6 6 12 12"], // x
  marks: ["m6 16 6-12 6 12", "M8 12h8", "m16 20 2 2 4-4"], // spell-check
};

/** A button that shows only a symbol, named in its tooltip and to screen readers; `count` beside
 *  it, if any. A click does only `run`: not what a click on what it's in would. */
export function symbolButton(name: string, paths: string[], run: () => void, title = name, count?: number): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "symbol-button";
  b.title = title;
  b.setAttribute("aria-label", name);
  b.append(symbol(paths));
  if (count !== undefined) {
    const c = document.createElement("span");
    c.className = "symbol-count";
    c.textContent = String(count);
    b.append(c);
  }
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    run();
  });
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
