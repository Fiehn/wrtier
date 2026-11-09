// The Keyboard shortcuts dialog (from Settings): every command the app has a key for, and the
// key it has. Click a key, then press the new one. Shortcuts are a Vault setting, so every
// Device has the same, unless one has its own (a different keyboard).

import { invoke } from "@tauri-apps/api/core";
import {
  bindings, COMMANDS, commandWith, display, keyOfEvent, mayBeAltGr, normalize, problemWith, setRecording, type Command, type CommandId,
} from "./keys";
import { MODS } from "./mods";

/** The part of the settings that's about shortcuts (vault/settings.rs). */
type ShortcutSettings = { shortcuts: Record<string, string>; shortcutsHere: boolean };

export type ShortcutsHooks<S extends ShortcutSettings> = {
  /** Uses settings Rust sent back (the keys change everywhere at once). */
  apply(settings: S): void;
  confirm(text: string, yes: string): Promise<boolean>;
};

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

/** The app's own commands by where they work, then each Mod's under its name. */
const GROUPS: { title: string; has: (c: Command) => boolean }[] = [
  { title: "Anywhere in the app", has: (c) => !c.mod && c.where === "app" },
  { title: "While writing", has: (c) => !c.mod && c.where === "editor" },
  ...MODS.map((m) => ({ title: `${m.name} (a Mod)`, has: (c: Command) => c.mod === m.id })),
];

export async function openShortcuts<S extends ShortcutSettings>(settings: S, hooks: ShortcutsHooks<S>): Promise<void> {
  const dialog = $<HTMLDialogElement>("shortcuts");
  const list = $("shortcuts-list");
  const error = $("shortcuts-error");
  const here = $<HTMLInputElement>("shortcuts-here");
  let recording: CommandId | null = null;

  const showError = (message: string | null) => {
    error.textContent = message ?? "";
    error.hidden = message === null;
  };

  async function save(keys: [string, string][]) {
    hooks.apply(await invoke<S>("set_shortcuts", { keys }));
  }

  function render() {
    const keys = bindings();
    here.checked = settings.shortcutsHere;
    list.replaceChildren(...GROUPS.flatMap((group) => {
      const commands = COMMANDS.filter(group.has);
      if (commands.length === 0) return [];
      const heading = document.createElement("h3");
      heading.textContent = group.title;
      const rows = commands.map((command) => {
        const row = document.createElement("div");
        row.className = "shortcut-row";
        const label = document.createElement("span");
        label.textContent = command.label;
        const key = document.createElement("button");
        key.type = "button";
        key.className = "shortcut-key";
        const current = keys[command.id];
        key.textContent = recording === command.id ? "Press the keys…" : current ? display(current) : "No key";
        key.classList.toggle("recording", recording === command.id);
        key.classList.toggle("none", !current);
        key.title = "Click, then press the new keys (Esc: keep it, Backspace: no key)";
        key.addEventListener("click", () => startRecording(command.id));
        row.append(label, key);
        if (current !== normalize(command.key)) {
          const reset = document.createElement("button");
          reset.type = "button";
          reset.className = "link";
          reset.textContent = "Default";
          reset.title = `Back to ${display(command.key)}`;
          reset.addEventListener("click", () => void run(() => give(command.id, normalize(command.key))));
          row.append(reset);
        }
        return row;
      });
      return [heading, ...rows];
    }));
  }

  const run = async (job: () => Promise<void>) => {
    showError(null);
    try {
      await job();
    } catch (e) {
      showError(String(e));
    }
    render();
  };

  /** Gives `command` the key `key`, asking first if another command has it (that one then has none). */
  async function give(command: CommandId, key: string) {
    const label = (id: CommandId) => COMMANDS.find((c) => c.id === id)!.label;
    const other = commandWith(bindings(), key);
    if (other === command) return;
    if (other && !(await hooks.confirm(
      `${display(key)} is already the key for “${label(other)}”. Use it for “${label(command)}” instead? “${label(other)}” will then have no key.`,
      "Use it",
    ))) return;
    const def = normalize(COMMANDS.find((c) => c.id === command)!.key);
    const keys: [string, string][] = [[command, key === def ? "" : key]];
    if (other) keys.unshift([other, "none"]);
    await save(keys);
  }

  function startRecording(command: CommandId) {
    recording = command;
    setRecording(true);
    showError(null);
    render();
  }

  function stopRecording() {
    recording = null;
    setRecording(false);
  }

  // While recording, the next key press is the new key (nothing else runs on it).
  const onKey = (e: KeyboardEvent) => {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    const command = recording;
    const plain = !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey;
    if (plain && e.key === "Escape") {
      stopRecording();
      render();
      return;
    }
    if (plain && (e.key === "Backspace" || e.key === "Delete")) {
      stopRecording();
      void run(() => save([[command, "none"]]));
      return;
    }
    const key = keyOfEvent(e);
    if (!key) return; // a modifier on its own: wait for the rest
    stopRecording();
    const problem = problemWith(key);
    if (problem) {
      showError(problem);
      render();
      return;
    }
    void run(async () => {
      if (mayBeAltGr(key) && !(await hooks.confirm(
        `On Windows, Ctrl+Alt is the same as AltGr, which types characters (such as µ, @ or €) on many keyboards. ${display(key)} may type one instead. Use it anyway?`,
        "Use it",
      ))) return;
      await give(command, key);
    });
  };
  // Escape closes a dialog; while recording it only stops recording.
  const onCancel = (e: Event) => {
    if (recording) e.preventDefault();
  };

  const onHere = () => void run(async () => {
    settings = await invoke<S>("set_shortcuts_here", { here: here.checked });
    hooks.apply(settings);
  });
  const onReset = () => void run(async () => {
    if (!(await hooks.confirm("Put every shortcut back to the app's default?", "Reset all"))) return;
    settings = await invoke<S>("reset_shortcuts");
    hooks.apply(settings);
  });

  window.addEventListener("keydown", onKey, { capture: true });
  dialog.addEventListener("cancel", onCancel);
  here.addEventListener("change", onHere);
  $("shortcuts-reset").addEventListener("click", onReset);
  showError(null);
  render();
  dialog.returnValue = "";
  dialog.showModal();
  await new Promise<void>((resolve) => dialog.addEventListener("close", () => resolve(), { once: true }));
  stopRecording();
  window.removeEventListener("keydown", onKey, { capture: true });
  dialog.removeEventListener("cancel", onCancel);
  here.removeEventListener("change", onHere);
  $("shortcuts-reset").removeEventListener("click", onReset);
}
