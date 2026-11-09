import { describe, expect, it } from "vitest";
import { bindingsFrom, commandWith, display, keyOfEvent, mayBeAltGr, normalize, problemWith } from "./keys";

const press = (key: string, mods: { ctrl?: boolean; alt?: boolean; shift?: boolean } = {}, code = "") => ({
  key, code, ctrlKey: !!mods.ctrl, altKey: !!mods.alt, shiftKey: !!mods.shift, metaKey: false,
});

describe("keys", () => {
  it("are written one way, whatever order or case they come in", () => {
    expect(normalize("Shift-Mod-M")).toBe("Mod-Shift-m");
    expect(normalize("Ctrl-\\")).toBe("Mod-\\");
    expect(normalize("Mod--")).toBe("Mod--");
  });

  it("are read from a key press", () => {
    expect(keyOfEvent(press("M", { ctrl: true, shift: true }, "KeyM"))).toBe("Mod-Shift-m");
    expect(keyOfEvent(press("&", { ctrl: true, shift: true }, "Digit7"))).toBe("Mod-Shift-7");
    expect(keyOfEvent(press("F2", {}, "F2"))).toBe("F2");
    expect(keyOfEvent(press("Control", { ctrl: true }))).toBeNull();
  });

  it("are shown the way people write them", () => {
    expect(display("Mod-Shift-m")).toBe("Ctrl+Shift+M");
    expect(display("Mod-\\")).toBe("Ctrl+\\");
    expect(display("Alt-ArrowUp")).toBe("Alt+Up");
  });

  it("must not type, and must leave the editor's basics alone", () => {
    expect(problemWith("m")).not.toBeNull();
    expect(problemWith("Shift-m")).not.toBeNull();
    expect(problemWith("Mod-c")).not.toBeNull();
    expect(problemWith("F5")).toBeNull();
    expect(problemWith("Alt-m")).toBeNull();
    expect(mayBeAltGr("Mod-Alt-m")).toBe(true);
    expect(mayBeAltGr("Mod-Shift-m")).toBe(false);
  });

  it("start from the defaults, and the settings change or remove some", () => {
    const b = bindingsFrom({ addComment: "Mod-Shift-c", strong: "none" });
    expect(b.addComment).toBe("Mod-Shift-c");
    expect(b.strong).toBeNull();
    expect(b.emph).toBe("Mod-i");
    expect(commandWith(b, "Ctrl-I")).toBe("emph");
    expect(commandWith(bindingsFrom({}), "Mod-Shift-m")).toBe("addComment");
  });
});
