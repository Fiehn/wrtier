// The Mods dialog: which Mods a Work has, and which this Device hides. Every change is saved at
// once (to vault.db, not the Work's History, so a Milestone or a Revert never touches it).
// The Mods for new Works, and the Vault Mods, are in Settings (`fillNewWorkMods`, `fillVaultMods`).

import { invoke } from "@tauri-apps/api/core";
import { MODS, unknownMods, unknownVaultMods, VAULT_MODS, type ModSettings, type ModsState } from "./mods";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

function checkbox(checked: boolean, change: (on: boolean) => void): HTMLInputElement {
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = checked;
  box.addEventListener("change", () => change(box.checked));
  return box;
}

function line(className: string, text: string): HTMLElement {
  const p = document.createElement("p");
  p.className = className;
  p.textContent = text;
  return p;
}

/** Mods from a newer version of the app: listed greyed, never switched from here. */
function unknownRows(ids: string[]): HTMLElement[] {
  return ids.map((id) => {
    const li = document.createElement("li");
    li.className = "mod-row unknown";
    li.append(line("mod-name", id), line("muted", "Needs a newer version of the app."));
    return li;
  });
}

/**
 * Opens the dialog for the Work `id`, in `language` (from its Preamble). `changed` runs after every change, with the state as
 * Rust saved it (the Work's editor follows at once); `settingsChanged` after a Mod's own
 * setting changed, or its Dismissed Marks were shown again. Resolves when it's closed.
 */
export async function openMods(id: string, name: string, language: string, changed: (state: ModsState) => void, settingsChanged: () => void): Promise<void> {
  const dialog = $<HTMLDialogElement>("mods");
  const list = $("mods-list");
  const error = $("mods-error");
  $("mods-heading").textContent = `Mods of ${name}`;
  let state = await invoke<ModsState>("get_mods", { id });
  let settings = await invoke<ModSettings>("get_mod_settings", { id });
  /** Which Mods' own settings are open, kept across a redraw. */
  const open = new Set<string>();

  const runSettings = (job: () => Promise<ModSettings>) => {
    error.hidden = true;
    job()
      .then((next) => {
        settings = next;
        settingsChanged();
        render();
      })
      .catch((e) => {
        error.textContent = String(e);
        error.hidden = false;
        render();
      });
  };
  const save = (scope: "vault" | "work" | "newWorks", key: string, value: string) =>
    runSettings(() => invoke<ModSettings>("set_mod_setting", { scope, id: scope === "work" ? id : null, key, value }));

  const run = (job: () => Promise<ModsState>) => {
    error.hidden = true;
    job()
      .then((next) => {
        state = next;
        changed(state);
        render();
      })
      .catch((e) => {
        error.textContent = String(e);
        error.hidden = false;
        render(); // the boxes go back to what's saved
      });
  };

  function render() {
    const rows = MODS.map((mod) => {
      const li = document.createElement("li");
      li.className = "mod-row";
      const on = state.work.includes(mod.id);
      const hidden = state.hidden.includes(mod.id);
      const main = document.createElement("label");
      main.className = "check mod-name";
      main.append(
        checkbox(on, (v) => run(() => invoke<ModsState>("set_mod", { id, name: mod.id, on: v }))),
        ` ${mod.name}`,
      );
      const hide = document.createElement("label");
      hide.className = "check here";
      hide.title = "This device leaves it out in every Work that has it. Other devices still use it.";
      hide.append(
        // Rust answers without a Work here: this Work's Mods are as they were.
        checkbox(hidden, (v) => run(async () => ({ ...(await invoke<ModsState>("set_mod_hidden", { name: mod.id, hidden: v })), work: state.work }))),
        " Hide on this device",
      );
      li.append(main, line("muted", mod.about), hide);
      if (on && hidden) li.append(line("muted", "This Work has it, but this device hides it."));
      if (on && mod.settings) {
        const part = mod.settings(settings, "work", save, language);
        if (part instanceof HTMLDetailsElement) {
          part.open = open.has(mod.id);
          part.addEventListener("toggle", () => (part.open ? open.add(mod.id) : open.delete(mod.id)));
        }
        li.append(part);
      }
      const dismissed = settings.dismissed[mod.id] ?? 0;
      if (mod.marks && dismissed > 0) {
        const p = line("muted mod-dismissed", `${dismissed} dismissed in this Work. `);
        const again = document.createElement("button");
        again.type = "button";
        again.textContent = "Show them again";
        again.addEventListener("click", () => runSettings(() => invoke<ModSettings>("show_dismissed", { id, name: mod.id })));
        p.append(again);
        li.append(p);
      }
      return li;
    });
    list.replaceChildren(...rows, ...unknownRows(unknownMods(state.work)));
  }

  error.hidden = true;
  render();
  await new Promise<void>((resolve) => {
    dialog.addEventListener("close", () => resolve(), { once: true });
    dialog.showModal();
  });
}

/**
 * The "Mods for new Works" part of Settings: one box per Mod, saved at once. Mods from a
 * newer version are named below, greyed.
 */
export async function fillNewWorkMods(list: HTMLElement): Promise<void> {
  let state = await invoke<ModsState>("get_mods", { id: null });
  let settings = await invoke<ModSettings>("get_mod_settings", { id: null });
  const save = (scope: "vault" | "work" | "newWorks", key: string, value: string) => {
    invoke<ModSettings>("set_mod_setting", { scope, id: null, key, value })
      .then((next) => {
        settings = next;
        render();
      })
      .catch((e) => list.append(line("dialog-error", String(e))));
  };
  const render = () => {
    const rows = MODS.map((mod) => {
      const li = document.createElement("li");
      const label = document.createElement("label");
      label.className = "check";
      label.title = mod.about;
      label.append(
        checkbox(state.newWorks.includes(mod.id), (on) => {
          invoke<ModsState>("set_new_work_mod", { name: mod.id, on })
            .then((next) => {
              state = next;
              render();
            })
            .catch((e) => {
              render(); // the box goes back to what's saved
              list.append(line("dialog-error", String(e)));
            });
        }),
        ` ${mod.name}`,
      );
      li.append(label);
      if (mod.settings && state.newWorks.includes(mod.id)) li.append(mod.settings(settings, "newWorks", save));
      return li;
    });
    list.replaceChildren(...rows, ...unknownRows(unknownMods(state.newWorks)));
  };
  render();
}

/**
 * The "Mods for the whole Vault" part of Settings: each Vault Mod's switch (for every Device)
 * and "Hide on this device", saved at once. `changed` runs after each change, with what Rust saved.
 */
export async function fillVaultMods(list: HTMLElement, changed: (state: ModsState) => void): Promise<void> {
  let state = await invoke<ModsState>("get_mods", { id: null });
  const save = (job: Promise<ModsState>) => {
    job
      .then((next) => {
        state = next;
        changed(state);
        render();
      })
      .catch((e) => {
        render(); // the boxes go back to what's saved
        list.append(line("dialog-error", String(e)));
      });
  };
  const render = () => {
    const rows = VAULT_MODS.map((mod) => {
      const li = document.createElement("li");
      li.className = "mod-row";
      const on = state.vault.includes(mod.id);
      const hidden = state.hidden.includes(mod.id);
      const main = document.createElement("label");
      main.className = "check mod-name";
      main.append(checkbox(on, (v) => save(invoke<ModsState>("set_vault_mod", { name: mod.id, on: v }))), ` ${mod.name}`);
      const hide = document.createElement("label");
      hide.className = "check here";
      hide.title = "This device leaves it out. Other devices still use it.";
      hide.append(checkbox(hidden, (v) => save(invoke<ModsState>("set_mod_hidden", { name: mod.id, hidden: v }))), " Hide on this device");
      li.append(main, line("muted", mod.about), hide);
      if (on && hidden) li.append(line("muted", "The Vault has it, but this device hides it."));
      return li;
    });
    list.replaceChildren(...rows, ...unknownRows(unknownVaultMods(state.vault)));
  };
  render();
}
