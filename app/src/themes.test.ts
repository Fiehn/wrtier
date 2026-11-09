import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { resolveTheme, themeChoices, type ThemeList } from "./themes";

const list: ThemeList = {
  themes: [
    { file: "night.css", name: "Night", dark: true },
    { file: "sepia.css", name: "Sepia", dark: false },
  ],
  problems: [],
};

describe("themes", () => {
  it("starts a Vault theme from the built-in colours of its kind", () => {
    expect(resolveTheme("vault:sepia.css", list)).toEqual({ dataTheme: "light", file: "sepia.css", missing: null });
    expect(resolveTheme("vault:night.css", list)).toEqual({ dataTheme: "dark", file: "night.css", missing: null });
  });

  it("keeps the built-in themes as they were", () => {
    expect(resolveTheme("light", list)).toEqual({ dataTheme: "light", file: null, missing: null });
    expect(resolveTheme("dark", list)).toEqual({ dataTheme: "dark", file: null, missing: null });
    expect(resolveTheme("system", list)).toEqual({ dataTheme: null, file: null, missing: null });
  });

  it("follows the system when the chosen theme isn't in the Vault, and says which", () => {
    expect(resolveTheme("vault:gone.css", list)).toEqual({ dataTheme: null, file: null, missing: "gone.css" });
  });

  it("lists the built-in themes, then the Vault's, and a missing one so the setting still shows", () => {
    expect(themeChoices(list, null).map((c) => c.label)).toEqual(["Follow the system", "Light", "Dark", "Night (dark)", "Sepia (light)"]);
    const withMissing = themeChoices(list, "gone.css");
    expect(withMissing[withMissing.length - 1]).toEqual({ value: "vault:gone.css", label: "gone.css (missing)", group: "vault" });
  });
});
