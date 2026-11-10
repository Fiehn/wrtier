// The phone layout (v1 plan, "On the phone"): one screen at a time, switched with a bar at the
// bottom: Works · Write · Pages · Comments · History. Works is the sidebar, Write the editor, and
// the other three are the side panel's tabs, each a whole screen. Any narrow window gets it, so
// it can be tried at phone width on desktop and in the tests.
//
// The Chapter list (a Mod) opens as a sheet from a button above the text. The bar hides while
// the keyboard is open: on Android, MainActivity.kt sets `keyboard-open` on <html>.

import type { PanelTab, SidePanel } from "./sidepanel";

export type Screen = "works" | "write" | PanelTab;

/** Narrower than this is a phone. */
const PHONE_WIDTH = "(max-width: 700px)";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

export class PhoneLayout {
  screen: Screen = "works";
  private readonly query = matchMedia(PHONE_WIDTH);
  private readonly app = $("app");
  private readonly buttons = [...$("phone-bar").querySelectorAll<HTMLButtonElement>("button[data-screen]")];
  private readonly chapters = $<HTMLDetailsElement>("chapters");
  private readonly chaptersHome = this.chapters.parentElement!;
  private readonly sheet = $<HTMLDialogElement>("chapters-sheet");
  private readonly sheetButton = $<HTMLButtonElement>("chapters-open");
  /** The side panel as it was on the wide layout, to go back to. */
  private wide: { shown: boolean; tab: PanelTab } | null = null;

  constructor(private readonly panel: SidePanel) {
    for (const button of this.buttons) button.addEventListener("click", () => this.show(button.dataset.screen as Screen));
    this.sheetButton.addEventListener("click", () => this.sheet.showModal());
    // A Chapter tapped in the sheet goes there (chapterlist.ts), so the sheet closes.
    this.sheet.addEventListener("click", (e) => {
      if ((e.target as HTMLElement).closest("#chapters-list li")) this.sheet.close();
    });
    // The button shows only while the Work in front has the Chapter list here.
    new MutationObserver(() => this.updateSheetButton()).observe(this.chapters, { attributes: true, attributeFilter: ["hidden"] });
    this.query.addEventListener("change", () => this.apply());
    this.apply();
  }

  /** The phone layout is on. */
  get active(): boolean {
    return this.query.matches;
  }

  /** Shows a screen (on the phone layout; elsewhere it does nothing). */
  show(screen: Screen) {
    if (!this.active) return;
    this.screen = screen;
    this.render();
  }

  /** Something asked to go to text in the Work (a Comment, a snippet, a page, a Chapter). */
  toWrite() {
    this.show("write");
  }

  private apply() {
    const phone = this.active;
    document.body.classList.toggle("phone", phone);
    if (phone) {
      this.wide ??= { shown: this.panel.shown, tab: this.panel.tab };
      this.sheet.prepend(this.chapters);
      this.chapters.open = true;
      this.render();
    } else {
      delete this.app.dataset.screen;
      this.chaptersHome.append(this.chapters);
      if (this.wide) {
        this.panel.setTab(this.wide.tab);
        this.panel.setShown(this.wide.shown);
        this.wide = null;
      }
    }
    this.updateSheetButton();
  }

  private render() {
    this.app.dataset.screen = this.screen;
    for (const button of this.buttons) {
      if (button.dataset.screen === this.screen) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    }
    // The panel's tabs are screens here: the pages compile, and the History loads, only while showing.
    if (this.screen === "works" || this.screen === "write") this.panel.setShown(false);
    else this.panel.setTab(this.screen);
  }

  private updateSheetButton() {
    this.sheetButton.hidden = !this.active || this.chapters.hidden;
  }
}
