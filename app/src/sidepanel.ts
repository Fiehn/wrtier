// The panel beside the editor: the Page preview and the Comments (the History joins in Phase 5),
// one tab at a time. It's shown and hidden as a whole (Ctrl+Shift+P). Which tab is open and
// whether the panel shows are Device state, kept in this Device's local storage.

export type PanelTab = "pages" | "comments";
const TABS: PanelTab[] = ["pages", "comments"];

export type PanelState = { shown: boolean; tab: PanelTab };

/**
 * What this Device remembered. Before the panel had tabs, only whether the Page preview was
 * open was kept (under "preview"); that still counts.
 */
export function readPanelState(get: (key: string) => string | null): PanelState {
  const shown = (get("panel") ?? get("preview")) === "open";
  const tab = get("panel-tab");
  return { shown, tab: TABS.includes(tab as PanelTab) ? (tab as PanelTab) : "pages" };
}

export class SidePanel {
  shown = false;
  tab: PanelTab = "pages";

  /**
   * `changed` runs after every change, with whether the Page preview is in view: it only
   * compiles then.
   */
  constructor(
    private readonly root: HTMLElement,
    private readonly tabs: HTMLElement[],
    private readonly panes: Record<PanelTab, HTMLElement>,
    private readonly toggleButton: HTMLElement,
    private readonly changed: (pagesInView: boolean) => void,
  ) {
    for (const button of tabs) {
      button.addEventListener("click", () => this.setTab(button.dataset.tab as PanelTab));
      // Left and right arrows move between the tabs, as in any tab list.
      button.addEventListener("keydown", (e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        const i = TABS.indexOf(this.tab);
        const next = TABS[(i + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length];
        this.setTab(next);
        this.tabs.find((t) => t.dataset.tab === next)?.focus();
      });
    }
    let remembered: PanelState = { shown: false, tab: "pages" };
    try {
      remembered = readPanelState((key) => localStorage.getItem(key));
    } catch {
      // nothing remembered: hidden, on the pages
    }
    this.shown = remembered.shown;
    this.tab = remembered.tab;
    this.apply();
  }

  /** The Page preview is in view: the panel is shown with its Pages tab. */
  get pagesInView(): boolean {
    return this.shown && this.tab === "pages";
  }

  setShown(shown: boolean) {
    this.shown = shown;
    this.apply();
  }

  toggle() {
    this.setShown(!this.shown);
  }

  /** Opens a tab (and the panel, if it was hidden). */
  setTab(tab: PanelTab) {
    this.tab = tab;
    this.shown = true;
    this.apply();
  }

  private apply() {
    this.root.hidden = !this.shown;
    for (const button of this.tabs) {
      const selected = button.dataset.tab === this.tab;
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected ? 0 : -1;
    }
    for (const tab of TABS) this.panes[tab].hidden = tab !== this.tab;
    this.toggleButton.textContent = this.shown ? "Hide panel" : "Show panel";
    try {
      localStorage.setItem("panel", this.shown ? "open" : "closed");
      localStorage.setItem("panel-tab", this.tab);
    } catch {
      // not remembered, then: no harm done
    }
    this.changed(this.pagesInView);
  }
}
