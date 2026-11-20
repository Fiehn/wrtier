// The panel beside the editor: the Page preview, the Comments, the Marks, the Footnotes, the
// Images and the History, one tab at a time. It's shown and hidden as a whole (Ctrl+Shift+P).
// Which tab is open and whether the panel shows are Device state, kept in this Device's local
// storage.
//
// The Footnotes tab is offered only while the Work in front has a Footnote or an Annotation
// (CONTEXT.md). Open when its last one goes, it stays until another tab is picked, so writing a
// Footnote and deleting it again doesn't make it come and go. The Marks tab is offered while the
// Work in front has a Mod that makes Marks.

import { withKey } from "./keys";

export type PanelTab = "pages" | "comments" | "marks" | "footnotes" | "images" | "history";
const TABS: PanelTab[] = ["pages", "comments", "marks", "footnotes", "images", "history"];

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
  /** Tabs not offered just now (the Footnotes tab, in a Work without any). */
  private unavailable = new Set<PanelTab>(["marks", "footnotes", "images"]);

  /**
   * `changed` runs after every change, with whether the Page preview is in view (it only
   * compiles then) and the tab in view (null while the panel is hidden).
   */
  constructor(
    private readonly root: HTMLElement,
    private readonly tabs: HTMLElement[],
    private readonly panes: Record<PanelTab, HTMLElement>,
    private readonly toggleButton: HTMLElement,
    private readonly changed: (pagesInView: boolean, inView: PanelTab | null) => void,
  ) {
    for (const button of tabs) {
      button.addEventListener("click", () => this.setTab(button.dataset.tab as PanelTab));
      // Left and right arrows move between the tabs, as in any tab list.
      button.addEventListener("keydown", (e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        const offered = this.offered();
        const i = offered.indexOf(this.tab);
        const next = offered[(i + (e.key === "ArrowRight" ? 1 : offered.length - 1)) % offered.length];
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

  /** The tabs offered now, in order (the open one always among them). */
  offered(): PanelTab[] {
    return TABS.filter((t) => t === this.tab || !this.unavailable.has(t));
  }

  /**
   * Offers a tab, or stops offering it. A tab that's open stays while it's in view, unless
   * `now` (another Work came to the front): then the panel goes to its Pages.
   */
  setAvailable(tab: PanelTab, available: boolean, now = false) {
    const was = this.unavailable.has(tab);
    if (available) this.unavailable.delete(tab);
    else this.unavailable.add(tab);
    if (!available && this.tab === tab && (now || !this.shown)) {
      this.tab = "pages";
      this.apply();
      return;
    }
    if (was !== !available) this.showTabs();
  }

  /** Is this tab offered now? */
  isAvailable(tab: PanelTab): boolean {
    return !this.unavailable.has(tab);
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

  private showTabs() {
    for (const button of this.tabs) {
      const tab = button.dataset.tab as PanelTab;
      const selected = tab === this.tab;
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected ? 0 : -1;
      button.hidden = !selected && this.unavailable.has(tab);
    }
  }

  /** The tab row's panel symbol: says (with the key) what clicking it does now. */
  showToggle() {
    const what = this.shown ? "Hide the panel" : "Show the panel";
    this.toggleButton.setAttribute("aria-pressed", String(this.shown));
    this.toggleButton.setAttribute("aria-label", what);
    this.toggleButton.title = withKey(this.shown ? what : `${what}: the Page preview, the Comments and the History`, "togglePanel");
  }

  private apply() {
    this.root.hidden = !this.shown;
    this.showTabs();
    for (const tab of TABS) this.panes[tab].hidden = tab !== this.tab;
    this.showToggle();
    try {
      localStorage.setItem("panel", this.shown ? "open" : "closed");
      localStorage.setItem("panel-tab", this.tab);
    } catch {
      // not remembered, then: no harm done
    }
    this.changed(this.pagesInView, this.shown ? this.tab : null);
  }
}
