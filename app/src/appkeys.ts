// The app-wide keyboard shortcuts (the editor's are in each editor's keymap), and the buttons
// and lines that name a key, which say the current one.

import { isCommand, keyLabel, onBindingsChanged, withKey, type CommandId, type KeyEvent } from "./keys";
import { $, act } from "./ui";

/** What each app-wide command does; false: it doesn't apply right now (the key is left alone). */
export type AppCommands = Partial<Record<CommandId, () => boolean>>;

/** Runs the command `e` is the key of, if one applies now; which one ran, or null. */
export function runAppCommand(e: KeyEvent, commands: AppCommands): CommandId | null {
  for (const [command, run] of Object.entries(commands) as [CommandId, () => boolean][]) {
    if (isCommand(e, command) && run()) return command;
  }
  return null;
}

/** The line in an empty window, naming the Quick switcher's key if it has one. */
export const emptyLine = (switcherKey: string) =>
  `Open a Vault, then choose a Work on the left${switcherKey ? `, or press ${switcherKey}` : ""}.`;

/** What the shortcuts act on. */
export type AppKeysDeps = {
  tabs: {
    readonly active: string | null;
    readonly activeIsWork: boolean;
    readonly formatted: boolean;
    close(id: string): Promise<void>;
    keysChanged(): void;
  };
  vaultOpen(): boolean;
  sidebarHidden(): boolean;
  setSidebar(shown: boolean): void;
  openSwitcher(): Promise<void>;
  setFormatted(on: boolean): void;
  togglePanel(): void;
  editPreamble(id: string): Promise<void>;
  editMods(id: string): Promise<void>;
  openFind(replace: boolean): void;
  openFindInVault(): void;
  zoom: { zoomIn(): void; zoomOut(): void; reset(): void };
  /** Other things that name a key say the current one. */
  keysChanged(): void;
};

export class AppKeys {
  private commands: AppCommands;

  constructor(private deps: AppKeysDeps) {
    const { tabs } = deps;
    this.commands = {
      toggleWorks: () => {
        deps.setSidebar(deps.sidebarHidden());
        return true;
      },
      quickSwitcher: () => {
        void deps.openSwitcher();
        return true;
      },
      closeTab: () => {
        if (!tabs.active) return false;
        act(() => tabs.close(tabs.active!));
        return true;
      },
      toggleView: () => {
        if (!tabs.active) return false;
        deps.setFormatted(!tabs.formatted);
        return true;
      },
      togglePanel: () => {
        deps.togglePanel();
        return true;
      },
      preamble: () => {
        if (!tabs.activeIsWork || !tabs.active) return false;
        act(() => deps.editPreamble(tabs.active!));
        return true;
      },
      mods: () => {
        if (!tabs.activeIsWork || !tabs.active) return false;
        act(() => deps.editMods(tabs.active!));
        return true;
      },
      // Always the app's own, never the web engine's page search (which only sees the lines drawn).
      find: () => {
        deps.openFind(false);
        return true;
      },
      replace: () => {
        deps.openFind(true);
        return true;
      },
      findInVault: () => {
        if (!deps.vaultOpen()) return false;
        deps.openFindInVault();
        return true;
      },
      // Only while the Page preview has the focus: in the editor these keys do nothing.
      zoomIn: () => this.inPages(() => deps.zoom.zoomIn()),
      zoomInAlt: () => this.inPages(() => deps.zoom.zoomIn()),
      zoomOut: () => this.inPages(() => deps.zoom.zoomOut()),
      zoomReset: () => this.inPages(() => deps.zoom.reset()),
    };
    window.addEventListener("keydown", (e) => {
      if (runAppCommand(e, this.commands)) e.preventDefault();
    }, { capture: true });
    onBindingsChanged(() => this.showKeys());
    this.showKeys(); // the defaults, until a Vault's settings say otherwise
  }

  /** Runs a Page preview command if the focus is in the Page preview (its pages or its header). */
  private inPages(run: () => void): boolean {
    if (!$("preview").contains(document.activeElement)) return false;
    run();
    return true;
  }

  /** Everything that names a key says the current one. */
  showKeys() {
    $("open-switcher").title = withKey("Open a Work", "quickSwitcher");
    $("find-open").title = withKey("Find and replace", "find");
    $("open-switcher-key").textContent = keyLabel("quickSwitcher");
    $("open-switcher-key").hidden = !keyLabel("quickSwitcher");
    $("open-find-in-vault").title = withKey("Find some text in every Work", "findInVault");
    $("find-in-vault-key").textContent = keyLabel("findInVault");
    $("find-in-vault-key").hidden = !keyLabel("findInVault");
    $("view-mode").title = withKey("Switch between the Formatted and the Raw view", "toggleView");
    $("preamble-open").title = withKey("The Work's Looks, Styles, Story elements and settings", "preamble");
    $("mods-open").title = withKey("The Work's optional features, such as spellcheck", "mods");
    $("panel-toggle").title = withKey("The Page preview and the Comments", "togglePanel");
    $("panel-close").title = withKey("Hide the panel", "togglePanel");
    $("empty").textContent = emptyLine(keyLabel("quickSwitcher"));
    $("sidebar-toggle").title = withKey(this.deps.sidebarHidden() ? "Show the Works" : "Hide the Works", "toggleWorks");
    this.deps.tabs.keysChanged();
    this.deps.keysChanged();
  }
}
