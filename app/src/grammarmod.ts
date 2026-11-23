// The Grammar checker Mod (CONTEXT.md, ADR 0023): what Harper's rules find in the Work's text, as
// Marks. English Works only. The interface reads the text (grammar.ts); Rust runs Harper
// (grammar.rs), which loads the first time a Work with this Mod asks, and never before.
//
// Each look sends Rust only the passages it hasn't asked about yet, a few dozen at a time, and
// keeps the answers for as long as the Work still has those passages. A rule's Marks follow its
// setting, as "grammar.rule.<rule>" for one Work (and for new Works): "off" (not checked), "on"
// (checked, though off by default), or unset (as by default). Rules are Harper's own names ("AnA"),
// and our own CommaSplice (comma_splice.rs); Harper's SpellCheck is never on, since the Spellchecker spells.

import { invoke } from "@tauri-apps/api/core";
import type { Extension } from "@codemirror/state";
import { isEnglish } from "./dialoguemod";
import { marksFrom, toSend, type Found } from "./grammar";
import { markSources } from "./marks";
import type { ModContext, ModSettings } from "./mods";
import { passagesOf, readPara } from "./prose";

export const GRAMMAR = "grammar";
const RULE = `${GRAMMAR}.rule.`;

/** How many passages go to Rust at once: the lock is let go between them, and the answers kept as they come. */
const BATCH = 40;

/** What the Mod asks of Harper (Rust; the tests give their own): what it finds in each passage, or null if the language isn't English. */
export type Harper = (language: string, paragraphs: string[], off: string[]) => Promise<Found[][] | null>;

const rust: Harper = (language, paragraphs, off) => invoke<Found[][] | null>("grammar_check", { language, paragraphs, off });

/**
 * Harper's rules that are off unless a Work switches them on, from a run over a whole novel
 * (docs/v3-plan.md): they mark too much in fiction. SentenceCapitalization fires after "Mr." and "Mrs.".
 * A test in grammar.rs keeps these Harper's names.
 */
export const OFF_BY_DEFAULT = ["LongSentences", "OxfordComma", "BoringWords", "Excellent", "SentenceCapitalization"];

/** The rules off in `settings` (a Work's, or new Works'), by name: those off by default unless switched on, and those switched off. */
export function rulesOff(settings: Record<string, string>): string[] {
  const value = (rule: string) => settings[`${RULE}${rule}`];
  const switchedOff = Object.entries(settings).filter(([k, v]) => k.startsWith(RULE) && v === "off").map(([k]) => k.slice(RULE.length));
  return [...new Set([...OFF_BY_DEFAULT.filter((r) => value(r) !== "on"), ...switchedOff])].sort();
}

/** The setting that switches `rule` on or off: unset where that's its default. */
export function ruleSetting(rule: string, on: boolean): [string, string] {
  const byDefault = !OFF_BY_DEFAULT.includes(rule);
  return [`${RULE}${rule}`, on === byDefault ? "" : on ? "on" : "off"];
}

/** The Mod's part in a Work's editor: nothing unless the Work is in English. `harper`: for the tests. */
export function grammarChecker(ctx: ModContext, harper: Harper = rust): Extension {
  if (!isEnglish(ctx.language)) return [];
  const language = ctx.language;
  const off = rulesOff(ctx.settings.work);
  // Kept for the editor's life (its settings changing makes a new one): what Harper found in each passage, and what's still being asked.
  const known = new Map<string, Found[]>();
  const asking = new Map<string, Promise<void>>();

  const ask = (texts: string[]) => {
    const answer = harper(language, texts, off).then(
      (found) => texts.forEach((t, i) => {
        known.set(t, found?.[i] ?? []);
        asking.delete(t);
      }),
      (e) => {
        texts.forEach((t) => asking.delete(t));
        throw e;
      },
    );
    for (const t of texts) asking.set(t, answer);
  };

  return markSources.of({
    mod: GRAMMAR,
    async find(state) {
      const source = state.doc.toString();
      const sent = passagesOf(source).map((r) => toSend(readPara(source, r.start, r.end))).filter((s) => /\p{L}/u.test(s.text));
      const texts = new Set(sent.map((s) => s.text));
      // What the Work no longer has is forgotten; what it has and Harper hasn't seen is asked about.
      for (const t of [...known.keys()]) if (!texts.has(t)) known.delete(t);
      const missing = [...texts].filter((t) => !known.has(t) && !asking.has(t));
      for (let i = 0; i < missing.length; i += BATCH) ask(missing.slice(i, i + BATCH));
      await Promise.all([...texts].map((t) => asking.get(t)));
      return sent.flatMap((s) => marksFrom(s, known.get(s.text) ?? [], GRAMMAR));
    },
  });
}

// ---- Its part of the Mods dialog, and of Settings ----

type Save = (scope: "vault" | "work" | "newWorks", key: string, value: string) => void;

const DIALECTS: Record<string, string> = { GB: "British", US: "American", CA: "Canadian", AU: "Australian", IN: "Indian" };

/** The kind of English a Work in `language` is checked as (grammar.rs `dialect`). */
export const dialectOf = (language: string) => DIALECTS[(language.trim().split("-")[1] ?? "").toUpperCase()] ?? "British";

const isRule = (name: string) => /^[A-Za-z0-9-]+$/.test(name);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

function button(label: string, run: () => void, className = ""): HTMLButtonElement {
  const b = el("button", className, label);
  b.type = "button";
  b.addEventListener("click", run);
  return b;
}

/**
 * The Grammar checker's part of the Mods dialog (`scope` "work", with the Work's `language`), and
 * of the Mods for new Works ("newWorks"): which kind of English it checks, and the rules off (by
 * default, or switched off), each switched on by clicking it. Harper has hundreds of rules, so they aren't listed:
 * a rule is switched off from one of its Marks, or here by its name.
 */
export function grammarSettings(settings: ModSettings, scope: "work" | "newWorks", save: Save, language = ""): HTMLElement {
  const box = el("details", "mod-settings");
  box.append(el("summary", "", scope === "work" ? "Rules" : "Rules for new Works"));
  if (scope === "work") {
    box.append(el("p", "muted", isEnglish(language)
      ? `Checked as ${dialectOf(language)} English. Its rules load the first time a Work with it opens, which takes a moment.`
      : `The Work's language is ${language}, and the Grammar checker reads English only, so nothing is marked.`));
  }
  const p = el("p", "mod-verbs");
  p.append("Switched off: ");
  const off = rulesOff(scope === "work" ? settings.work : settings.newWorks);
  if (off.length === 0) p.append("none");
  for (const rule of off) {
    const chip = button(`${rule} ×`, () => save(scope, ...ruleSetting(rule, true)), "mod-verb");
    chip.title = `Check ${rule} again`;
    p.append(chip);
  }
  box.append(p, el("p", "muted", scope === "work"
    ? "Switch a rule off from one of its Marks (“Stop checking this rule in this Work”), or by its name."
    : "New Works start with these rules off. A rule switched off in a Work shows its name in that Work's list."));
  const row = el("div", "mod-verb-add");
  const input = el("input", "");
  input.type = "text";
  input.placeholder = "A rule's name: CommaSplice";
  input.setAttribute("aria-label", "A rule to switch off");
  row.append(input, button("Switch off", () => {
    const name = input.value.trim();
    if (isRule(name)) save(scope, ...ruleSetting(name, false));
  }));
  box.append(row);
  return box;
}
