// The Page preview: the active Work's compiled pages, beside the editor.
//
// It compiles only while it's open, about 300 ms after the last edit, from the text as Rust
// has it (the open typing burst included). Each page comes as its own SVG, and only pages in
// view whose content changed are drawn again. When the Work doesn't compile, the last pages
// that did stay, faded, under a bar with the first error.

import { invoke } from "@tauri-apps/api/core";

// The same shapes as the Rust side's (typeset.rs).
type PageSize = { width: number; height: number };
type PagePicture = { hash: string; svg: string | null };
type Problem = { message: string; hints: string[]; from: number | null; to: number | null; line: number | null };
/** A spot on the pages: the page (from 0), and points from its top left. */
type Spot = { page: number; x: number; y: number };
type JumpTo = { kind: "text"; at: number } | { kind: "pages"; spot: Spot };
type Compiled = { pages: PageSize[]; stale: boolean; errors: Problem[]; warnings: Problem[]; deviceFonts: string[]; ms: number };

/** What the rest of the page does for the preview. */
export type PreviewHooks = {
  /** Resolves once everything typed in this Work has reached Rust. */
  settled(id: string): Promise<void>;
  /** Shows this stretch of the Work's text in the editor. */
  reveal(id: string, from: number, to: number): void;
  /** Where the cursor is in this Work (UTF-16), if it's open. */
  cursorOf(id: string): number | null;
  error(message: string): void;
};

/** How long after the last edit it compiles. */
const PAUSE_MS = 300;
/** How long after the cursor stops moving the preview follows it. */
const FOLLOW_MS = 120;
/** How close (in pixels) the cursor's spot may come to the preview's edges before it scrolls. */
const FOLLOW_MARGIN = 60;

/**
 * A page in the list. After each compile, a page in view asks Rust whether it changed (sending
 * the hash of the picture it has) and gets a new picture only if it did. Pages out of view ask
 * when they come into view.
 */
type Page = {
  el: HTMLElement;
  /** In points. */
  width: number;
  height: number;
  /** The hash of the picture shown. */
  drawn: string | null;
  url: string | null;
  /** The compile this page was last checked against. */
  checked: number;
  drawing: boolean;
};

export class PagePreview {
  open = false;
  /** Keep the cursor's spot in view (a Vault setting). */
  follows = true;
  private followTimer: number | undefined;
  /** Where the cursor is on the pages, marked with a thin line. */
  private readonly mark: HTMLElement;
  /** The Work shown. */
  private work: string | null = null;
  private timer: number | undefined;
  private running = false;
  /** An edit came while compiling: compile again when done. */
  private again = false;
  private pages: Page[] = [];
  /** Counts the compiles that gave pages, so each page knows if it's been checked since. */
  private generation = 0;
  private inView = new Set<number>();
  private readonly observer: IntersectionObserver;

  constructor(
    private readonly root: HTMLElement,
    private readonly bar: HTMLElement,
    private readonly list: HTMLElement,
    private readonly hooks: PreviewHooks,
  ) {
    // Pages are drawn as they come near the visible area (a screen's height ahead).
    this.observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        const index = Number((entry.target as HTMLElement).dataset.page);
        if (entry.isIntersecting) this.inView.add(index);
        else this.inView.delete(index);
      }
      this.drawInView();
    }, { root: this.list, rootMargin: "100% 0px" });
    this.mark = document.createElement("div");
    this.mark.className = "cursor-mark";
    // A click on a page puts the cursor at that place in the text.
    this.list.addEventListener("click", (e) => {
      const el = (e.target as HTMLElement).closest<HTMLElement>(".page");
      const page = el ? this.pages[Number(el.dataset.page)] : undefined;
      if (!page || !this.work) return;
      const box = el!.getBoundingClientRect();
      const x = ((e.clientX - box.left) / box.width) * page.width;
      const y = ((e.clientY - box.top) / box.height) * page.height;
      void this.jump(this.work, Number(el!.dataset.page), x, y);
    });
  }

  setFollows(on: boolean) {
    this.follows = on;
    if (on) this.followSoon(0);
    else this.mark.remove();
  }

  /** A typing burst was recorded: compile, so the pages are those of the History as it is. */
  committed(work: string) {
    if (work === this.work && this.open) this.compileSoon(0);
  }

  /** The cursor moved in a Work. */
  cursor(work: string) {
    if (work === this.work) this.followSoon(FOLLOW_MS);
  }

  private async jump(work: string, page: number, x: number, y: number) {
    try {
      const to = await invoke<JumpTo | null>("preview_jump", { id: work, page, x, y });
      if (!to || work !== this.work) return;
      if (to.kind === "text") this.hooks.reveal(work, to.at, to.at);
      else this.scrollTo(to.spot, true);
    } catch (e) {
      this.hooks.error(String(e));
    }
  }

  private followSoon(delay: number) {
    if (!this.follows || !this.open) return;
    clearTimeout(this.followTimer);
    this.followTimer = window.setTimeout(() => void this.follow(), delay);
  }

  /** Keeps the cursor's spot on the pages in view, and marks it. */
  private async follow() {
    const work = this.work;
    if (!work || !this.follows || !this.open) return;
    const at = this.hooks.cursorOf(work);
    if (at === null) return;
    try {
      const spot = await invoke<Spot | null>("preview_locate", { id: work, at });
      if (work !== this.work || !this.follows) return;
      if (spot) this.scrollTo(spot, false);
      else this.mark.remove();
    } catch {
      // Not compiled yet: the next compile follows.
    }
  }

  /** Scrolls a spot into view (always, or only if it's near an edge), and marks it if it's the cursor's. */
  private scrollTo(spot: Spot, always: boolean) {
    const page = this.pages[spot.page];
    if (!page) return;
    const scale = page.el.clientHeight / page.height;
    const y = page.el.offsetTop + spot.y * scale;
    const top = this.list.scrollTop;
    const bottom = top + this.list.clientHeight;
    if (always || y < top + FOLLOW_MARGIN || y > bottom - FOLLOW_MARGIN) {
      this.list.scrollTo({ top: Math.max(0, y - this.list.clientHeight / 3) });
    }
    if (!always) {
      // A caret about one line high, ending on the baseline.
      const line = 12;
      this.mark.style.left = `${(spot.x / page.width) * 100}%`;
      this.mark.style.top = `${((spot.y - line * 0.8) / page.height) * 100}%`;
      this.mark.style.height = `${(line / page.height) * 100}%`;
      if (this.mark.parentElement !== page.el) page.el.append(this.mark);
    }
  }

  setOpen(open: boolean) {
    this.open = open;
    this.root.hidden = !open;
    if (open) this.compileSoon(0);
    else clearTimeout(this.timer);
  }

  /** The Work in front changed (or none is). */
  show(work: string | null) {
    if (work === this.work) return;
    this.work = work;
    this.clear();
    this.showBar(null);
    if (work && this.open) this.compileSoon(0);
  }

  /** The text of a Work changed (typed here, or taken in from elsewhere). */
  edited(work: string) {
    if (work === this.work && this.open) this.compileSoon(PAUSE_MS);
  }

  /** A Work's tab closed: its pages aren't needed any more. */
  forget(work: string) {
    if (work === this.work) this.show(null);
    invoke("forget_preview", { id: work }).catch(() => {});
  }

  private compileSoon(delay: number) {
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.compile(), delay);
  }

  private async compile() {
    const work = this.work;
    if (!work || !this.open) return;
    if (this.running) {
      this.again = true;
      return;
    }
    this.running = true;
    try {
      await this.hooks.settled(work);
      const compiled = await invoke<Compiled>("compile_preview", { id: work });
      if (work === this.work) this.showCompiled(compiled);
    } catch (e) {
      if (work === this.work) this.hooks.error(String(e));
    } finally {
      this.running = false;
      if (this.again) {
        this.again = false;
        this.compileSoon(0);
      }
    }
  }

  private showCompiled(compiled: Compiled) {
    this.list.classList.toggle("stale", compiled.stale);
    if (!compiled.stale) this.generation++;
    // Pages added or removed at the end; the ones that stay keep their picture until redrawn.
    while (this.pages.length > compiled.pages.length) this.removePage(this.pages.pop()!);
    compiled.pages.forEach((size, index) => {
      const page = this.pages[index] ?? this.addPage(index);
      page.width = size.width;
      page.height = size.height;
      page.el.style.aspectRatio = `${size.width} / ${size.height}`;
    });
    this.list.dataset.empty = compiled.pages.length === 0 ? (compiled.errors.length ? "error" : "blank") : "";
    this.showBar(compiled);
    this.drawInView();
    // The pages may have moved under the cursor.
    if (!compiled.stale) this.followSoon(0);
  }

  private addPage(index: number): Page {
    const el = document.createElement("div");
    el.className = "page";
    el.dataset.page = String(index);
    this.list.append(el);
    const page: Page = { el, width: 1, height: 1, drawn: null, url: null, checked: -1, drawing: false };
    this.pages.push(page);
    this.observer.observe(el);
    return page;
  }

  private removePage(page: Page) {
    this.observer.unobserve(page.el);
    this.inView.delete(Number(page.el.dataset.page));
    if (page.url) URL.revokeObjectURL(page.url);
    page.el.remove();
  }

  private clear() {
    this.mark.remove();
    for (const page of this.pages) this.removePage(page);
    this.pages = [];
    this.list.classList.remove("stale");
    this.list.dataset.empty = "";
  }

  /** Checks every page in view that hasn't been checked against the latest compile. */
  private drawInView() {
    for (const index of this.inView) {
      const page = this.pages[index];
      if (page && !page.drawing && page.checked !== this.generation) void this.draw(page, index);
    }
  }

  private async draw(page: Page, index: number) {
    const work = this.work;
    const generation = this.generation;
    page.drawing = true;
    try {
      const picture = await invoke<PagePicture>("preview_page", { id: work, page: index, known: page.drawn });
      if (work !== this.work || this.pages[index] !== page) return;
      page.checked = generation;
      if (picture.svg === null) return; // the same as the one shown
      const url = URL.createObjectURL(new Blob([picture.svg], { type: "image/svg+xml" }));
      let img = page.el.querySelector("img");
      if (!img) {
        img = document.createElement("img");
        img.alt = `Page ${index + 1}`;
        img.draggable = false;
        page.el.append(img);
      }
      img.src = url;
      if (page.url) URL.revokeObjectURL(page.url);
      page.url = url;
      page.drawn = picture.hash;
    } catch {
      // The pages changed meanwhile (fewer of them): the next compile sorts it out.
    } finally {
      page.drawing = false;
      // Another compile finished while this was on its way: check again against that one.
      if (this.pages[index] === page && page.checked !== this.generation && this.inView.has(index)) void this.draw(page, index);
    }
  }

  /** The line above the pages: the first error, warnings, and fonts only this Device has. */
  private showBar(compiled: Compiled | null) {
    this.bar.replaceChildren();
    if (!compiled) {
      this.bar.hidden = true;
      return;
    }
    const work = this.work;
    const error = compiled.errors[0];
    if (error) {
      const line = document.createElement("p");
      line.className = "preview-error";
      const where = error.line ? `Line ${error.line}: ` : "";
      const more = compiled.errors.length > 1 ? ` (and ${compiled.errors.length - 1} more)` : "";
      line.textContent = `${where}${error.message}${more}. `;
      line.title = error.hints.join("\n");
      if (work && error.from !== null) {
        const from = error.from;
        const to = error.to ?? from;
        const show = document.createElement("button");
        show.type = "button";
        show.className = "link";
        show.textContent = "Show me";
        show.addEventListener("click", () => this.hooks.reveal(work, from, to));
        line.append(show);
      }
      this.bar.append(line);
    }
    if (compiled.deviceFonts.length > 0) {
      const line = document.createElement("p");
      line.className = "preview-note";
      const names = compiled.deviceFonts.join(", ");
      const them = compiled.deviceFonts.length === 1 ? "it" : "them";
      line.textContent = `${names} ${compiled.deviceFonts.length === 1 ? "comes" : "come"} from this Device, not the Vault. ` +
        `Other Devices may render this Work differently. Copy ${them} into the Vault's fonts folder to fix that.`;
      this.bar.append(line);
    }
    if (compiled.warnings.length > 0) {
      const line = document.createElement("p");
      line.className = "preview-note";
      line.textContent = compiled.warnings.length === 1 ? "1 warning" : `${compiled.warnings.length} warnings`;
      line.title = compiled.warnings.map((w) => (w.line ? `Line ${w.line}: ` : "") + w.message).join("\n");
      this.bar.append(line);
    }
    this.bar.hidden = this.bar.childElementCount === 0;
  }
}
