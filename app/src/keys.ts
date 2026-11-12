// Keyboard shortcuts: the app's commands, their default keys, and the keys the Author gave them
// (a Vault setting, which a Device can have its own of).
//
// Keys are written the way CodeMirror writes them ("Mod-Shift-m": Mod is Ctrl, or Cmd on a Mac),
// so the editor's own keymap can use them as they are.

export type CommandId =
  | "toggleWorks" | "quickSwitcher" | "closeTab" | "toggleView" | "togglePanel" | "preamble" | "mods" | "find" | "replace"
  | "undo" | "redo" | "redoAlt" | "strong" | "emph" | "addComment" | "textHistory"
  // The Chapter list Mod's
  | "nextChapter" | "previousChapter" | "chapterUp" | "chapterDown";

export type Command = {
  id: CommandId;
  label: string;
  /** "app": anywhere in the app; "editor": while writing in a Work. */
  where: "app" | "editor";
  key: string;
  /**
   * The Mod it belongs to (mods.ts), if any. It's listed under that Mod whether or not any Work
   * has it, so its key is settled once; in a Work without the Mod, its key offers to add it.
   */
  mod?: string;
};

export const COMMANDS: Command[] = [
  { id: "toggleWorks", label: "Show or hide the Works", where: "app", key: "Mod-\\" },
  { id: "quickSwitcher", label: "Open a Work", where: "app", key: "Mod-o" },
  { id: "closeTab", label: "Close the tab", where: "app", key: "Mod-w" },
  { id: "toggleView", label: "Formatted or Raw view", where: "app", key: "Mod-e" },
  { id: "togglePanel", label: "Show or hide the panel", where: "app", key: "Mod-Shift-p" },
  { id: "preamble", label: "The Work's Preamble", where: "app", key: "Mod-Shift-l" },
  { id: "mods", label: "The Work's Mods", where: "app", key: "Mod-Shift-k" },
  { id: "find", label: "Find and replace", where: "app", key: "Mod-f" },
  { id: "replace", label: "Find and replace, with Replace showing", where: "app", key: "Mod-h" },
  { id: "undo", label: "Undo", where: "editor", key: "Mod-z" },
  { id: "redo", label: "Redo", where: "editor", key: "Mod-Shift-z" },
  { id: "redoAlt", label: "Redo (a second key)", where: "editor", key: "Mod-y" },
  { id: "strong", label: "Strong (bold)", where: "editor", key: "Mod-b" },
  { id: "emph", label: "Emphasis (italic)", where: "editor", key: "Mod-i" },
  { id: "addComment", label: "Add a Comment", where: "editor", key: "Mod-Shift-m" },
  { id: "textHistory", label: "History of this text (the selection, or the paragraph)", where: "editor", key: "Mod-Shift-h" },
  { id: "nextChapter", label: "Go to the next Chapter", where: "editor", key: "Alt-PageDown", mod: "chapters" },
  { id: "previousChapter", label: "Go to the previous Chapter", where: "editor", key: "Alt-PageUp", mod: "chapters" },
  { id: "chapterUp", label: "Move this Chapter up (a Move)", where: "editor", key: "Alt-Shift-PageUp", mod: "chapters" },
  { id: "chapterDown", label: "Move this Chapter down (a Move)", where: "editor", key: "Alt-Shift-PageDown", mod: "chapters" },
];

/** Each command's key; null: it has none. */
export type Bindings = Record<CommandId, string | null>;

/** The editor's basics, which can't be given to a command. */
const RESERVED = ["Mod-a", "Mod-c", "Mod-x", "Mod-v", "Mod-Space"];

const mac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

/** A key in one fixed form ("Mod-Alt-Shift-x"), so two ways of writing it compare equal. */
export function normalize(key: string): string {
  const parts = key.endsWith("--") ? [...key.slice(0, -2).split("-"), "-"] : key.split("-");
  let name = parts.pop() ?? "";
  const mods = new Set(parts.map((p) => p.toLowerCase()));
  const mod = mods.has("mod") || mods.has("ctrl") || mods.has("control") || mods.has("cmd") || mods.has("meta");
  if (name.length === 1) name = name.toLowerCase();
  if (name === " ") name = "Space";
  return [mod && "Mod", mods.has("alt") && "Alt", mods.has("shift") && "Shift", name].filter(Boolean).join("-");
}

type KeyEvent = { key: string; code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean };

/** The key pressed, in the fixed form; null for a modifier on its own. */
export function keyOfEvent(e: KeyEvent): string | null {
  if (["Control", "Shift", "Alt", "Meta", "AltGraph", "OS", "Dead", "Unidentified"].includes(e.key)) return null;
  let name = e.key;
  if (name === " ") name = "Space";
  else if (/^Digit\d$/.test(e.code)) name = e.code.slice(5); // Shift+7 is "7", not the symbol it types
  else if (name.length === 1) name = name.toLowerCase();
  const mod = mac ? e.metaKey : e.ctrlKey;
  return [mod && "Mod", e.altKey && "Alt", e.shiftKey && "Shift", name].filter(Boolean).join("-");
}

const NAMES: Record<string, string> = {
  ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right", Escape: "Esc", Delete: "Del", Backspace: "Backspace",
};

/** How a key is shown: "Ctrl+Shift+M". */
export function display(key: string): string {
  const parts = normalize(key).split("-");
  const name = parts.pop() ?? "";
  const mods = parts.map((p) => (p === "Mod" ? (mac ? "⌘" : "Ctrl") : p));
  return [...mods, NAMES[name] ?? (name.length === 1 ? name.toUpperCase() : name)].join("+");
}

/** Why a key can't be a shortcut, or null if it can. */
export function problemWith(key: string): string | null {
  const k = normalize(key);
  if (RESERVED.includes(k)) return `${display(k)} is kept for the editor itself.`;
  const parts = k.split("-");
  const name = parts[parts.length - 1];
  if (!parts.includes("Mod") && !parts.includes("Alt") && !/^F\d{1,2}$/.test(name)) {
    return "Use Ctrl or Alt with it (or an F key), so it doesn't type.";
  }
  return null;
}

/** Ctrl+Alt is AltGr on Windows keyboards, which types characters (µ, @, € …) on many of them. */
export function mayBeAltGr(key: string): boolean {
  const parts = normalize(key).split("-");
  return !mac && parts.includes("Mod") && parts.includes("Alt");
}

/** The keys in use: each command's default, unless the settings say otherwise ("none": no key). */
export function bindingsFrom(stored: Record<string, string>): Bindings {
  const bindings = {} as Bindings;
  for (const command of COMMANDS) {
    const value = stored[command.id];
    bindings[command.id] = value === "none" ? null : value ? normalize(value) : normalize(command.key);
  }
  return bindings;
}

/** The command that has this key, if any. */
export function commandWith(bindings: Bindings, key: string): CommandId | null {
  const k = normalize(key);
  return COMMANDS.find((c) => bindings[c.id] === k)?.id ?? null;
}

// ---- The keys in use now ----

let current: Bindings = bindingsFrom({});
const listeners: (() => void)[] = [];

export function setBindings(bindings: Bindings) {
  current = bindings;
  for (const listener of listeners) listener();
}

export function bindings(): Bindings {
  return current;
}

/** Runs `listener` whenever the keys change (labels that mention them follow). */
export function onBindingsChanged(listener: () => void) {
  listeners.push(listener);
}

/** A command's key as shown ("Ctrl+B"), or "" if it has none. */
export function keyLabel(command: CommandId): string {
  const key = current[command];
  return key ? display(key) : "";
}

/** "Text (Ctrl+B)", or just the text if the command has no key. */
export function withKey(text: string, command: CommandId): string {
  const key = keyLabel(command);
  return key ? `${text} (${key})` : text;
}

/** Does this keydown run `command`? Never while a new key is being recorded in the dialog. */
export function isCommand(e: KeyEvent, command: CommandId): boolean {
  const key = current[command];
  return !recording && key !== null && keyOfEvent(e) === key;
}

let recording = false;

/** The shortcuts dialog is waiting for a key: no command runs meanwhile. */
export function setRecording(on: boolean) {
  recording = on;
}
