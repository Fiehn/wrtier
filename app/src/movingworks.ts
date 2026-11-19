// Moving Works and folders: dragged onto a folder in the Vault browser, or with Move to… in
// their menu. Where something can go is worked out in vaultmoves.ts; this is the dragging, the
// shading of where it would land, and showing where it went.

import { folderOf, foldersOf } from "./tree";
import { destinations, whereTo, type Movable, type Taken } from "./vaultmoves";
import { showMenu, type MenuItem } from "./menu";
import type { VaultBrowser } from "./vaultbrowser";
import { $, act, showInfo } from "./ui";

/** What a folder dragged in the Vault browser carries: its path. */
export const FOLDER_DRAG = "application/x-wrtier-folder";

/** The folder a drop goes into, from the row it's over (see `MovingWorks.dropFolder`). */
export type DropRow = { work: string } | { folder: string } | null;

/**
 * The folder a drop on `row` goes into: a folder's row (or anything inside it) is that folder,
 * a Work's row is its folder, and the space around the list (null) is the top of the Vault.
 */
export function dropFolder(row: DropRow): string {
  if (row && "work" in row) return folderOf(row.work);
  return row?.folder ?? "";
}

/** What the drag says when it can't go somewhere: once each time the reason changes, not on every move of the mouse. */
export class RefusedLine {
  private said: string | null = null;

  /** The line to show now, if any. */
  next(refused: string | null): string | null {
    const say = refused !== null && refused !== this.said ? refused : null;
    this.said = refused;
    return say;
  }

  reset() {
    this.said = null;
  }
}

export type MovingDeps = {
  browser: VaultBrowser;
  findInVault: { refresh(): Promise<void> };
};

export class MovingWorks {
  /**
   * What's being dragged to move, while it is: kept a moment past the drag's end, since
   * WebKitGTK reports the end before the drop (see dragging.ts).
   */
  private moving: Movable | null = null;
  private forgetMoving: number | undefined;
  /** A closed folder dragged over opens after a moment, to reach the folders in it. */
  private springOpen: { path: string; timer: number } | null = null;
  private refused = new RefusedLine();

  constructor(private deps: MovingDeps) {
    const vaultSide = $("sidebar-vault");
    vaultSide.addEventListener("dragover", (e) => {
      if (!this.moving || vaultSide.classList.contains("finding")) return;
      const into = this.dropFolderAt(e.target);
      const result = whereTo(this.moving, into, this.taken());
      this.springOpenSoon(e.target);
      e.preventDefault(); // it's ours, wherever it is over the Vault browser
      // Where it can't go, it can't be dropped (the cursor says so), and the info line says why.
      if (e.dataTransfer) e.dataTransfer.dropEffect = "to" in result ? "move" : "none";
      this.showDropTarget("to" in result ? into : null);
      const say = this.refused.next("refused" in result ? result.refused : null);
      if (say) showInfo(say);
    });
    vaultSide.addEventListener("dragleave", (e) => {
      // Out of the Vault browser altogether (not just from one row to the next).
      if (!(e.relatedTarget instanceof Node && vaultSide.contains(e.relatedTarget))) {
        this.showDropTarget(null);
        this.springOpenSoon(null);
      }
    });
    vaultSide.addEventListener("drop", (e) => {
      const item = this.moving;
      if (!item || vaultSide.classList.contains("finding")) return;
      e.preventDefault();
      this.moving = null;
      this.showDropTarget(null);
      this.springOpenSoon(null);
      this.refused.reset();
      const result = whereTo(item, this.dropFolderAt(e.target), this.taken());
      if ("to" in result) act(() => this.moveTo(item, result.to));
    });
  }

  /** A Work's or folder's drag started. */
  start(item: Movable) {
    window.clearTimeout(this.forgetMoving);
    this.moving = item;
  }

  /** The drag ended (maybe before its drop: see `moving`). */
  end() {
    window.clearTimeout(this.forgetMoving);
    this.forgetMoving = window.setTimeout(() => (this.moving = null), 1000);
    this.showDropTarget(null);
    this.springOpenSoon(null);
    this.refused.reset();
  }

  private taken(): Taken {
    const { browser } = this.deps;
    return { works: browser.entries.map((e) => e.id), folders: browser.folders };
  }

  /** The folder a drop at `target` goes into (see `dropFolder`). */
  private dropFolderAt(target: EventTarget | null): string {
    const el = target instanceof Element ? target : null;
    const work = el?.closest<HTMLElement>("#works li.work-entry");
    if (work) return dropFolder({ work: work.dataset.id ?? "" });
    const folder = el?.closest<HTMLElement>("#works li.folder")?.dataset.path;
    return dropFolder(folder === undefined ? null : { folder });
  }

  /** Shades where a drop would go: a folder, or (for "") the whole list. */
  private showDropTarget(into: string | null) {
    for (const el of document.querySelectorAll(".drop-target")) el.classList.remove("drop-target");
    if (into === null) return;
    const el = into === "" ? $("works") : document.querySelector(`#works li.folder[data-path="${CSS.escape(into)}"]`);
    el?.classList.add("drop-target");
  }

  /** Opens a closed folder whose row has been dragged over for a moment. */
  private springOpenSoon(target: EventTarget | null) {
    const row = target instanceof Element ? target.closest<HTMLElement>("#works .folder-toggle") : null;
    const folderEl = row?.parentElement ?? null;
    const path = folderEl && !folderEl.classList.contains("expanded") ? folderEl.dataset.path ?? null : null;
    if (this.springOpen?.path === path) return;
    if (this.springOpen) window.clearTimeout(this.springOpen.timer);
    this.springOpen = path === null ? null : {
      path,
      timer: window.setTimeout(() => {
        this.springOpen = null;
        if (!folderEl!.classList.contains("expanded")) row!.click();
      }, 750),
    };
  }

  /** Moves a Work or folder to `to`, then shows where it went: its folder opened, its row flashing. */
  private async moveTo(item: Movable, to: string) {
    const { browser, findInVault } = this.deps;
    const moved = item.kind === "work" ? await browser.moveWork(item.id, to) : await browser.moveFolder(item.path, to);
    browser.openFoldersOf(foldersOf(moved));
    await browser.refresh();
    void findInVault.refresh();
    const row = item.kind === "work"
      ? document.querySelector(`#works li.work-entry[data-id="${CSS.escape(moved)}"]`)
      : document.querySelector(`#works li.folder[data-path="${CSS.escape(moved)}"]`);
    if (!row) return;
    row.scrollIntoView({ block: "nearest" });
    row.classList.add("just-moved");
    window.setTimeout(() => row.classList.remove("just-moved"), 1600);
  }

  /** Move to…: the folders it can go to (no Hidden ones), "Top of the Vault" first. None if there are none. */
  menuItem(item: Movable, at: { x: number; y: number }): MenuItem[] {
    const choices = destinations(item, this.deps.browser.shownFolders());
    if (choices.length === 0) return [];
    return [{
      label: "Move to…",
      run: () => showMenu(at.x, at.y, choices.map((f) => ({
        label: f === "" ? "Top of the Vault" : f,
        run: () => act(async () => {
          const result = whereTo(item, f, this.taken());
          if ("refused" in result) showInfo(result.refused);
          else if ("to" in result) await this.moveTo(item, result.to);
        }),
      }))),
    }];
  }
}
