// The Syntax highlighting Mod (CONTEXT.md, ADR 0023): nouns, verbs, adjectives, adverbs and
// conjunctions each in a colour of their own, so the Author sees how their sentences are built
// (a run of adverbs, a page of "and"s). English Works only. It only colours the text in the
// editor: never the Page preview or an Export.
//
// The text is read as the Grammar checker reads it (grammar.ts `toSend`): emphasis seen through,
// headings, Story elements, comments and code left out, each Footnote a passage of its own.
// Harper (Rust, grammar.rs `word_types`) gives each word's type where it stands, so "record" is a
// verb in "I will record it" and a noun in "a record". A word Harper doesn't know gets no colour,
// unless it starts with a capital, when it's taken for a name (a noun).
//
// Only what's on screen is asked for, from paragraph edge to paragraph edge, and the answers are
// kept by the passage's text for every Work, so scrolling back, or a Mod's settings changing,
// asks nothing again. While the Author types, the colours move with the text; the paragraph
// being typed is asked about again once the typing pauses, and keeps its old colours until then.
//
// Which word types are coloured is the whole Vault's ("syntax.type.<type>": "off" or unset), as
// it's how the Author likes to read, not something about one Work; so is how ("syntax.style"):
// the words' own colour (unset), or a block of colour behind them ("block"), the text as it was.

import { invoke } from "@tauri-apps/api/core";
import { RangeSetBuilder, StateEffect, type EditorState, type Extension, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, type EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { isEnglish } from "./dialoguemod";
import { toSend } from "./grammar";
import type { ModContext, ModSettings } from "./mods";
import { notesOf, paragraphsOf, readPara } from "./prose";

export const SYNTAX = "syntax";
const TYPE = `${SYNTAX}.type.`;

/** A word and its type where it stands (grammar.rs `Word`): `from`..`to` in the passage as sent; `tag` a Universal POS tag. */
export type Word = { from: number; to: number; tag: string };

/** What the Mod asks of Harper (Rust; the tests give their own): each passage's words, or null if the language isn't English. */
export type Tagger = (language: string, paragraphs: string[]) => Promise<Word[][] | null>;

const rust: Tagger = (language, paragraphs) => invoke<Word[][] | null>("word_types", { language, paragraphs });

export type WordTypeId = "noun" | "verb" | "adjective" | "adverb" | "conjunction";

/** The word types coloured, each from Harper's tags. Pronouns, determiners, prepositions and the rest stay as they are. */
export const WORD_TYPES: { id: WordTypeId; name: string; tags: string[]; example: string }[] = [
  { id: "noun", name: "Nouns", tags: ["NOUN", "PROPN"], example: "house, Elizabeth" },
  { id: "verb", name: "Verbs", tags: ["VERB", "AUX"], example: "walked, was" },
  { id: "adjective", name: "Adjectives", tags: ["ADJ"], example: "old, quiet" },
  { id: "adverb", name: "Adverbs", tags: ["ADV"], example: "slowly, never" },
  { id: "conjunction", name: "Conjunctions", tags: ["CCONJ", "SCONJ"], example: "and, because" },
];

const TYPE_OF = new Map(WORD_TYPES.flatMap((t) => t.tags.map((tag) => [tag, t.id] as const)));

/** The word type a tag is coloured as, if any. */
export const typeOf = (tag: string): WordTypeId | undefined => TYPE_OF.get(tag);

/** The setting that switches a word type's colour off for the Vault ("off"), or back on (""). */
export const typeSetting = (id: WordTypeId, on: boolean): [string, string] => [`${TYPE}${id}`, on ? "" : "off"];

/** The word types coloured: all of them, less those switched off for the Vault. */
export function typesShown(settings: ModSettings): Set<WordTypeId> {
  return new Set(WORD_TYPES.filter((t) => settings.vault[`${TYPE}${t.id}`] !== "off").map((t) => t.id));
}

/** How the word types are shown: the words in their colour, or a block of it behind them. */
export type WordStyle = "text" | "block";

const STYLE = `${SYNTAX}.style`;

/** The setting for how word types are shown, for the Vault. */
export const styleSetting = (style: WordStyle): [string, string] => [STYLE, style === "block" ? "block" : ""];

/** How the Vault shows word types: as the words' colour unless it chose blocks. */
export const styleOf = (settings: ModSettings): WordStyle => (settings.vault[STYLE] === "block" ? "block" : "text");

/** How many passages a cache holds before it forgets those not on screen. */
const KEEP = 4000;

/** How many passages go to Rust at once. */
const BATCH = 40;

/** How long the typing must pause before the paragraph being typed is asked about again. */
const SETTLE_MS = 400;

/** What Harper said about each passage it was asked about (by its text), and what's still being asked. */
export class WordTypes {
  private known = new Map<string, Word[]>();
  private asking = new Set<string>();

  constructor(private tagger: Tagger = rust) {}

  get(text: string): Word[] | undefined {
    return this.known.get(text);
  }

  /** Asks about those of `texts` not known or being asked; resolves when all of those are answered. */
  ask(language: string, texts: string[]): Promise<void> {
    const missing = [...new Set(texts)].filter((t) => !this.known.has(t) && !this.asking.has(t));
    const asked: Promise<void>[] = [];
    for (let i = 0; i < missing.length; i += BATCH) {
      const batch = missing.slice(i, i + BATCH);
      batch.forEach((t) => this.asking.add(t));
      asked.push(this.tagger(language, batch).then(
        (words) => batch.forEach((t, n) => {
          this.known.set(t, words?.[n] ?? []);
          this.asking.delete(t);
        }),
        () => batch.forEach((t) => this.asking.delete(t)), // asked again at the next look
      ));
    }
    return Promise.all(asked).then(() => {});
  }

  /** Forgets all but `keep` once too much is kept. */
  prune(keep: Set<string>) {
    if (this.known.size <= KEEP) return;
    for (const t of [...this.known.keys()]) if (!keep.has(t)) this.known.delete(t);
  }
}

/** Every editor's: what Harper said is the same whichever Work the passage is in. */
const shared = new WordTypes();

/** The class a word type is shown with, in each style (also the examples in the Mods dialog). */
export const classOf = (id: WordTypeId, style: WordStyle) => (style === "block" ? `cm-word-block-${id}` : `cm-word-${id}`);

const looks = (style: WordStyle) =>
  Object.fromEntries(WORD_TYPES.map((t) => [t.id, Decoration.mark({ class: `cm-word ${classOf(t.id, style)}` })])) as Record<WordTypeId, Decoration>;
const LOOK: Record<WordStyle, Record<WordTypeId, Decoration>> = { text: looks("text"), block: looks("block") };

const isBlank = (line: string) => /^[ \t]*$/.test(line);

/** From the start of the paragraph at `from` to the end of the paragraph at `to` (blank line to blank line). */
export function paragraphEdges(doc: Text, from: number, to: number): [number, number] {
  let first = doc.lineAt(from);
  while (first.number > 1 && !isBlank(doc.line(first.number - 1).text)) first = doc.line(first.number - 1);
  let last = doc.lineAt(to);
  while (last.number < doc.lines && !isBlank(doc.line(last.number + 1).text)) last = doc.line(last.number + 1);
  return [first.from, last.to];
}

/**
 * The colours for the text between `from` and `to` (whole paragraphs), as far as `words` knows
 * them, with only the types in `shown`, in `style`. A passage it doesn't know yet keeps the colours `keep`
 * has there; `missing` are their texts, to ask about.
 */
export function colour(state: EditorState, from: number, to: number, words: WordTypes, shown: Set<WordTypeId>, keep: DecorationSet = Decoration.none, style: WordStyle = "text"): { decorations: DecorationSet; missing: string[]; texts: Set<string> } {
  const [start, end] = paragraphEdges(state.doc, from, to);
  const source = state.doc.sliceString(start, end);
  const passages = paragraphsOf(source).flatMap((p) => [p, ...notesOf(source, p.start, p.end)]);
  const found: { from: number; to: number; deco: Decoration }[] = [];
  const missing: string[] = [];
  const texts = new Set<string>();
  for (const passage of passages) {
    const s = toSend(readPara(source, passage.start, passage.end));
    if (!/\p{L}/u.test(s.text)) continue;
    texts.add(s.text);
    const known = words.get(s.text);
    if (!known) {
      missing.push(s.text);
      // Its old colours, until Harper answers (a Footnote's inside it too, which may then be coloured twice: see below).
      const [a, b] = [start + passage.start, start + passage.end];
      keep.between(a, b, (f, t, deco) => {
        if (f >= a && t <= b) found.push({ from: f, to: t, deco });
      });
      continue;
    }
    const p = s.para;
    for (const w of known) {
      const type = typeOf(w.tag);
      if (!type || !shown.has(type) || w.from < 0 || w.from >= w.to || w.to > s.at.length) continue;
      const span = s.at.slice(w.from, w.to);
      if (span.some((i) => i < 0)) continue; // across something left out
      found.push({ from: start + p.from[span[0]], to: start + p.to[span[span.length - 1]], deco: LOOK[style][type] });
    }
  }
  // A Footnote's passage comes after its paragraph's, and a kept colour may sit anywhere in its passage.
  found.sort((a, b) => a.from - b.from || a.to - b.to);
  const builder = new RangeSetBuilder<Decoration>();
  let at = -1;
  for (const f of found) {
    if (f.from < at) continue; // never two colours on one word, the first kept
    builder.add(f.from, f.to, f.deco);
    at = f.to;
  }
  return { decorations: builder.finish(), missing, texts };
}

/** Harper answered: colour again. */
const answered = StateEffect.define<null>();

/** The Mod's part in a Work's editor: nothing unless the Work is in English. `words`: for the tests. */
export function syntaxHighlighting(ctx: ModContext, words: WordTypes = shared): Extension {
  if (!isEnglish(ctx.language)) return [];
  const language = ctx.language;
  const shown = typesShown(ctx.settings);
  const style = styleOf(ctx.settings);
  if (shown.size === 0) return [];

  return ViewPlugin.fromClass(class {
    decorations: DecorationSet = Decoration.none;
    private timer: ReturnType<typeof setTimeout> | undefined;
    private gone = false;

    constructor(readonly view: EditorView) {
      this.look();
    }

    update(u: ViewUpdate) {
      if (u.docChanged) {
        this.decorations = this.decorations.map(u.changes);
        clearTimeout(this.timer);
        this.timer = setTimeout(() => this.refresh(), SETTLE_MS);
      } else if (u.viewportChanged || u.transactions.some((tr) => tr.effects.some((e) => e.is(answered)))) {
        // Not while a pause is awaited: the paragraph being typed would be asked about at every key.
        if (this.timer === undefined) this.look();
      }
    }

    private refresh() {
      this.timer = undefined;
      if (!this.gone) this.view.dispatch({ effects: answered.of(null) });
    }

    private look() {
      const { from, to } = this.view.viewport;
      const { decorations, missing, texts } = colour(this.view.state, from, to, words, shown, this.decorations, style);
      this.decorations = decorations;
      words.prune(texts);
      // An answer while the Author types waits for the pause, which looks again anyway.
      if (missing.length) void words.ask(language, missing).then(() => {
        if (this.timer === undefined && !this.gone) this.view.dispatch({ effects: answered.of(null) });
      });
    }

    destroy() {
      this.gone = true;
      clearTimeout(this.timer);
    }
  }, { decorations: (v) => v.decorations });
}

// ---- Its part of the Mods dialog, and of Settings ----

type Save = (scope: "vault" | "work" | "newWorks", key: string, value: string) => void;

/** Which word types are coloured, each with a switch, and how. The whole Vault's, so the same in both places. */
export function syntaxSettings(settings: ModSettings, scope: "work" | "newWorks", save: Save, language = ""): HTMLElement {
  const box = document.createElement("details");
  box.className = "mod-settings";
  const summary = document.createElement("summary");
  summary.textContent = "Word types";
  box.append(summary);
  if (scope === "work" && !isEnglish(language)) {
    const p = document.createElement("p");
    p.className = "muted";
    p.textContent = `The Work's language is ${language}, and word types are known for English only, so nothing is coloured.`;
    box.append(p);
  }
  const shown = typesShown(settings);
  const style = styleOf(settings);
  const list = document.createElement("ul");
  list.className = "mod-rules";
  for (const type of WORD_TYPES) {
    const li = document.createElement("li");
    const label = document.createElement("label");
    label.className = "check";
    const check = document.createElement("input");
    check.type = "checkbox";
    check.checked = shown.has(type.id);
    check.addEventListener("change", () => save("vault", ...typeSetting(type.id, check.checked)));
    const example = document.createElement("span");
    example.className = `mod-rule-example ${classOf(type.id, style)}`;
    example.textContent = type.example;
    label.append(check, ` ${type.name}`);
    li.append(label, example);
    list.append(li);
  }
  const styles = document.createElement("p");
  styles.className = "mod-word-styles";
  const options: [WordStyle, string][] = [["text", "Colour the words"], ["block", "A block of colour behind them"]];
  for (const [value, name] of options) {
    const label = document.createElement("label");
    label.className = "check";
    const radio = document.createElement("input");
    radio.type = "radio";
    radio.name = `syntax-style-${scope}`;
    radio.checked = style === value;
    radio.addEventListener("change", () => {
      if (radio.checked) save("vault", ...styleSetting(value));
    });
    label.append(radio, ` ${name}`);
    styles.append(label);
  }
  const note = document.createElement("p");
  note.className = "muted";
  note.textContent = "Which word types are coloured, and how, is the whole Vault's, in every Work with Syntax highlighting.";
  box.append(list, styles, note);
  return box;
}
