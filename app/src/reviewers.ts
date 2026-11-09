// The Reviewers dialog: the Vault's Reviewers, each with a name, a role label and a colour slot.
// Every change is saved at once (to vault.db); removing one only hides them as a sender.

import { invoke } from "@tauri-apps/api/core";

export type Reviewer = {
  id: string;
  name: string;
  role: string;
  /** A colour slot: the theme defines --reviewer-<colour> and --reviewer-<colour>-soft. */
  colour: string;
  removed: boolean;
};

/** The colour slots, in the order new Reviewers get them (vault/reviewers.rs). */
export const COLOURS = ["teal", "amber", "red", "violet", "green", "orange", "pink", "brown"];

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

/** A Reviewer's colour as CSS. */
export function colourOf(colour: string, soft = false): string {
  return `var(--reviewer-${colour}${soft ? "-soft" : ""})`;
}

function swatch(colour: string): HTMLElement {
  const dot = document.createElement("span");
  dot.className = "swatch";
  dot.style.background = colourOf(colour);
  return dot;
}

function capital(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/**
 * Opens the dialog. `confirm` asks a yes/no question (for bringing back a removed Reviewer who
 * has the name being added).
 */
export async function openReviewers(confirm: (text: string, yes: string) => Promise<boolean>): Promise<void> {
  const dialog = $<HTMLDialogElement>("reviewers");
  const error = $("reviewers-error");
  const showError = (e: unknown) => {
    error.textContent = String(e);
    error.hidden = false;
  };
  const run = (job: () => Promise<unknown>) => {
    error.hidden = true;
    job().catch(showError);
  };

  async function update(r: Reviewer, change: Partial<Reviewer>) {
    const next = { ...r, ...change };
    await invoke<Reviewer>("update_reviewer", { id: r.id, name: next.name, role: next.role, colour: next.colour });
  }

  function row(r: Reviewer): HTMLLIElement {
    const li = document.createElement("li");
    const fields = document.createElement("div");
    fields.className = "row";
    const name = document.createElement("input");
    name.value = r.name;
    name.maxLength = 100;
    name.setAttribute("aria-label", "Name");
    const role = document.createElement("input");
    role.value = r.role;
    role.maxLength = 60;
    role.placeholder = "Role";
    role.setAttribute("aria-label", "Role");
    // Saved when the box is left (or Enter is pressed); a refused name goes back to what it was.
    name.addEventListener("change", () => run(async () => {
      try { await update(r, { name: name.value }); } finally { await render(); }
    }));
    role.addEventListener("change", () => run(async () => {
      try { await update(r, { role: role.value }); } finally { await render(); }
    }));
    // Enter saves the box (by leaving it) instead of closing the dialog.
    for (const box of [name, role]) {
      box.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          box.blur();
        }
      });
    }
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "link";
    remove.textContent = "Remove";
    remove.title = "No longer offer them when adding a Comment. Their Comments stay as they are.";
    remove.addEventListener("click", () => run(async () => {
      await invoke("set_reviewer_removed", { id: r.id, removed: true });
      await render();
    }));
    fields.append(swatch(r.colour), name, role, remove);

    const colours = document.createElement("div");
    colours.className = "reviewer-colours";
    colours.setAttribute("role", "radiogroup");
    colours.setAttribute("aria-label", `${r.name}'s colour`);
    for (const colour of COLOURS) {
      const pick = document.createElement("button");
      pick.type = "button";
      pick.className = "colour-pick";
      pick.style.background = colourOf(colour);
      pick.title = capital(colour);
      pick.setAttribute("role", "radio");
      pick.setAttribute("aria-checked", String(colour === r.colour));
      pick.addEventListener("click", () => run(async () => {
        if (colour !== r.colour) await update(r, { colour });
        await render();
      }));
      colours.append(pick);
    }
    li.append(fields, colours);
    return li;
  }

  function removedRow(r: Reviewer): HTMLLIElement {
    const li = document.createElement("li");
    li.className = "row";
    const label = document.createElement("span");
    label.className = "reviewer-label";
    label.textContent = r.role ? `${r.name} (${r.role})` : r.name;
    const back = document.createElement("button");
    back.type = "button";
    back.className = "link";
    back.textContent = "Bring back";
    back.addEventListener("click", () => run(async () => {
      await invoke("set_reviewer_removed", { id: r.id, removed: false });
      await render();
    }));
    li.append(swatch(r.colour), label, back);
    return li;
  }

  let reviewers: Reviewer[] = [];
  async function render() {
    reviewers = await invoke<Reviewer[]>("get_reviewers");
    const current = reviewers.filter((r) => !r.removed);
    const removed = reviewers.filter((r) => r.removed);
    $("reviewers-list").replaceChildren(...current.map(row));
    $("reviewers-none").hidden = current.length > 0;
    $("reviewers-removed").replaceChildren(...removed.map(removedRow));
    $("reviewers-removed-box").hidden = removed.length === 0;
  }

  const nameInput = $<HTMLInputElement>("reviewer-name");
  const roleInput = $<HTMLInputElement>("reviewer-role");
  async function add() {
    const name = nameInput.value.trim();
    if (!name) {
      nameInput.focus();
      return;
    }
    // Someone removed earlier with this name: offer them back (with their colour and Comments).
    const before = reviewers.find((r) => r.removed && r.name.toLowerCase() === name.toLowerCase());
    if (before && await confirm(`${before.name} was removed earlier. Bring them back, with their colour, instead of adding a new Reviewer?`, "Bring back")) {
      await invoke("set_reviewer_removed", { id: before.id, removed: false });
    } else {
      await invoke("add_reviewer", { name, role: roleInput.value });
    }
    nameInput.value = "";
    roleInput.value = "";
    await render();
    nameInput.focus();
  }
  const addButton = $("reviewer-add");
  const onAdd = () => run(add);
  // Enter in the boxes adds, instead of closing the dialog.
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      onAdd();
    }
  };
  addButton.addEventListener("click", onAdd);
  nameInput.addEventListener("keydown", onKey);
  roleInput.addEventListener("keydown", onKey);

  error.hidden = true;
  await render();
  dialog.returnValue = "";
  dialog.showModal();
  await new Promise<void>((resolve) => dialog.addEventListener("close", () => resolve(), { once: true }));
  addButton.removeEventListener("click", onAdd);
  nameInput.removeEventListener("keydown", onKey);
  roleInput.removeEventListener("keydown", onKey);
}
