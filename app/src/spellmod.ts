// The Spellchecker Mod (CONTEXT.md, ADR 0019): words the Dictionary for the Work's language
// doesn't know, as Marks. An unknown word is a Mistake (red), or a Query (blue) when it looks like
// a name: capitalised where no sentence starts, there or anywhere else in the Work. Its card
// offers the words it may have meant (asked of the Dictionary only when the card opens), "Allow
// in Work", "Add to dictionary" and "Dismiss here only".
//
// The interface reads the text and collects its words (spelling.ts); Rust says which ones the
// Dictionary doesn't know (spelling.rs), and only about words it hasn't been asked about before.
//
// Its settings (vault/marks.rs), as "spellcheck.<name>":
// - "spellcheck.word.<language>.<word>" for the whole Vault: "add", an Added word, right in every
//   Work in that language whatever its region.
// - "spellcheck.allow.<word>" for one Work: "on", an Allowed word.
// - "spellcheck.rule.<rule>" for one Work (and for new Works): "off" (not checked) or unset.

import { invoke } from "@tauri-apps/api/core";
import type { Extension } from "@codemirror/state";
import { markSources, type Mark, type MarkLevel } from "./marks";
import type { ModContext, ModSettings } from "./mods";
import { editsFor, namesOf, OwnWords, plain, suggestionsFor, wordsOf, wordToKeep } from "./spelling";

export const SPELLCHECK = "spellcheck";
const WORD = `${SPELLCHECK}.word.`;
const ALLOW = `${SPELLCHECK}.allow.`;
const RULE = `${SPELLCHECK}.rule.`;

/** The rules, by the names they're stored under. Never change one once it has shipped. */
export const SPELL_RULES = [
  { id: "unknown", level: "mistake", about: "A word the Dictionary doesn't know.", example: "teh" },
  { id: "name", level: "query", about: "An unknown word that looks like a name: capitalised where no sentence starts, there or elsewhere in the Work.", example: "Kvothe" },
] as const satisfies readonly { id: string; level: MarkLevel; about: string; example: string }[];

type RuleId = (typeof SPELL_RULES)[number]["id"];

/** The language Added words are kept under: the language without its region ("en" for "en-GB", and for none). */
export const languageOf = (language: string) => language.trim().split("-")[0].toLowerCase() || "en";

/** The Added words for a language: the whole Vault's. */
export function addedWords(settings: ModSettings, language: string): string[] {
  const prefix = `${WORD}${languageOf(language)}.`;
  return Object.entries(settings.vault).filter(([k, v]) => k.startsWith(prefix) && v === "add").map(([k]) => k.slice(prefix.length));
}

/** A Work's Allowed words. */
export function allowedWords(work: Record<string, string>): string[] {
  return Object.entries(work).filter(([k, v]) => k.startsWith(ALLOW) && v === "on").map(([k]) => k.slice(ALLOW.length));
}

export function rulesOn(settings: Record<string, string>): Set<RuleId> {
  return new Set(SPELL_RULES.map((r) => r.id).filter((id) => settings[`${RULE}${id}`] !== "off"));
}

const addKey = (language: string, word: string) => `${WORD}${languageOf(language)}.${plain(word)}`;
const allowKey = (word: string) => `${ALLOW}${plain(word)}`;

// ---- Asking the Dictionary ----

/** What the Mod asks of the Dictionary (Rust; the tests give their own). */
export type Dictionary = {
  /** Which of `words` it doesn't know; null if there's no Dictionary for the language. */
  unknown(language: string, words: string[]): Promise<string[] | null>;
  suggest(language: string, word: string): Promise<string[]>;
};

const rust: Dictionary = {
  unknown: (language, words) => invoke<string[] | null>("spell_check", { language, words }),
  suggest: (language, word) => invoke<string[]>("spell_suggest", { language, word }),
};

/** What the Dictionaries said, by language: whether each word asked about is known, and what was suggested. */
const known = new Map<string, Map<string, boolean>>();
const suggested = new Map<string, Map<string, string[]>>();

/** Forgets what the Dictionaries said (one was downloaded, updated or removed). */
export function forgetSpelling() {
  known.clear();
  suggested.clear();
}

const of = <T>(map: Map<string, Map<string, T>>, language: string) => {
  let m = map.get(language);
  if (!m) map.set(language, (m = new Map()));
  return m;
};

// ---- In the editor ----

/** The Mod's part in a Work's editor. `dictionary`: for the tests. */
export function spellchecker(ctx: ModContext, dictionary: Dictionary = rust): Extension {
  const language = ctx.language;
  const rules = rulesOn(ctx.settings.work);
  const own = new OwnWords([...addedWords(ctx.settings, language), ...allowedWords(ctx.settings.work)]);
  const knownHere = of(known, language);

  const suggest = async (word: string) => {
    const cache = of(suggested, language);
    let words = cache.get(word);
    if (!words) cache.set(word, (words = await dictionary.suggest(language, word)));
    return suggestionsFor(word, words, own.all);
  };

  return markSources.of({
    mod: SPELLCHECK,
    async find(state) {
      const words = wordsOf(state.doc.toString());
      const ask = [...new Set(words.map((w) => w.word).filter((w) => !own.knows(w) && !knownHere.has(w)))];
      if (ask.length > 0) {
        const unknown = await dictionary.unknown(language, ask);
        if (unknown === null) return []; // no Dictionary for it: nothing is marked (the Mods dialog says so)
        const out = new Set(unknown);
        for (const w of ask) knownHere.set(w, !out.has(w));
      }
      const names = namesOf(words);
      const marks: Mark[] = [];
      for (const w of words) {
        if (own.knows(w.word) || knownHere.get(w.word) !== false) continue;
        const capital = w.word[0] !== w.word[0].toLowerCase();
        const rule: RuleId = capital && (w.midSentence || names.has(w.word)) ? "name" : "unknown";
        if (!rules.has(rule)) continue;
        const keep = wordToKeep(w.word, rule === "name");
        marks.push({
          mod: SPELLCHECK,
          rule,
          level: rule === "name" ? "query" : "mistake",
          from: w.from,
          to: w.to,
          why: rule === "name" ? `“${w.word}” isn't in the Dictionary. A name?` : `“${w.word}” isn't in the Dictionary.`,
          suggestions: [],
          suggest: async (mark, st) => (await suggest(w.word)).map((s) => ({ label: s, edits: editsFor(st.doc, mark.from, mark.to, s) })),
          actions: [
            { label: "Allow in Work", run: () => ctx.setSetting("work", allowKey(keep), "on") },
            { label: "Add to dictionary", run: () => ctx.setSetting("vault", addKey(language, keep), "add") },
          ],
          // A dismissal is of this spelling here, whichever kind it shows as, until the word changes (ADR 0018).
          dismissAs: "word",
          dismissExact: true,
          dismissLabel: "Dismiss here only",
        });
      }
      return marks;
    },
  });
}

// ---- Its part of the Mods dialog, and of Settings ----

type Save = (scope: "vault" | "work" | "newWorks", key: string, value: string) => void;

/** What spelling.rs says about a Work's Dictionary. */
type DictionaryFor = {
  code: string | null;
  name: string | null;
  builtIn: boolean;
  insteadOf: string | null;
  offer: Offered | null;
};
type Offered = { code: string; name: string; license: string; size: number };
type Kept = { code: string; name: string; license: string | null; update: boolean };
type Dictionaries = { inVault: Kept[]; builtIn: Kept[]; offered: Offered[] };

/** Tells every editor the Dictionaries changed, so the Spellchecker looks again (main.ts listens). */
function dictionariesChanged() {
  forgetSpelling();
  window.dispatchEvent(new CustomEvent("wrtier-dictionaries-changed"));
}

const mb = (bytes: number) => `${(bytes / 1_000_000).toFixed(1)} MB`;

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

/** A button that downloads a Dictionary, saying how it goes; `done` after. */
function downloadButton(offer: Offered, label: string, status: HTMLElement, done: () => void): HTMLButtonElement {
  const b = button(label, () => {
    b.disabled = true;
    status.textContent = `Downloading ${offer.name}…`;
    invoke("download_dictionary", { code: offer.code })
      .then(() => {
        status.textContent = `${offer.name} is in the Vault now.`;
        dictionariesChanged();
        done();
      })
      .catch((e) => {
        status.textContent = String(e);
        b.disabled = false;
      });
  });
  b.title = `${mb(offer.size)}, licence ${offer.license}. Kept in the Vault's dictionaries folder, with its licence, for every device.`;
  return b;
}

/** Which Dictionary a Work is checked with, and what can be downloaded for it. Filled in when Rust answers. */
function dictionaryLine(language: string): HTMLElement {
  const box = el("div", "spell-dictionary");
  const status = el("p", "muted");
  const fill = () => {
    invoke<DictionaryFor>("dictionary_for", { language })
      .then((d) => {
        const lines: (string | HTMLElement)[] = [];
        if (d.code) {
          lines.push(`Checked with the ${d.name} Dictionary, ${d.builtIn ? "built into the app" : "from the Vault"}.`);
          if (d.insteadOf) lines.push(` There's no ${d.insteadOf} Dictionary, so it stands in.`);
          else if (!language.includes("-")) lines.push(" To check another kind of the language, give it a region in the Preamble, such as en-US.");
        } else {
          lines.push(`There's no Dictionary for the Work's language (${language || "en"}), so nothing is marked.`);
        }
        const p = el("p", "muted");
        p.append(...lines);
        box.replaceChildren(p);
        if (d.offer) {
          const row = el("p", "spell-offer");
          row.append(downloadButton(d.offer, `Download ${d.offer.name}`, status, fill), ` ${mb(d.offer.size)}, licence ${d.offer.license}`);
          box.append(row);
        }
        box.append(status);
      })
      .catch((e) => box.replaceChildren(el("p", "dialog-error", String(e))));
  };
  box.append(el("p", "muted", "Looking for its Dictionary…"));
  fill();
  return box;
}

/** A list of words as chips, each taken back by clicking it, and a field to add one. */
function wordList(title: string, words: string[], remove: (w: string) => void, add: (w: string) => void, placeholder: string): HTMLElement {
  const box = el("div", "spell-words");
  const p = el("p", "mod-verbs");
  p.append(`${title}: `);
  if (words.length === 0) p.append("none");
  for (const w of [...words].sort((a, b) => a.localeCompare(b))) {
    const chip = button(`${w} ×`, () => remove(w), "mod-verb");
    chip.title = `Take “${w}” back`;
    p.append(chip);
  }
  const row = el("div", "mod-verb-add");
  const input = el("input", "");
  input.type = "text";
  input.placeholder = placeholder;
  input.setAttribute("aria-label", title);
  const word = () => input.value.trim();
  const ok = (w: string) => /^[\p{L}\p{M}]+(?:['’][\p{L}\p{M}]+)*$/u.test(w);
  row.append(input, button("Add", () => { if (ok(word())) add(word()); }));
  box.append(p, row);
  return box;
}

/**
 * The Spellchecker's part of the Mods dialog (`scope` "work", with the Work's `language`):
 * its Dictionary, its two rules, the Work's Allowed words and the Added words for its language.
 * For new Works ("newWorks"), only the rules.
 */
export function spellcheckSettings(settings: ModSettings, scope: "work" | "newWorks", save: Save, language = ""): HTMLElement {
  const box = el("details", "mod-settings");
  box.append(el("summary", "", scope === "work" ? "Dictionary, rules and words" : "Rules for new Works"));
  if (scope === "work") box.append(dictionaryLine(language));

  const on = rulesOn(scope === "work" ? settings.work : settings.newWorks);
  const rules = el("ul", "mod-rules");
  for (const rule of SPELL_RULES) {
    const li = el("li", "");
    const label = el("label", "check");
    const check = el("input", "");
    check.type = "checkbox";
    check.checked = on.has(rule.id);
    check.addEventListener("change", () => save(scope, `${RULE}${rule.id}`, check.checked ? "" : "off"));
    const level = el("span", `mod-rule-level mod-rule-${rule.level}`);
    level.title = rule.level === "mistake" ? "A Mistake (red)" : "A Query (blue)";
    label.append(check, level, ` ${rule.about}`);
    li.append(label, el("span", "mod-rule-example", rule.example));
    rules.append(li);
  }
  box.append(rules);
  if (scope === "newWorks") return box;

  box.append(
    wordList("Allowed in this Work", allowedWords(settings.work),
      (w) => save("work", allowKey(w), ""), (w) => save("work", allowKey(w), "on"), "A name: Kvothe"),
    el("p", "muted", `Added words are right in every Work in this language (${languageOf(language)}), whatever its region; the list is the whole Vault's.`),
    wordList("Added to the dictionary", addedWords(settings, language),
      (w) => save("vault", addKey(language, w), ""), (w) => save("vault", addKey(language, w), "add"), "A word of yours: wrtier"),
  );
  return box;
}

/**
 * The Dictionaries part of Settings: those in the Vault (to update or remove), those built in,
 * and a list of those the app offers to download, each with its size and licence.
 */
export async function fillDictionaries(box: HTMLElement): Promise<void> {
  const status = el("p", "muted");
  const render = async () => {
    let d: Dictionaries;
    try {
      d = await invoke<Dictionaries>("list_dictionaries");
    } catch (e) {
      box.replaceChildren(el("p", "dialog-error", String(e)));
      return;
    }
    const list = el("ul", "dictionary-list");
    for (const k of d.builtIn) {
      const overridden = d.inVault.some((v) => v.code === k.code);
      list.append(el("li", "", `${k.name} (${k.code}), built into the app${overridden ? ": the Vault's own copy is used instead" : ""}.`));
    }
    for (const k of d.inVault) {
      const li = el("li", "");
      li.append(`${k.name} (${k.code}), in the Vault${k.license ? `, licence ${k.license}` : ""}. `);
      const offer = k.update ? { code: k.code, name: k.name, license: k.license ?? "", size: 0 } : null;
      if (offer) li.append(downloadButton(offer, "Update", status, () => void render()), " ");
      li.append(button("Remove", () => {
        if (!window.confirm(`Remove the ${k.name} Dictionary from the Vault? Every device stops using it, and Works in it are checked with ${d.builtIn.some((b) => b.code === k.code) ? "the built-in one" : "nothing, until it's downloaded again"}.`)) return;
        invoke("remove_dictionary", { code: k.code })
          .then(() => {
            dictionariesChanged();
            void render();
          })
          .catch((e) => (status.textContent = String(e)));
      }));
      list.append(li);
    }
    const row = el("div", "dictionary-download");
    const pick = el("select", "");
    pick.setAttribute("aria-label", "A Dictionary to download");
    for (const o of d.offered) {
      const opt = el("option", "", `${o.name} (${o.code}), ${mb(o.size)}, ${o.license}`);
      opt.value = o.code;
      pick.append(opt);
    }
    const get = button("Download", () => {
      const offer = d.offered.find((o) => o.code === pick.value);
      if (offer) downloadButton(offer, "", status, () => void render()).click();
    });
    row.append(pick, get);
    box.replaceChildren(list, row, status);
  };
  await render();
}
