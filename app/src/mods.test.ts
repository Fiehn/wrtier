import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { inUse, modExtensions, modNamed, unknownMods } from "./mods";
import { COMMANDS, normalize, problemWith } from "./keys";

const state = (work: string[], hidden: string[] = []) => ({ work, newWorks: [], hidden });

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
