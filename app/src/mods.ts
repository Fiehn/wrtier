// Mods (ADR 0013): optional features that ship with the app, which the Author adds to each Work
// separately. A Mod only adds ways to see and work on a Work; it never changes the text, the
// History, or how the Work renders or exports.
//
// Rust keeps which Mods each Work has (in vault.db, one field per Mod) without knowing which
// Mods there are; this list is the only place that does. A Mod named there that isn't in this
// list comes from a newer version of the app: it's shown greyed and never switched from here.

import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

export type Mod = {
  /** How it's stored: never change one once it has shipped. */
  id: string;
  name: string;
  /** One line saying what it does. */
  about: string;
  /** What it adds to a Work's editor while the Work has it. */
  editor?: () => Extension;
};

export const MODS: Mod[] = [
  {
    id: "chapters",
    name: "Chapter list",
    about: "Lists the Work's Chapters on the left, with their word counts. Click one to go to it; drag one to reorder (a Move, so its History and Comments go with it).",
  },
  {
    id: "counts",
    name: "Word and character count",
    about: "Shows the Work's words and characters under the editor, and the selection's while text is selected. Markup, comments and line breaks aren't counted.",
  },
  {
    id: "spellcheck",
    name: "Spellcheck",
    about: "Underlines misspelt words, using this device's own spellchecker and its language. Right-click a word for suggestions.",
    // The editor turns the platform's spellcheck off; this turns it back on.
    editor: () => EditorView.contentAttributes.of({ spellcheck: "true" }),
  },
];

/**
 * Vault Mods: added once to the whole Vault, since they're about the Vault and not about any
 * one Work. Never offered in a Work's Mods, or for new Works.
 */
export const VAULT_MODS: Mod[] = [
  {
    id: "colours",
    name: "Colours",
    about: "Colour Works and folders in the Vault browser and Find in Vault. Right-click one and choose Colour…. Switching this off keeps the colours for later.",
  },
];

/** Which Mods a Work has, which new Works get, which Vault Mods the Vault has, and which this Device hides (vault/mods.rs). */
export type ModsState = { work: string[]; newWorks: string[]; vault: string[]; hidden: string[] };

export function modNamed(id: string): Mod | undefined {
  return MODS.find((m) => m.id === id);
}

/** The Mods at work in a Work on this Device: the Work has them, this Device doesn't hide them, and this app has them. */
export function inUse(state: ModsState): Mod[] {
  return MODS.filter((m) => state.work.includes(m.id) && !state.hidden.includes(m.id));
}

/** The names in `list` this app has no Mod for (they come from a newer version). */
export function unknownMods(list: string[]): string[] {
  return list.filter((id) => !modNamed(id));
}

/** The Vault Mods in `list` this app has no Vault Mod for. */
export function unknownVaultMods(list: string[]): string[] {
  return list.filter((id) => !VAULT_MODS.some((m) => m.id === id));
}

/** Is a Vault Mod at work on this Device: the Vault has it, and this Device doesn't hide it? */
export function vaultModInUse(state: ModsState, id: string): boolean {
  return (state.vault ?? []).includes(id) && !state.hidden.includes(id);
}

/** What a Work's Mods add to its editor. */
export function modExtensions(mods: Mod[]): Extension {
  return mods.flatMap((m) => (m.editor ? [m.editor()] : []));
}
