// The Dialogue checker Mod (the v2 plan): its rules (dialogue.ts) as Marks in a Work's editor.
// English Works only. Its settings (vault/marks.rs), as "dialogue.<name>":
// - "dialogue.verb.<word>" for the whole Vault: "add" (a Speech verb the built-in list hasn't) or
//   "remove" (one it has that isn't one); unset otherwise. Both are kept, so a newer built-in
//   list never undoes a removal.
// - "dialogue.rule.<rule>" for one Work (and for new Works): "off" (not checked) or unset.

import type { Extension } from "@codemirror/state";
import { checkDialogue, RULES, SPEECH_VERBS, type Finding, type Options, type RuleId } from "./dialogue";
import { markSources, type Mark } from "./marks";
import type { ModContext, ModSettings } from "./mods";

export const DIALOGUE = "dialogue";
const VERB = `${DIALOGUE}.verb.`;
export const RULE = `${DIALOGUE}.rule.`;
const BUILT_IN = new Set(SPEECH_VERBS);

/** Is the Work in English (or in Typst's default, which is)? */
export const isEnglish = (language: string) => language.trim() === "" || language.trim().toLowerCase() === "en";

/** The Speech verbs in use: the built-in ones, with the Author's added and without those taken off. */
export function speechVerbs(settings: ModSettings): Set<string> {
  const verbs = new Set(SPEECH_VERBS);
  for (const [key, value] of Object.entries(settings.vault)) {
    if (!key.startsWith(VERB)) continue;
    const word = key.slice(VERB.length);
    if (value === "add") verbs.add(word);
    else if (value === "remove") verbs.delete(word);
  }
  return verbs;
}

/** The rules switched on in `scope` (a Work's settings, or new Works'). */
export function rulesOn(settings: Record<string, string>): Set<RuleId> {
  return new Set(RULES.map((r) => r.id).filter((id) => settings[`${RULE}${id}`] !== "off"));
}

export function optionsOf(settings: ModSettings): Options {
  return { speechVerbs: speechVerbs(settings), rules: rulesOn(settings.work) };
}

/** The setting that makes `word` a Speech verb (`on`) or not, and its value. */
export function verbSetting(word: string, on: boolean): [string, string] {
  const key = `${VERB}${word.toLowerCase()}`;
  const builtIn = BUILT_IN.has(word.toLowerCase());
  return [key, on === builtIn ? "" : on ? "add" : "remove"];
}

function toMark(f: Finding, opts: Options, ctx: ModContext): Mark {
  const mark: Mark = { mod: DIALOGUE, rule: f.rule, level: f.level, from: f.from, to: f.to, why: f.why, suggestions: f.suggestions };
  const verb = f.verb;
  if (verb) {
    const is = opts.speechVerbs.has(verb);
    mark.action = {
      label: is ? `Don't treat “${verb}” as a Speech verb` : `Treat “${verb}” as a Speech verb`,
      run: () => ctx.setSetting("vault", ...verbSetting(verb, !is)),
    };
  }
  return mark;
}

/** The Mod's part in a Work's editor: nothing unless the Work is in English. */
export function dialogueChecker(ctx: ModContext): Extension {
  if (!isEnglish(ctx.language)) return [];
  const opts = optionsOf(ctx.settings);
  return markSources.of({
    mod: DIALOGUE,
    find: (state) => checkDialogue(state.doc.toString(), opts).map((f) => toMark(f, opts, ctx)),
  });
}

// ---- Its part of the Mods dialog, and of Settings (Mods for new Works) ----

type Save = (scope: "vault" | "work" | "newWorks", key: string, value: string) => void;

const isWord = (w: string) => /^\p{L}[\p{L}'’-]*$/u.test(w);

/**
 * The rules, each with a switch, for this Work (`scope` "work") or for new Works ("newWorks");
 * and, for a Work, the Speech verbs, which are the whole Vault's.
 */
export function dialogueSettings(settings: ModSettings, scope: "work" | "newWorks", save: Save): HTMLElement {
  const box = document.createElement("details");
  box.className = "mod-settings";
  const summary = document.createElement("summary");
  summary.textContent = scope === "work" ? "Rules and Speech verbs" : "Rules for new Works";
  box.append(summary);

  const on = rulesOn(scope === "work" ? settings.work : settings.newWorks);
  const rules = document.createElement("ul");
  rules.className = "mod-rules";
  for (const rule of RULES) {
    const li = document.createElement("li");
    const label = document.createElement("label");
    label.className = "check";
    const check = document.createElement("input");
    check.type = "checkbox";
    check.checked = on.has(rule.id);
    check.addEventListener("change", () => save(scope, `${RULE}${rule.id}`, check.checked ? "" : "off"));
    const level = document.createElement("span");
    level.className = `mod-rule-level mod-rule-${rule.level}`;
    level.title = rule.level === "rule" ? "A Rule break (yellow)" : "A Query (blue)";
    label.append(check, level, ` ${rule.about}`);
    const example = document.createElement("span");
    example.className = "mod-rule-example";
    example.textContent = rule.example.replace(/\n\n/g, " ¶ ");
    li.append(label, example);
    rules.append(li);
  }
  box.append(rules);
  if (scope === "newWorks") return box;

  const verbs = speechVerbs(settings);
  const added = [...verbs].filter((v) => !BUILT_IN.has(v)).sort();
  const removed = [...BUILT_IN].filter((v) => !verbs.has(v)).sort();
  const head = document.createElement("p");
  head.className = "muted";
  head.textContent = `Speech verbs make the words next to dialogue a tag (“she said”) rather than an Action beat (“She nodded.”). ${BUILT_IN.size} are built in; this list is the whole Vault's.`;
  box.append(head);
  const listOf = (title: string, words: string[], areOn: boolean) => {
    const p = document.createElement("p");
    p.className = "mod-verbs";
    p.append(`${title}: `);
    if (words.length === 0) p.append("none");
    for (const w of words) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "mod-verb";
      chip.textContent = `${w} ×`;
      chip.title = areOn ? `Take “${w}” off again` : `Make “${w}” a Speech verb again`;
      chip.addEventListener("click", () => save("vault", ...verbSetting(w, !areOn)));
      p.append(chip);
    }
    return p;
  };
  box.append(listOf("Added", added, true), listOf("Taken off", removed, false));
  const row = document.createElement("div");
  row.className = "mod-verb-add";
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = "A verb as written: grinned";
  input.setAttribute("aria-label", "A Speech verb");
  const word = () => input.value.trim().toLowerCase();
  const add = document.createElement("button");
  add.type = "button";
  add.textContent = "Add";
  add.addEventListener("click", () => { if (isWord(word())) save("vault", ...verbSetting(word(), true)); });
  const off = document.createElement("button");
  off.type = "button";
  off.textContent = "Take off";
  off.addEventListener("click", () => { if (isWord(word())) save("vault", ...verbSetting(word(), false)); });
  row.append(input, add, off);
  box.append(row);
  return box;
}
