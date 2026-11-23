// Mods (ADR 0013): optional features that ship with the app, which the Author adds to each Work
// separately. A Mod only adds ways to see and work on a Work; it never changes the text, the
// History, or how the Work renders or exports.
//
// Rust keeps which Mods each Work has (in vault.db, one field per Mod) without knowing which
// Mods there are; this list is the only place that does. A Mod named there that isn't in this
// list comes from a newer version of the app: it's shown greyed and never switched from here.

import type { Extension } from "@codemirror/state";
import { dialogueChecker, dialogueSettings } from "./dialoguemod";
import { grammarChecker, grammarSettings } from "./grammarmod";
import { spellcheckSettings, spellchecker } from "./spellmod";
import { syntaxHighlighting, syntaxSettings } from "./syntaxmod";

export type Mod = {
  /** How it's stored: never change one once it has shipped. */
  id: string;
  name: string;
  /** One line saying what it does. */
  about: string;
  /** What it can get wrong, said wherever it's offered (the Mods dialog, and Settings). */
  caveat?: string;
  /** What it adds to a Work's editor while the Work has it. */
  editor?: (ctx: ModContext) => Extension;
  /** It makes Marks (marks.ts): the editor asks the Vault where its Dismissed ones are. */
  marks?: boolean;
  /** Its own part of the Mods dialog (`scope` "work", with the Work's `language`) and of the Mods for new Works ("newWorks"). */
  settings?: (settings: ModSettings, scope: "work" | "newWorks", save: (scope: "vault" | "work" | "newWorks", key: string, value: string) => void, language?: string) => HTMLElement;
};

/** Mods' own settings (vault/marks.rs `ModSettings`): each "<mod>.<name>" with its value. */
export type ModSettings = {
  vault: Record<string, string>;
  work: Record<string, string>;
  newWorks: Record<string, string>;
  /** How many Marks each Mod has dismissed in the Work. */
  dismissed: Record<string, number>;
};

/** What a Mod's editor part knows about the Work: its language, the Mods' settings, and how to change one. */
export type ModContext = {
  /** The Work's language from its Preamble ("" is Typst's default, English). */
  language: string;
  settings: ModSettings;
  /** Sets a Mod's setting ("" unsets it) for the Vault, this Work, or new Works; the editor follows. */
  setSetting(scope: "vault" | "work" | "newWorks", key: string, value: string): void;
};

export const noSettings = (): ModSettings => ({ vault: {}, work: {}, newWorks: {}, dismissed: {} });

export const MODS: Mod[] = [
  {
    id: "chapters",
    name: "Chapter list",
    about: "Lists the Work's Chapters on the left, with their word counts. Click one to go to it; drag one to reorder (a Move, so its History and Comments go with it).",
  },
  {
    id: "counts",
    name: "Word count",
    about: "Shows the Work's words, sentences and characters under the editor, and the selection's while text is selected. Markup, comments and line breaks aren't counted, and headings aren't sentences.",
  },
  {
    id: "spellcheck",
    name: "Spellchecker",
    about: "Marks words the Work's Dictionary doesn't know: red for a likely typo, blue for what looks like a name. Hover or click a Mark for suggestions, or to allow the word in this Work or add it to your dictionary.",
    caveat: "It isn't perfect: it can miss a misspelt word or mark one that's right, and its suggestions aren't always the word you meant. It has only been tested on English.",
    editor: (ctx) => spellchecker(ctx),
    marks: true,
    settings: spellcheckSettings,
  },
  {
    id: "dialogue",
    name: "Dialogue checker",
    about: "Marks dialogue that breaks the rules of English dialogue punctuation and paragraphing, and offers to put it right: yellow for a rule broken, blue for one to check. Hover or click a Mark, or press F8 to go from one to the next. English Works only.",
    caveat: "It isn't perfect: it can miss dialogue that breaks a rule or mark dialogue that's fine, so read a suggestion before accepting it. It has only been tested on English.",
    editor: (ctx) => dialogueChecker(ctx),
    marks: true,
    settings: dialogueSettings,
  },
  {
    id: "grammar",
    name: "Grammar checker",
    about: "Marks grammar, punctuation and word choice that Harper's rules catch: red for a likely mistake, yellow for a rule broken, blue for one to check. Hover or click a Mark for suggestions, or to stop checking its rule in this Work. Spelling is the Spellchecker's. English Works only.",
    caveat: "It isn't perfect: fiction breaks rules on purpose, in dialogue above all, and it can mark what's right or miss what's wrong. It loads the first time a Work with it opens, which takes a moment and about 80 MB of memory, and checking a long Work the first time can take half a minute.",
    editor: (ctx) => grammarChecker(ctx),
    marks: true,
    settings: grammarSettings,
  },
  {
    id: "syntax",
    name: "Syntax highlighting",
    about: "Colours nouns, verbs, adjectives, adverbs and conjunctions, each in its own colour, so you can see how your sentences are built. Each word's type is read from the words around it. Only in the editor, never in the Page preview or an Export. English Works only.",
    caveat: "It isn't perfect: it can take a word for the wrong type, and a word it doesn't know isn't coloured, unless it starts with a capital, when it's taken for a name. It reads with the Grammar checker's English, which loads the first time a Work with either opens, taking a moment and most of the Grammar checker's 80 MB of memory (once, when a Work has both).",
    editor: (ctx) => syntaxHighlighting(ctx),
    settings: syntaxSettings,
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
export function modExtensions(mods: Mod[], ctx: ModContext): Extension {
  return mods.flatMap((m) => (m.editor ? [m.editor(ctx)] : []));
}

/** Do any of these Mods make Marks? */
export const makesMarks = (mods: Mod[]) => mods.some((m) => m.marks);
