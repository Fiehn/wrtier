// The Chapter list Mod: the Chapters of the Work in front, in the sidebar, with their word counts.
// Clicking one goes to it; dragging one to another place moves it there as a Move, so its
// History and Comments go with it. Parts show as dividers, and stay where they are.
// Its keys (next and previous Chapter, move this Chapter up or down) work through the same.

import { chaptersIn, itemAt, mapThrough, neighbourDest, planMove, type Item, type MoveOp } from "./chapters";

export type ChapterHooks = {
  /** The Work's text as its editor has it now (null if it isn't open). */
  textOf(id: string): string | null;
  cursorOf(id: string): number | null;
  /** Puts the cursor at `at` in the Work and shows it. */
  goTo(id: string, at: number): void;
  /** Moves text as Moves, then puts the cursor at `cursor` (in the text after them). */
  move(id: string, moves: MoveOp[], cursor: number): void;
};

/** How long after typing stops the list catches up (the counts needn't follow every key). */
const SETTLE_MS = 400;

export class ChapterList {
  /** The Work shown (in front, and with the Mod in use on this Device), or null: hidden. */
  private work: string | null = null;
  private items: Item[] = [];
  private current = -1;
  private timer: number | undefined;
  /** The Chapter being dragged (its index), while it is. */
  private dragging: number | null = null;

  constructor(
    private readonly root: HTMLElement,
    private readonly list: HTMLElement,
    private readonly none: HTMLElement,
    private readonly hooks: ChapterHooks,
  ) {}

  /** Shows the Chapters of `work`, or hides the list (null). */
  show(work: string | null) {
    this.work = work;
    this.root.hidden = work === null;
    this.refresh();
  }

  /** A Work's text changed: if it's the one shown, the list catches up after a pause. */
  edited(id: string) {
    if (id !== this.work) return;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.refresh(), SETTLE_MS);
  }

  /** The cursor moved: the Chapter it's in is marked. */
  cursor(id: string, at: number) {
    if (id !== this.work) return;
    const current = itemAt(this.items, at);
    if (current === this.current) return;
    this.current = current;
    this.list.querySelectorAll("li[data-index]").forEach((li) => {
      li.classList.toggle("current", Number((li as HTMLElement).dataset.index) === current);
    });
  }

  private refresh() {
    clearTimeout(this.timer);
    const text = this.work ? this.hooks.textOf(this.work) : null;
    this.items = text === null ? [] : chaptersIn(text);
    const at = this.work ? this.hooks.cursorOf(this.work) : null;
    this.current = at === null ? -1 : itemAt(this.items, at);
    this.render();
  }

  // ---- Its keys ----

  /**
   * Goes to the next (1) Chapter's heading, or back (-1): to this Chapter's heading, then the
   * one before, as moving by paragraph does.
   */
  step(id: string, direction: -1 | 1): boolean {
    const text = this.hooks.textOf(id);
    const at = this.hooks.cursorOf(id);
    if (text === null || at === null) return false;
    const chapters = chaptersIn(text).filter((i) => i.kind === "chapter");
    const target = direction === 1 ? chapters.find((i) => i.from > at) : [...chapters].reverse().find((i) => i.from < at);
    if (target) this.hooks.goTo(id, target.from);
    return true;
  }

  /** Moves the Chapter the cursor is in up (-1) or down (1), past one neighbour. */
  shift(id: string, direction: -1 | 1): boolean {
    const text = this.hooks.textOf(id);
    const at = this.hooks.cursorOf(id);
    if (text === null || at === null) return false;
    const items = chaptersIn(text);
    const index = itemAt(items, at);
    const dest = neighbourDest(items, index, direction, text.length);
    if (dest !== null) this.moveTo(id, text, items[index], dest, at);
    return true;
  }

  /** Moves a Chapter to `dest`; the cursor goes with it (to `cursor`, or its heading). */
  private moveTo(id: string, text: string, item: Item, dest: number, cursor = item.from) {
    const moves = planMove(text, item.from, item.to, dest);
    if (moves.length === 0) return;
    const inside = cursor >= item.from && cursor < item.to ? cursor : item.from;
    this.hooks.move(id, moves, moves.reduce((pos, m) => mapThrough(pos, m), inside));
    this.refresh();
  }

  // ---- Drawing it ----

  private render() {
    const rows = this.items.map((item, index) => {
      const li = document.createElement("li");
      li.dataset.index = String(index);
      li.className = item.kind;
      li.classList.toggle("current", index === this.current);
      const title = document.createElement("span");
      title.className = "chapter-title";
      title.textContent = item.title;
      li.append(title);
      if (item.kind === "chapter") {
        const words = document.createElement("span");
        words.className = "chapter-words";
        words.textContent = item.words.toLocaleString();
        words.title = item.words === 1 ? "1 word" : `${item.words.toLocaleString()} words`;
        li.append(words);
        li.draggable = true;
        li.title = "Click to go there; drag to move it";
        li.addEventListener("dragstart", (e) => {
          this.dragging = index;
          e.dataTransfer?.setData("text/plain", item.title); // some engines only drag with data
          if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
          li.classList.add("dragging");
        });
        li.addEventListener("dragend", () => {
          this.dragging = null;
          this.clearDrop();
          li.classList.remove("dragging");
        });
      }
      li.addEventListener("click", () => this.work && this.hooks.goTo(this.work, item.from));
      this.dropTarget(li, index);
      return li;
    });
    // Below the last one: dropping there moves a Chapter to the end.
    const end = document.createElement("li");
    end.className = "chapter-end";
    this.dropTarget(end, this.items.length);
    this.list.replaceChildren(...rows, ...(this.items.length > 0 ? [end] : []));
    this.none.hidden = this.items.length > 0;
  }

  private clearDrop() {
    this.list.querySelectorAll(".drop-before").forEach((el) => el.classList.remove("drop-before"));
  }

  /** Dropping a Chapter on `el` puts it before item `index` (or at the end). */
  private dropTarget(el: HTMLElement, index: number) {
    el.addEventListener("dragover", (e) => {
      if (this.dragging === null) return; // not one of ours
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
      if (!el.classList.contains("drop-before")) {
        this.clearDrop();
        el.classList.add("drop-before");
      }
    });
    el.addEventListener("dragleave", () => el.classList.remove("drop-before"));
    el.addEventListener("drop", (e) => {
      const from = this.dragging;
      this.dragging = null;
      this.clearDrop();
      if (from === null || !this.work) return;
      e.preventDefault();
      // The text as it is now: if its Chapters changed meanwhile, start again from the list.
      const text = this.hooks.textOf(this.work);
      if (text === null) return;
      const items = chaptersIn(text);
      if (items.length !== this.items.length || items[from]?.title !== this.items[from]?.title) {
        this.refresh();
        return;
      }
      const dest = index < items.length ? items[index].from : text.length;
      this.moveTo(this.work, text, items[from], dest);
    });
  }
}
