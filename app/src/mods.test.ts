import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { inUse, modExtensions, modNamed, unknownMods, unknownVaultMods, vaultModInUse, VAULT_MODS } from "./mods";
import { COMMANDS, normalize, problemWith } from "./keys";

const state = (work: string[], hidden: string[] = [], vault: string[] = []) => ({ work, newWorks: [], vault, hidden });

describe("which Mods are at work in a Work", () => {
  it("is the Work's own, less those this device hides", () => {
    expect(inUse(state(["spellcheck"])).map((m) => m.id)).toEqual(["spellcheck"]);
    expect(inUse(state(["spellcheck"], ["spellcheck"]))).toEqual([]);
    expect(inUse(state([], ["spellcheck"]))).toEqual([]);
  });

  it("leaves out a Mod from a newer version of the app, and names it", () => {
    expect(inUse(state(["dialogue", "spellcheck"])).map((m) => m.id)).toEqual(["spellcheck"]);
    expect(unknownMods(["dialogue", "spellcheck"])).toEqual(["dialogue"]);
  });
});

describe("Vault Mods", () => {
  it("are at work when the Vault has them and this device doesn't hide them", () => {
    expect(vaultModInUse(state([], [], ["colours"]), "colours")).toBe(true);
    expect(vaultModInUse(state([], ["colours"], ["colours"]), "colours")).toBe(false);
    expect(vaultModInUse(state([], [], []), "colours")).toBe(false);
  });

  it("are never a Work's Mods, and one from a newer version is named", () => {
    // Even listed on a Work (by hand, or a mistake), a Vault Mod isn't at work there.
    expect(inUse(state(["colours", "spellcheck"])).map((m) => m.id)).toEqual(["spellcheck"]);
    for (const m of VAULT_MODS) expect(modNamed(m.id), m.id).toBeUndefined();
    expect(unknownVaultMods(["colours", "tags"])).toEqual(["tags"]);
  });
});

describe("the spellcheck Mod", () => {
  // The attributes the Mods give the editor's text (CodeMirror itself starts with spellcheck "false").
  const spellcheck = (mods: string[]) => {
    const editor = EditorState.create({ doc: "Teh rain", extensions: modExtensions(inUse(state(mods))) });
    const attrs = editor.facet(EditorView.contentAttributes).map((a) => (typeof a === "function" ? null : a?.spellcheck));
    return attrs.find((v) => v) ?? null;
  };

  it("turns the platform's spellcheck on, which the editor otherwise keeps off", () => {
    expect(spellcheck(["spellcheck"])).toBe("true");
    expect(spellcheck([])).toBeNull();
  });
});

describe("Mods' commands", () => {
  it("belong to a Mod the app has, and every command has its own usable default key", () => {
    for (const c of COMMANDS) if (c.mod) expect(modNamed(c.mod), c.id).toBeDefined();
    const keys = COMMANDS.map((c) => normalize(c.key));
    expect(new Set(keys).size).toBe(keys.length);
    for (const c of COMMANDS) expect(problemWith(c.key), c.id).toBeNull();
  });
});
