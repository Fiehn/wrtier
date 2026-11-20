// Settings: its tabs (Editor, Reviewers, Vault, Keyboard shortcuts, App), and the settings in the
// Editor and Vault tabs. A setting is the Vault's, the same on every Device, unless its "This
// device only" box is ticked: then this Device keeps its own value.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { bindingsFrom, setBindings } from "./keys";
import { reviewersTab } from "./reviewers";
import { shortcutsTab } from "./shortcutsdialog";
import { fillNewWorkMods, fillVaultMods } from "./modsdialog";
import { fillDictionaries } from "./spellmod";
import { ThemeApplier, themeChoices, VAULT_PREFIX } from "./themes";
import type { ModsState } from "./mods";
import type { Tabs } from "./tabs";
import type { ReadingPanel } from "./reading";
import type { PagePreview } from "./preview";
import type { VaultBrowser } from "./vaultbrowser";
import { openLicenses } from "./licensesdialog";
import { $, act, ask, confirmAsk, small, SYMBOLS, symbolButton } from "./ui";

// The same shape as the Rust side's (vault/settings.rs).
export type Settings = {
  /** "system", "light", "dark", or "vault:<file>" for a theme in the Vault's `themes/`. */
  theme: string;
  fontSize: number;
  themeHere: boolean;
  fontSizeHere: boolean;
  lineNumbers: boolean;
  lineNumbersHere: boolean;
  highlightLine: boolean;
  highlightLineHere: boolean;
  previewFollows: boolean;
  previewFollowsHere: boolean;
  commentHighlights: boolean;
  commentHighlightsHere: boolean;
  /** Shortcuts changed from the defaults: command → key, or "none". */
  shortcuts: Record<string, string>;
  shortcutsHere: boolean;
  /** This Device keeps its own list of Hidden folders. */
  hiddenFoldersHere: boolean;
  /** What the other Devices use, for the settings this Device overrides. */
  shared: { theme: string; fontSize: number; lineNumbers: boolean; highlightLine: boolean; previewFollows: boolean; commentHighlights: boolean };
};

/** The Editor tab's single settings: the name Rust knows each by, and its elements' id. */
const SINGLE_SETTINGS = [
  { name: "theme", id: "theme" },
  { name: "fontSize", id: "font-size" },
  { name: "lineNumbers", id: "line-numbers" },
  { name: "highlightLine", id: "highlight-line" },
  { name: "previewFollows", id: "preview-follows" },
  { name: "commentHighlights", id: "comment-highlights" },
] as const;

type SingleSetting = (typeof SINGLE_SETTINGS)[number]["name"];

/** How a shared value reads under a setting this Device overrides ("Other devices: 17"). `themeName`: a theme's name as the picker shows it. */
export function sharedValueText(name: SingleSetting, shared: Settings["shared"], themeName: (value: string) => string | undefined): string {
  const value = shared[name];
  if (typeof value === "boolean") return value ? "on" : "off";
  if (name === "theme") return themeName(String(value)) ?? String(value).replace(VAULT_PREFIX, "");
  return String(value);
}

export type SettingsTab = "editor" | "reviewers" | "vault" | "shortcuts" | "app";
const SETTINGS_TABS: SettingsTab[] = ["editor", "reviewers", "vault", "shortcuts", "app"];

/** The tabs Settings has: with no Vault open, only App. */
export const settingsTabs = (vaultOpen: boolean): SettingsTab[] => (vaultOpen ? SETTINGS_TABS : ["app"]);

/** The tab an arrow key moves to from `tab` (round the ends). */
export function tabAfter(tab: SettingsTab, key: "ArrowLeft" | "ArrowRight"): SettingsTab {
  const i = SETTINGS_TABS.indexOf(tab);
  return SETTINGS_TABS[(i + (key === "ArrowRight" ? 1 : SETTINGS_TABS.length - 1)) % SETTINGS_TABS.length];
}

export type SettingsDeps = {
  tabs: Tabs;
  readingPanel: ReadingPanel;
  preview: PagePreview;
  browser: VaultBrowser;
  findInVault: { refresh(): Promise<void> };
};

export class SettingsDialog {
  /** The theme setting as last applied, so a change to the Vault's themes can apply it again. */
  private themeSetting = "system";
  private themes = new ThemeApplier(() => this.showThemes());
  private dialog = $<HTMLDialogElement>("settings");
  /** The tab Settings opens on: the one last shown in this run of the app. */
  private tab: SettingsTab = "editor";
  // The highlights take any new names and colours.
  private reviewers = reviewersTab(confirmAsk, () => this.deps.tabs.refreshHighlights());
  private shortcuts = shortcutsTab<Settings>(this.dialog, { apply: (settings) => this.apply(settings), confirm: confirmAsk });

  constructor(private deps: SettingsDeps) {
    // Ticking "This device only" keeps the value in use, as this Device's own; unticking goes
    // back to the value the other Devices use, and never changes theirs.
    for (const { name, id } of SINGLE_SETTINGS) {
      $(`${id}-here`).addEventListener("change", () => act(async () => {
        this.apply(await invoke<Settings>("set_setting_here", { name, here: $<HTMLInputElement>(`${id}-here`).checked }));
      }));
    }
    const onOff = (name: string, box: string) => {
      $(box).addEventListener("change", () => this.set(name, $<HTMLInputElement>(box).checked ? "on" : "off", $<HTMLInputElement>(`${box}-here`).checked));
    };
    onOff("lineNumbers", "line-numbers");
    onOff("highlightLine", "highlight-line");
    onOff("previewFollows", "preview-follows");
    onOff("commentHighlights", "comment-highlights");
    $("theme").addEventListener("change", () => this.set("theme", $<HTMLSelectElement>("theme").value, $<HTMLInputElement>("theme-here").checked));
    // Importing copies a theme's file into the Vault's themes folder, then uses it.
    $("import-theme").addEventListener("click", () => act(() => this.importTheme()));
    // A theme was added, changed or removed (here, in another editor, or from another Device).
    void listen("themes-changed", () => void this.themes.apply(this.themeSetting));
    $("font-size").addEventListener("change", () => this.set("fontSize", $<HTMLInputElement>("font-size").value, $<HTMLInputElement>("font-size-here").checked));
    $("hidden-folders-here").addEventListener("change", () => act(async () => {
      this.apply(await invoke<Settings>("set_hidden_folders_here", { here: $<HTMLInputElement>("hidden-folders-here").checked }));
      await deps.browser.refresh();
      void deps.findInVault.refresh();
      this.fillHiddenFolders();
    }));
    for (const tab of SETTINGS_TABS) {
      const button = $(`settings-tab-${tab}`);
      button.addEventListener("click", () => this.showTab(tab));
      // Left and right arrows move between the tabs, as in any tab list.
      button.addEventListener("keydown", (e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        const next = tabAfter(tab, e.key);
        this.showTab(next);
        $(`settings-tab-${next}`).focus();
      });
    }
    $("open-settings").addEventListener("click", () => act(() => this.open()));
    $("open-licenses").addEventListener("click", () => act(openLicenses));
  }

  /** Shows the theme picker's choices and lines for the theme setting as it was last applied. */
  private showThemes() {
    const { list, resolved } = this.themes;
    const select = $<HTMLSelectElement>("theme");
    select.replaceChildren();
    const vaultGroup = document.createElement("optgroup");
    vaultGroup.label = "This Vault's themes";
    for (const choice of themeChoices(list, resolved.missing)) {
      const option = new Option(choice.label, choice.value);
      (choice.group === "vault" ? vaultGroup : select).append(option);
    }
    if (vaultGroup.children.length) select.append(vaultGroup);
    select.value = this.themeSetting;
    const missing = $("theme-missing");
    missing.hidden = !resolved.missing;
    missing.textContent = resolved.missing
      ? `The theme ${resolved.missing} isn't in this Vault's themes folder, so the app follows the system. Choose another theme, or import it again.`
      : "";
    const problems = $("theme-problems");
    problems.hidden = list.problems.length === 0;
    problems.textContent = list.problems.map((p) => `${p.file} in the themes folder isn't a theme: ${p.reason}`).join("\n");
  }

  /** Reads the Vault's settings and applies them. */
  async load() {
    this.apply(await invoke<Settings>("get_settings"));
  }

  /** Makes the window, the editors and Settings' boxes follow `settings`. */
  apply(settings: Settings) {
    const { tabs, readingPanel, preview } = this.deps;
    const root = document.documentElement;
    this.themeSetting = settings.theme;
    void this.themes.apply(settings.theme);
    root.style.setProperty("--editor-font-size", `${settings.fontSize}px`);
    $<HTMLInputElement>("font-size").value = String(settings.fontSize);
    $<HTMLInputElement>("line-numbers").checked = settings.lineNumbers;
    $<HTMLInputElement>("highlight-line").checked = settings.highlightLine;
    $<HTMLInputElement>("preview-follows").checked = settings.previewFollows;
    $<HTMLInputElement>("comment-highlights").checked = settings.commentHighlights;
    const themeName = (value: string) => [...$<HTMLSelectElement>("theme").options].find((o) => o.value === value)?.textContent ?? undefined;
    for (const { name, id } of SINGLE_SETTINGS) {
      const here = settings[`${name}Here`];
      $<HTMLInputElement>(`${id}-here`).checked = here;
      const other = $(`${id}-other`);
      other.hidden = !here;
      other.textContent = here ? `Other devices: ${sharedValueText(name, settings.shared, themeName)}` : "";
    }
    $<HTMLInputElement>("hidden-folders-here").checked = settings.hiddenFoldersHere;
    tabs.setOptions({ lineNumbers: settings.lineNumbers, highlightLine: settings.highlightLine });
    tabs.setCommentOptions({ ...tabs.commentOptions, highlights: settings.commentHighlights });
    readingPanel.setCommentHighlights(settings.commentHighlights);
    setBindings(bindingsFrom(settings.shortcuts));
    preview.setFollows(settings.previewFollows);
  }

  /** Sets a setting: for this Device only while its "This device only" box is ticked, else for every Device. */
  private set(name: string, value: string, here: boolean) {
    act(async () => this.apply(await invoke<Settings>("set_setting", { name, value, here })));
  }

  private async importTheme() {
    type Imported = { status: "imported"; file: string; name: string } | { status: "exists"; name: string };
    const path = await invoke<string | null>("pick_theme_file");
    if (path === null) return;
    let done = await invoke<Imported>("import_theme", { path, replace: false });
    if (done.status === "exists") {
      if (!(await confirmAsk(`This Vault already has a theme called “${done.name}”. Replace it with the one you picked?`, "Replace"))) return;
      done = await invoke<Imported>("import_theme", { path, replace: true });
    }
    if (done.status !== "imported") return;
    this.set("theme", VAULT_PREFIX + done.file, $<HTMLInputElement>("theme-here").checked);
  }

  /** Settings' list of Hidden folders, each with Unhide (or Remove, for one that's gone from the Vault). */
  private fillHiddenFolders() {
    const { browser } = this.deps;
    const list = $("hidden-folders");
    list.replaceChildren();
    if (browser.hiddenFolders.length === 0) {
      const none = document.createElement("li");
      none.className = "muted";
      none.textContent = "None. Right-click a folder in the Vault browser to hide it.";
      list.append(none);
    }
    for (const path of browser.hiddenFolders) {
      const item = document.createElement("li");
      const name = document.createElement("span");
      name.className = "hidden-folder-name";
      name.textContent = path;
      item.append(name);
      const unhide = async () => {
        await browser.setFolderHidden(path, false);
        this.fillHiddenFolders();
      };
      if (browser.folders.includes(path)) {
        item.append(symbolButton(`Unhide ${path}`, SYMBOLS.show, () => act(unhide), `Unhide: show ${path} in the Vault browser again`));
      } else {
        item.append(small("not in the Vault now"), symbolButton(`Take ${path} off the list`, SYMBOLS.takeOff, () => act(unhide), `Take ${path} off the list`));
      }
      list.append(item);
    }
  }

  private showTab(tab: SettingsTab, remember = true) {
    if (tab !== "shortcuts") this.shortcuts.hide();
    if (remember) this.tab = tab;
    for (const t of SETTINGS_TABS) {
      const button = $(`settings-tab-${t}`);
      const selected = t === tab;
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected ? 0 : -1;
      $(`settings-${t}`).hidden = !selected;
    }
  }

  /**
   * Opens Settings, at `tab` if given, else at the tab last shown. Each tab is filled in afresh,
   * since another Device may have changed what's in it.
   */
  async open(tab?: SettingsTab) {
    const { browser, findInVault } = this.deps;
    const available = settingsTabs(browser.isOpen);
    const appOnly = available.length === 1;
    this.dialog.classList.toggle("app-only", appOnly);
    $("settings-tabs").hidden = appOnly;
    if (browser.isOpen) {
      const settings = await invoke<Settings>("get_settings");
      this.apply(settings);
      await browser.refresh(); // and the Hidden folders with them
      this.fillHiddenFolders();
      await fillVaultMods($("vault-mods"), (state: ModsState) => {
        browser.vaultMods = state;
        act(async () => {
          await browser.refresh(); // the colours come or go
          void findInVault.refresh();
        });
      });
      await fillNewWorkMods($("new-work-mods"));
      await fillDictionaries($("dictionaries"));
      await this.reviewers.show();
      this.shortcuts.show(settings);
    }
    const wanted = tab ?? this.tab;
    // With only App, the tab to come back to stays as it was.
    this.showTab(available.includes(wanted) ? wanted : available[0], !appOnly);
    await ask(this.dialog);
    this.shortcuts.hide();
  }
}
