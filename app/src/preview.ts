// The Page preview: the active Work's compiled pages, in the Pages tab of the side panel.
//
// It compiles only while it's in view, about 300 ms after the last edit, from the text as Rust
// has it (the open typing burst included). Each page comes as its own SVG, and only pages in
// view whose content changed are drawn again. When the Work doesn't compile, the last pages
// that did stay, faded, under a bar with the first error.

import { invoke } from "@tauri-apps/api/core";
import { PageZoom } from "./zoom";

// The same shapes as the Rust side's (typeset.rs).
type PageSize = { width: number; height: number };
type PagePicture = { hash: string; svg: string | null };
type Problem = { message: string; hints: string[]; from: number | null; to: number | null; line: number | null };
/** A spot on the pages: the page (from 0), and points from its top left. */
type Spot = { page: number; x: number; y: number };
type JumpTo = { kind: "text"; at: number } | { kind: "pages"; spot: Spot };
export type Milestone = { id: string; name: string; timeMs: number };
type Compiled = { pages: PageSize[]; stale: boolean; errors: Problem[]; warnings: Problem[]; deviceFonts: string[]; ms: number };

/** What the rest of the page does for the preview. */
export type PreviewHooks = {
  /** Resolves once everything typed in this Work has reached Rust. */
  settled(id: string): Promise<void>;
  /** Shows this stretch of the Work's text in the editor. */
  reveal(id: string, from: number, to: number): void;
  /** Where the cursor is in this Work (UTF-16), if it's open. */
  cursorOf(id: string): number | null;
  /** The Work's Looks, in its Preamble's order. */
  looksOf(id: string): Promise<string[]>;
  /** The Work's Milestones, oldest first. */
  milestonesOf(id: string): Promise<Milestone[]>;
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
  /** How big the pages are shown (zoom.ts). */
  readonly zoom: PageZoom;

  /** The Look the pages are in: picked here, remembered on this Device for each Work. */
  private look: string | null = null;
  /** The Milestone shown instead of the Work as it is now (null: now). Its pages are read-only. */
  private milestone: Milestone | null = null;
  private milestones: Milestone[] = [];

  /** The pages' name on the Rust side: the Work's, or the Work at a Milestone's. */
  private get key(): string | null {
    if (!this.work) return null;
    return this.milestone ? `${this.work}@${this.milestone.id}` : this.work;
  }

  constructor(
    private readonly bar: HTMLElement,
    private readonly list: HTMLElement,
    private readonly picker: HTMLSelectElement,
    private readonly versionPicker: HTMLSelectElement,
    zoomMenu: HTMLSelectElement,
    private readonly hooks: PreviewHooks,
  ) {
    this.zoom = new PageZoom(list, zoomMenu, () => this.pages);
    this.versionPicker.addEventListener("change", () => {
      this.milestone = this.milestones.find((m) => m.id === this.versionPicker.value) ?? null;
      this.clear();
      this.compileSoon(0);
    });
    this.picker.addEventListener("change", () => {
      this.look = this.picker.value || null;
      try {
        if (this.work) localStorage.setItem(`look:${this.work}`, this.picker.value);
      } catch {
        // not remembered, then: no harm done
      }
      this.compileSoon(0);
    });
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
    // A click on a page puts the cursor at that place in the text (not on a Milestone's pages:
    // the text has changed since).
    this.list.addEventListener("click", (e) => {
      const el = (e.target as HTMLElement).closest<HTMLElement>(".page");
      const page = el ? this.pages[Number(el.dataset.page)] : undefined;
      if (!page || !this.work || this.milestone) return;
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
    if (work === this.work && this.open && !this.milestone) this.compileSoon(0);
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
    if (!this.follows || !this.open || this.milestone) return;
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
    // Zoomed in past the panel's width: sideways too.
    if (this.list.scrollWidth > this.list.clientWidth) {
      const x = page.el.offsetLeft + spot.x * scale;
      const left = this.list.scrollLeft;
      if (always || x < left + FOLLOW_MARGIN || x > left + this.list.clientWidth - FOLLOW_MARGIN) {
        this.list.scrollLeft = Math.max(0, x - this.list.clientWidth / 2);
      }
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

  /** Whether the pages are in view (the panel shows its Pages tab): only then does it compile.
   *  Coming into view compiles at once, so the pages catch up with what was typed meanwhile. */
  setOpen(open: boolean) {
    if (open === this.open) return;
    this.open = open;
    if (open) {
      this.compileSoon(0);
    } else {
      clearTimeout(this.timer);
      clearTimeout(this.followTimer);
    }
  }

  /** The Work in front changed (or none is). */
  show(work: string | null) {
    if (work === this.work) return;
    this.work = work;
    this.milestone = null; // another Work: shown as it is now
    this.clear();
    this.showBar(null);
    void this.milestonesChanged(work);
    void this.preambleChanged(work);
  }

  /** A Work's Preamble changed (its Looks may have): pick the Look again, and compile. */
  async preambleChanged(work: string | null) {
    if (work !== this.work) return;
    let looks: string[] = [];
    if (work) {
      try {
        looks = await this.hooks.looksOf(work);
      } catch {
        // not readable right now: the Work's first Look is used
      }
    }
    if (work !== this.work) return;
    let remembered: string | null = null;
    try {
      remembered = work ? localStorage.getItem(`look:${work}`) : null;
    } catch {
      // nothing remembered
    }
    this.look = remembered && looks.includes(remembered) ? remembered : (looks[0] ?? null);
    this.picker.replaceChildren(...looks.map((name, i) => {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = i === 0 ? `${name} (the Work's own)` : name;
      return option;
    }));
    this.picker.value = this.look ?? "";
    this.picker.hidden = looks.length < 2;
    if (work && this.open) this.compileSoon(0);
  }

  /** The text of a Work changed (typed here, or taken in from elsewhere). A Milestone's pages
   *  don't change with it. */
  edited(work: string) {
    if (work === this.work && this.open && !this.milestone) this.compileSoon(PAUSE_MS);
  }

  /** The Look the pages of this Work are shown in, if they're shown (an Export starts from it). */
  lookFor(work: string): string | null {
    return work === this.work ? this.look : null;
  }

  /** The Work shown, if any. */
  /** The Milestone shown instead of the Work as it is now, if one: an Export from here is of it. */
  get shownMilestone(): { id: string; name: string } | null {
    return this.milestone;
  }

  get shown(): string | null {
    return this.work;
  }

  /** The Work's Milestones changed (one was named): the picker offers them. */
  async milestonesChanged(work: string | null) {
    if (work !== this.work) return;
    let list: Milestone[] = [];
    if (work) {
      try {
        list = await this.hooks.milestonesOf(work);
      } catch {
        // not readable right now: only "now"
      }
    }
    if (work !== this.work) return;
    this.milestones = list;
    if (this.milestone && !list.some((m) => m.id === this.milestone!.id)) this.milestone = null;
    const option = (value: string, text: string) => {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = text;
      return o;
    };
    const date = (ms: number) => new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
    this.versionPicker.replaceChildren(option("", "Now"), ...[...list].reverse().map((m) => option(m.id, `${m.name} (${date(m.timeMs)})`)));
    this.versionPicker.value = this.milestone?.id ?? "";
    this.versionPicker.hidden = list.length === 0;
  }

  /** A Work's tab closed: its pages aren't needed any more. */
  forget(work: string) {
    if (work === this.work) this.show(null);
    invoke("forget_preview", { id: work }).catch(() => {});
    for (const m of this.milestones) invoke("forget_preview", { id: `${work}@${m.id}` }).catch(() => {});
  }

  private compileSoon(delay: number) {
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.compile(), delay);
  }

  private async compile() {
    const work = this.work;
    const key = this.key;
    if (!work || !this.open) return;
    if (this.running) {
      this.again = true;
      return;
    }
    this.running = true;
    try {
      await this.hooks.settled(work);
      const compiled = await invoke<Compiled>("compile_preview", { id: work, look: this.look, milestone: this.milestone?.id ?? null });
      if (key === this.key) this.showCompiled(compiled);
    } catch (e) {
      if (key === this.key) this.hooks.error(String(e));
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
    this.zoom.pagesChanged();
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
    this.zoom.cleared();
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
    const key = this.key;
    const generation = this.generation;
    page.drawing = true;
    try {
      const picture = await invoke<PagePicture>("preview_page", { id: key, page: index, known: page.drawn });
      if (key !== this.key || this.pages[index] !== page) return;
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
    const work = this.milestone ? null : this.work; // a Milestone's text isn't in the editor
    if (this.milestone) {
      const line = document.createElement("p");
      line.className = "preview-milestone";
      line.textContent = `The Work as it was at the Milestone “${this.milestone.name}”, with the Styles it had then. Read-only: pick Now to see it as it is.`;
      this.bar.append(line);
    }
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
