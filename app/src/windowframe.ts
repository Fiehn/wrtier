// The window's frame (v3 plan, "The top bar"). The window opens without the system's frame
// (tauri.conf.json), unless this Device chose the native one in Settings (Device state, read by
// the Rust core when the window opens). Without it, the app is its own title bar, as Obsidian
// is: the top row of each column (the Vault bar, the tab row, the Reading header, the side
// panel's tabs) is part of it, and anything in those rows that isn't a button moves the window,
// a double click maximizing it. The window's buttons sit at its top-right corner, and whichever
// row reaches that corner leaves room for them. On the phone layout, where each screen has a
// different top row, they get a strip of their own across the top instead.
//
// While a dialog is open the page under it is inert, title bar included: Escape or a click
// outside closes the dialog first.

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onAndroid } from "./android";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

/** The rows that are part of the title bar. */
const TOP_ROWS = "#sidebar-top, #tabrow, .reading-header, .panel-tabs, #window-controls";

/** The rows that can reach the window's top-right corner, and must leave room there. */
const CORNER_ROWS = "#tabrow, .reading-header, .panel-tabs";

/** What can be clicked or typed in, in a top row: everything else there moves the window. */
const CONTROLS = "button, input, select, textarea, a, label, summary, [role='tab'], .tab";

/** The window's title: the tab in front, the Vault, and the app (as in the taskbar and Alt+Tab). */
export function windowTitle(tab: string | null, vault: string | null): string {
  return [tab, vault, "wrtier"].filter((part) => part).join(" – ");
}

/** Which of the rows (as their boxes on screen) reaches furthest right; -1 if none is shown. */
export function rightmost(boxes: { right: number; width: number }[]): number {
  let at = -1;
  boxes.forEach((box, i) => {
    if (box.width > 0 && (at === -1 || box.right >= boxes[at].right)) at = i;
  });
  return at;
}

export class WindowFrame {
  private readonly window = onAndroid ? null : getCurrentWindow();
  private lastTitle = "";

  /** `custom`: the app draws the frame (the window has no system frame). */
  private constructor(readonly custom: boolean) {
    if (!custom || !this.window) return;
    const window = this.window;
    document.body.classList.add("custom-frame");
    $("window-controls").hidden = false;
    $("window-minimize").addEventListener("click", () => void window.minimize());
    $("window-maximize").addEventListener("click", () => void window.toggleMaximize());
    $("window-close").addEventListener("click", () => void window.close());
    document.addEventListener("mousedown", (e) => this.pressed(e));
    void window.onResized(() => void this.showMaximized());
    void this.showMaximized();
    // A panel shown or hidden, a narrower window, the phone layout: the row at the corner changes.
    const observer = new ResizeObserver(() => this.makeRoom());
    for (const id of ["sidebar", "reading-panel", "side-panel"]) observer.observe($(id));
    observer.observe(document.querySelector("main")!);
  }

  /** Has this window the app's frame? Never on Android, which has no window to frame. */
  static async start(): Promise<WindowFrame> {
    if (onAndroid) return new WindowFrame(false);
    const decorated = await getCurrentWindow().isDecorated().catch(() => true);
    return new WindowFrame(!decorated);
  }

  /** Shows `tab` (the tab in front's name) and `vault` (the open Vault's name) as the window's title. */
  title(tab: string | null, vault: string | null) {
    const title = windowTitle(tab, vault);
    if (title === this.lastTitle) return;
    this.lastTitle = title;
    document.title = title;
    void this.window?.setTitle(title).catch(() => {});
  }

  /** Settings → App: which frame the window gets from the next start. */
  showSetting(canChoose: boolean, native: boolean) {
    $("frame-setting").hidden = !canChoose;
    if (!canChoose) return;
    const select = $<HTMLSelectElement>("window-frame");
    select.value = native ? "native" : "custom";
    const restart = () => ($("frame-restart").hidden = (select.value === "custom") === this.custom);
    restart();
    select.addEventListener("change", () => {
      restart();
      invoke("set_window_frame", { native: select.value === "native" }).catch(() => {
        select.value = native ? "native" : "custom";
        restart();
      });
    });
  }

  /** A press in a top row, not on anything in it, moves the window; a double click maximizes it. */
  private pressed(e: MouseEvent) {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    if (!e.target.closest(TOP_ROWS) || e.target.closest(CONTROLS)) return;
    e.preventDefault();
    if (e.detail === 2) void this.window!.toggleMaximize();
    else void this.window!.startDragging();
  }

  /** Maximized, the middle button restores the window: its symbol and name say so. */
  private async showMaximized() {
    const maximized = await this.window!.isMaximized().catch(() => false);
    document.body.classList.toggle("window-maximized", maximized);
    const button = $("window-maximize");
    button.title = maximized ? "Restore" : "Maximize";
    button.setAttribute("aria-label", button.title);
  }

  /** The row at the window's top-right corner leaves room for its buttons. */
  private makeRoom() {
    const rows = [...document.querySelectorAll<HTMLElement>(CORNER_ROWS)];
    const at = rightmost(rows.map((row) => row.getBoundingClientRect()));
    rows.forEach((row, i) => row.classList.toggle("frame-room", i === at));
  }
}
