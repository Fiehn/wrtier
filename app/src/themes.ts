// Themes: light, dark, following the system, or a Vault theme from `themes/`.
//
// A Vault theme is a CSS file whose first line names it and says whether it's light or dark
// (themes.rs reads it). The app starts from its own light or dark colours (data-theme on the
// root) and adds the file after its own styles; the built-in colours are in a CSS layer, so the
// theme overrides them whatever its selectors. A theme named by the setting but not in the Vault
// (deleted, renamed, or not synced here yet) means following the system, and Settings says so.

import { invoke } from "@tauri-apps/api/core";

export type VaultTheme = { file: string; name: string; dark: boolean };
export type NotATheme = { file: string; reason: string };
export type ThemeList = { themes: VaultTheme[]; problems: NotATheme[] };

/** What a theme setting means with the Vault's themes as they are. */
export type Resolved = {
  /** The root's data-theme: the built-in colours to start from, or null to follow the system. */
  dataTheme: "light" | "dark" | null;
  /** The Vault theme's file to add on top, if any. */
  file: string | null;
  /** The setting names a Vault theme this Vault doesn't have. */
  missing: string | null;
};

export const VAULT_PREFIX = "vault:";

export function resolveTheme(setting: string, list: ThemeList): Resolved {
  if (setting === "light" || setting === "dark") return { dataTheme: setting, file: null, missing: null };
  if (setting.startsWith(VAULT_PREFIX)) {
    const file = setting.slice(VAULT_PREFIX.length);
    const theme = list.themes.find((t) => t.file === file);
    if (theme) return { dataTheme: theme.dark ? "dark" : "light", file, missing: null };
    return { dataTheme: null, file: null, missing: file };
  }
  return { dataTheme: null, file: null, missing: null };
}

/** The picker's choices: the built-in ones, then the Vault's themes by name. */
export function themeChoices(list: ThemeList, missing: string | null): { value: string; label: string; group: "app" | "vault" }[] {
  const choices: { value: string; label: string; group: "app" | "vault" }[] = [
    { value: "system", label: "Follow the system", group: "app" },
    { value: "light", label: "Light", group: "app" },
    { value: "dark", label: "Dark", group: "app" },
  ];
  for (const t of list.themes) choices.push({ value: VAULT_PREFIX + t.file, label: `${t.name} (${t.dark ? "dark" : "light"})`, group: "vault" });
  // Kept in the list, so the setting shows as it is until another theme is chosen.
  if (missing) choices.push({ value: VAULT_PREFIX + missing, label: `${missing} (missing)`, group: "vault" });
  return choices;
}

const STYLE_ID = "vault-theme";

/** Applies theme settings to the page. Only the newest request wins, if several overlap. */
export class ThemeApplier {
  private request = 0;
  list: ThemeList = { themes: [], problems: [] };
  resolved: Resolved = { dataTheme: null, file: null, missing: null };

  constructor(private readonly changed: () => void) {}

  /** Shows `setting`, reading the Vault's themes again (they may have changed anywhere). */
  async apply(setting: string) {
    const request = ++this.request;
    let list: ThemeList;
    try {
      list = await invoke<ThemeList>("list_themes");
    } catch {
      list = { themes: [], problems: [] }; // no Vault open: only the built-in themes
    }
    const resolved = resolveTheme(setting, list);
    let css = "";
    if (resolved.file) {
      try {
        css = await invoke<string>("read_theme", { file: resolved.file });
      } catch {
        // Gone between listing and reading: follow the system, as for a missing one.
        Object.assign(resolved, { dataTheme: null, file: null, missing: resolved.file });
      }
    }
    if (request !== this.request) return;
    this.list = list;
    this.resolved = resolved;
    const root = document.documentElement;
    if (resolved.dataTheme) root.dataset.theme = resolved.dataTheme;
    else delete root.dataset.theme;
    let style = document.getElementById(STYLE_ID);
    if (css) {
      if (!style) {
        style = document.createElement("style");
        style.id = STYLE_ID;
      }
      document.head.append(style); // last, after the app's own styles
      if (style.textContent !== css) style.textContent = css;
    } else {
      style?.remove();
    }
    this.changed();
  }
}
