// The Images tab (ADR 0021): the Work in front's Images, each with its picture, its size, where
// the text uses it (or that it doesn't) and its Past pictures (a list that opens under it, where
// one is used again or deleted for good), where it's replaced, renamed or deleted; then the Images of the Works it includes, to look at only (they're changed in their
// own Work). Offered only while there's an Image to list. It also hands the editor the pictures
// its Formatted view shows.

import { invoke } from "@tauri-apps/api/core";
import type { Picture, Pictures } from "./images";

/** A Past picture of an Image (vault/images.rs `PastPicture`). */
export type PastPicture = { hash: string; givenMs: number; size: number };

/** One Image (vault/images.rs `ImageRow`). */
export type ImageRow = {
  name: string;
  kind: string;
  size: number;
  hash: string;
  /** Where the text names it (UTF-16). */
  uses: number[];
  past: number;
  pastSize: number;
  /** Its Past pictures, newest first. */
  pastPictures: PastPicture[];
  standIn: boolean;
};

export type IncludedImages = { work: string; label: string; images: ImageRow[] };

export type ImagesView = { own: ImageRow[]; included: IncludedImages[]; pastFolder: string };

export type ImagesHooks = {
  /** The Work in front, if it's a Work. */
  work(): string | null;
  /** Puts the cursor at `at` in the Work's text and brings it into view. */
  go(work: string, at: number): void;
  insert(work: string): void;
  replace(work: string, image: ImageRow): void;
  rename(work: string, image: ImageRow): void;
  remove(work: string, image: ImageRow): void;
  /** Gives the Image one of its Past pictures again. */
  usePast(work: string, image: ImageRow, past: PastPicture): void;
  /** Deletes some of the Image's Past pictures, for good (asking first). */
  deletePast(work: string, image: ImageRow, past: PastPicture[]): void;
  openPast(work: string): void;
  openWork(work: string): void;
  /** The pictures for the Work's editor. */
  pictures(work: string, pictures: Pictures): void;
  /** Whether the Work in front has an Image to list (`now`: another Work just came to the front). */
  available(has: boolean, now: boolean): void;
  error(message: string): void;
};

const SETTLE_MS = 500;

const MIME: Record<string, string> = { PNG: "image/png", JPEG: "image/jpeg", GIF: "image/gif", WebP: "image/webp", SVG: "image/svg+xml" };

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
}

/** A size as the Author reads it: "820 KB", "12.4 MB". */
export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export class ImagesPanel {
  private visible = false;
  private timer: number | undefined;
  private shownFor: string | null = null;
  /** Asked of Rust last, so an older answer arriving late is dropped. */
  private asking = 0;
  /** Each picture's address for the page, by its hash: read once. */
  private readonly urls = new Map<string, Promise<string | null>>();
  /** Which use of each Image "Go to" went to last, to go on to the next. */
  private readonly next = new Map<string, number>();
  /** The Images whose Past pictures are listed (open), by name. */
  private readonly opened = new Set<string>();

  constructor(private readonly list: HTMLElement, private readonly hooks: ImagesHooks) {}

  setVisible(visible: boolean) {
    this.visible = visible;
    if (visible) void this.refresh();
  }

  /** The Work in front changed (or another came to the front). */
  workChanged() {
    clearTimeout(this.timer);
    void this.refresh();
  }

  /** Something in a Work may have changed its Images (its History grew: an edit, undo, a sync).
   *  `now`: the tab itself changed them, so it's asked again at once, not after the typing settles. */
  changed(id: string, now = false) {
    if (id !== this.hooks.work()) return;
    clearTimeout(this.timer);
    if (now) void this.refresh();
    else this.timer = window.setTimeout(() => void this.refresh(), SETTLE_MS);
  }

  /** A picture's address for the page; null if it can't be shown in one (a PDF). */
  private url(work: string, row: ImageRow, included: string | null): Promise<string | null> {
    const mime = MIME[row.kind];
    if (!mime) return Promise.resolve(null);
    let url = this.urls.get(row.hash);
    if (!url) {
      url = invoke<ArrayBuffer>("image_picture", { id: work, name: row.name, work: included })
        .then((bytes) => URL.createObjectURL(new Blob([bytes], { type: mime })))
        .catch(() => {
          this.urls.delete(row.hash);
          return null;
        });
      this.urls.set(row.hash, url);
    }
    return url;
  }

  /** A Past picture's address for the page (read when its list opens); null for a PDF. */
  private pastUrl(work: string, kind: string, hash: string): Promise<string | null> {
    const mime = MIME[kind];
    if (!mime) return Promise.resolve(null);
    let url = this.urls.get(hash);
    if (!url) {
      url = invoke<ArrayBuffer>("past_picture", { id: work, hash })
        .then((bytes) => URL.createObjectURL(new Blob([bytes], { type: mime })))
        .catch(() => {
          this.urls.delete(hash);
          return null;
        });
      this.urls.set(hash, url);
    }
    return url;
  }

  async refresh() {
    const work = this.hooks.work();
    const now = work !== this.shownFor;
    this.shownFor = work;
    const asked = ++this.asking;
    if (!work) {
      this.hooks.available(false, now);
      this.draw(null, null, new Map());
      return;
    }
    let view: ImagesView | null;
    try {
      view = await invoke<ImagesView | null>("get_images", { id: work });
    } catch {
      return; // not this time (the Work is closing, say): asked again on the next change
    }
    if (asked !== this.asking || !view) return;
    const has = view.own.length > 0 || view.included.some((g) => g.images.length > 0);
    this.hooks.available(has, now);
    const urls = new Map<string, string | null>();
    await Promise.all([
      ...view.own.map(async (row) => urls.set(row.hash, await this.url(work, row, null))),
      ...view.included.flatMap((g) => g.images.map(async (row) => urls.set(row.hash, await this.url(work, row, g.work)))),
    ]);
    if (asked !== this.asking) return;
    const byName = new Map<string, Picture>();
    for (const row of view.own) byName.set(row.name.toLowerCase(), { name: row.name, url: urls.get(row.hash) ?? null, kind: row.kind, standIn: row.standIn });
    this.hooks.pictures(work, { known: true, byName });
    this.draw(work, view, urls);
  }

  private draw(work: string | null, view: ImagesView | null, urls: Map<string, string | null>) {
    if (!this.visible) return;
    this.list.replaceChildren();
    if (!work || !view) return;
    const insert = el("button", "link", "Insert Image…");
    insert.type = "button";
    insert.title = "Add a picture file as an Image, at the cursor";
    insert.addEventListener("click", () => this.hooks.insert(work));
    if (view.own.length === 0 && view.included.length === 0) {
      this.list.append(el("p", "muted", "No Images. Add a picture with Insert Image, or drag a picture file onto the text."), insert);
      return;
    }
    const header = el("div", "images-header");
    header.append(insert);
    if (view.own.some((r) => r.past > 0)) {
      const past = el("button", "link", "Past pictures…");
      past.type = "button";
      past.title = `Open the folder of pictures these Images had before (${view.pastFolder}). Delete any you don't need: a Milestone that had one then shows the current picture instead, and says so.`;
      past.addEventListener("click", () => this.hooks.openPast(work));
      header.append(past);
    }
    this.list.append(header);
    if (view.own.length > 0) {
      if (view.included.length > 0) this.list.append(el("h3", "", "In this Work"));
      const ul = el("ul", "image-list");
      for (const row of view.own) ul.append(this.rowOf(work, row, urls.get(row.hash) ?? null, null));
      this.list.append(ul);
    }
    for (const group of view.included) {
      const h = el("h3", "images-included");
      h.append(document.createTextNode(`In ${group.label}`));
      const open = el("button", "link", "Open");
      open.type = "button";
      open.title = `Its Images are changed in ${group.label} itself`;
      open.addEventListener("click", () => this.hooks.openWork(group.work));
      h.append(open);
      this.list.append(h);
      const ul = el("ul", "image-list included");
      for (const row of group.images) ul.append(this.rowOf(work, row, urls.get(row.hash) ?? null, group.label));
      this.list.append(ul);
    }
  }

  private rowOf(work: string, row: ImageRow, url: string | null, from: string | null): HTMLElement {
    const li = el("li", "image-row");
    const thumb = el("div", "image-thumb");
    if (url) {
      const img = el("img");
      img.src = url;
      img.alt = "";
      img.draggable = false;
      thumb.append(img);
    } else {
      thumb.append(el("span", "muted", row.kind));
    }
    const about = el("div", "image-about");
    about.append(el("strong", "image-title", row.name));
    const facts = [row.kind, sizeText(row.size)];
    if (!from) facts.push(row.uses.length === 0 ? "not used" : row.uses.length === 1 ? "used once" : `used ${row.uses.length} times`);
    // Another Work's Past pictures are only counted: they're its own to use or delete.
    if (from && row.past > 0) facts.push(`${row.past} Past ${row.past === 1 ? "picture" : "pictures"} (${sizeText(row.pastSize)})`);
    about.append(el("span", `image-facts${!from && row.uses.length === 0 ? " unused" : ""}`, facts.join(" · ")));
    if (row.standIn) about.append(el("span", "image-warning", "Its picture was deleted from the Past pictures: it shows the one on hand."));
    if (!from) {
      const actions = el("div", "image-actions");
      const button = (label: string, title: string, run: () => void) => {
        const b = el("button", "link", label);
        b.type = "button";
        b.title = title;
        b.addEventListener("click", run);
        actions.append(b);
      };
      if (row.uses.length > 0) {
        button("Go to", "Where the text uses it (again for the next place)", () => {
          const i = ((this.next.get(row.name) ?? -1) + 1) % row.uses.length;
          this.next.set(row.name, i);
          this.hooks.go(work, row.uses[i]);
        });
      }
      button("Replace…", "Give it another picture: every place that uses it shows the new one", () => this.hooks.replace(work, row));
      button("Rename…", "Rename it, and every place in the text that uses it, as one action", () => this.hooks.rename(work, row));
      button("Delete", "Delete it: its picture is kept as a Past picture, and undo brings it back", () => this.hooks.remove(work, row));
      about.append(actions);
      if (row.pastPictures.length > 0) about.append(this.pastOf(work, row));
    }
    li.append(thumb, about);
    return li;
  }

  /** The Image's Past pictures: a button that opens their list, and the list while it's open. */
  private pastOf(work: string, row: ImageRow): HTMLElement {
    const box = el("div", "image-past");
    const open = this.opened.has(row.name);
    const n = row.pastPictures.length;
    const toggle = el("button", "image-past-toggle", `${open ? "▾" : "▸"} ${n} Past ${n === 1 ? "picture" : "pictures"} (${sizeText(row.pastSize)})`);
    toggle.type = "button";
    toggle.setAttribute("aria-expanded", String(open));
    toggle.title = "The pictures it had before: use one again, or delete those you don't need";
    toggle.addEventListener("click", () => {
      if (this.opened.has(row.name)) this.opened.delete(row.name);
      else this.opened.add(row.name);
      box.replaceWith(this.pastOf(work, row));
    });
    box.append(toggle);
    if (!open) return box;
    const list = el("ul", "image-past-list");
    for (const past of row.pastPictures) {
      const li = el("li", "image-past-row");
      li.dataset.hash = past.hash;
      const thumb = el("div", "image-thumb small");
      thumb.append(el("span", "muted", row.kind));
      void this.pastUrl(work, row.kind, past.hash).then((url) => {
        if (!url) return;
        const img = el("img");
        img.src = url;
        img.alt = "";
        img.draggable = false;
        thumb.replaceChildren(img);
      });
      const about = el("div", "image-about");
      const date = new Date(past.givenMs).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
      about.append(el("span", "image-facts", `Added ${date} · ${sizeText(past.size)}`));
      const actions = el("div", "image-actions");
      const use = el("button", "link", "Use this one");
      use.type = "button";
      use.title = "Give the Image this picture again: the one it has now becomes a Past picture";
      use.addEventListener("click", () => this.hooks.usePast(work, row, past));
      const remove = el("button", "link", "Delete");
      remove.type = "button";
      remove.title = "Delete this picture for good, on every Device";
      remove.addEventListener("click", () => this.hooks.deletePast(work, row, [past]));
      actions.append(use, remove);
      about.append(actions);
      li.append(thumb, about);
      list.append(li);
    }
    box.append(list);
    if (n > 1) {
      const all = el("button", "link", `Delete all ${n}`);
      all.type = "button";
      all.title = "Delete every Past picture of this Image, for good, on every Device";
      all.addEventListener("click", () => this.hooks.deletePast(work, row, row.pastPictures));
      box.append(all);
    }
    return box;
  }
}
