// Find in Vault (Ctrl+Shift+F, v2 plan): finding some text in every Work in the Vault at once,
// with the matches listed by Work, in the Vault browser in place of the Works list. It only
// finds; replacing is Find and replace in one Work ("Open with Find and replace").
//
// Every Work is matched as its text reads in the Formatted view, with Find's own rules
// (find.ts), whichever view it's in. Each Work's own text once: an Inclusion is its one
// character, so included text is found only in the Work it belongs to. A Work open in a tab is
// read from its editor, typing included; any other from its History (Rust, `text_for_finding`),
// once, and again only when its file changes. The list keeps up by itself: a changed Work's
// group updates in place and the rest stays where it was.

import type { Text } from "@codemirror/state";
import { endOf, matchesIn, readingOfText, startOf, type Query, type Reading } from "./find";
import { chaptersIn, itemAt, type Item } from "./chapters";
import type { ElementInfo } from "./elements";

/** At most this many matches are listed; the counts stay true past it. */
export const MAX_SHOWN = 2000;
/** How long after the last key it searches. */
const PAUSE_MS = 250;

/** What Rust says about a Work's text (vault/find.rs). */
export type TextForFinding =
  | { kind: "unchanged" }
  | { kind: "text"; stamp: string; text: string }
  | { kind: "unreadable"; reason: string };

/** A match's few words around it, as the text reads. */
export type Snippet = { before: string; match: string; after: string };

/** A match in a Work: where it is in its text as typed, its Chapter, and how it reads. */
export type VaultMatch = { from: number; to: number; chapter: string | null; snippet: Snippet };

/** A Work in the Works list's order: its ID, its name, and why it can't be searched (if so). */
export type VaultWork = { id: string; name: string; problem: string | null };

// ---- Matching (pure) ----

/** Up to `room` characters either side of a match, cut at a space, within its paragraph. */
export function snippetAt(reading: Reading, a: number, b: number, room = 48): Snippet {
  const text = reading.text;
  const lineStart = text.lastIndexOf("\n", a - 1) + 1;
  let lineEnd = text.indexOf("\n", b);
  if (lineEnd === -1) lineEnd = text.length;
  let start = Math.max(lineStart, a - room);
  if (start > lineStart) {
    const space = text.indexOf(" ", start);
    if (space !== -1 && space < a) start = space + 1;
  }
  let end = Math.min(lineEnd, b + room * 2);
  if (end < lineEnd) {
    const space = text.lastIndexOf(" ", end);
    if (space > b) end = space;
  }
  // A Story element's chip or an Inclusion stops a match; in a snippet it's a dot.
  const plain = (s: string) => s.replace(/\u0000/g, "·");
  return {
    before: (start > lineStart ? "…" : "") + plain(text.slice(start, a)).trimStart(),
    match: plain(text.slice(a, b)),
    after: plain(text.slice(b, end)).trimEnd() + (end < lineEnd ? "…" : ""),
  };
}

/** Every match of `query` in a Work's text, in reading order, each with its Chapter's title. */
export function matchesInWork(reading: Reading, chapters: Item[], query: Query): VaultMatch[] {
  return matchesIn(reading.text, query).map(([a, b]) => {
    const from = startOf(reading, a);
    const at = itemAt(chapters, from);
    return { from, to: endOf(reading, b), chapter: at === -1 ? null : chapters[at].title, snippet: snippetAt(reading, a, b) };
  });
}

/** "23 matches in 4 Works". */
export function summary(total: number, works: number): string {
  if (total === 0) return "No matches";
  const m = total === 1 ? "1 match" : `${total.toLocaleString("en")} matches`;
  return `${m} in ${works === 1 ? "1 Work" : `${works} Works`}`;
}

// ---- The panel ----

export type FindInVaultHooks = {
  /** The Works, in the Works list's order (folders first, as it shows them). */
  works(): VaultWork[];
  /** The text of a Work open in a tab, as it is in the editor; null if it isn't open. */
  openText(id: string): Text | null;
  /** A Work's text from its History, unless it still has the stamp `known`. */
  read(id: string, known: string | null): Promise<TextForFinding>;
  /** The Style library's Story elements (whose calls stop a match, as in the editor). */
  elements(): ElementInfo[];
  /** Shows a match in its Work (a passing tab if it isn't open); `focus`: into the editor. */
  show(id: string, from: number, to: number, focus: boolean): Promise<void>;
  /** Opens the Work as an ordinary tab, with Find and replace showing this query. */
  openWithFind(id: string, query: Query, match: { from: number; to: number } | null): Promise<void>;
  /** Shown or closed: the Works list goes or comes back. */
  shown(open: boolean): void;
  error(message: string): void;
};

/** A Work's text as last read, how it reads, and its Chapters. */
type Known = { stamp: string | null; text: Text | string; elements: string; reading: Reading; chapters: Item[] };

/** A Work with matches, or one that couldn't be searched. */
type Found = { work: VaultWork; total: number; matches: VaultMatch[] };
type Unsearched = { work: VaultWork; unreadable: string };
type Group = Found | Unsearched;

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

export class FindInVault {
  private readonly panel = $("vault-find");
  private readonly input = $<HTMLInputElement>("vault-find-input");
  private readonly matchCase = $<HTMLInputElement>("vault-find-case");
  private readonly wholeWords = $<HTMLInputElement>("vault-find-words");
  private readonly status = $("vault-find-summary");
  private readonly list = $("vault-find-results");

  /** Closed Works' texts, by ID. Open ones are read from their editor each time it changed. */
  private readonly closed = new Map<string, Known>();
  private readonly open = new Map<string, Known>();
  private groups: Group[] = [];
  /** The Works folded away in the list. */
  private readonly folded = new Set<string>();
  /** The match picked (by Up and Down, or a click): its Work and where it is. */
  private current: { id: string; from: number } | null = null;
  /** Counts the searches, so one that's overtaken stops. */
  private generation = 0;
  private timer: number | undefined;
  /** What the list was last drawn from, to leave it alone when nothing changed. */
  private drawn = "";

  constructor(private readonly hooks: FindInVaultHooks) {
    this.input.addEventListener("input", () => this.searchSoon());
    this.matchCase.addEventListener("change", () => void this.search());
    this.wholeWords.addEventListener("change", () => void this.search());
    $("vault-find-close").addEventListener("click", () => this.close());
    this.panel.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        this.close();
      } else if (e.key === "Enter" && e.target === this.input) {
        e.preventDefault();
        void this.search();
      } else if (e.key === "ArrowDown" && e.target === this.input) {
        e.preventDefault();
        this.step(1);
      }
    });
    this.list.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        this.step(e.key === "ArrowDown" ? 1 : -1);
      } else if (e.key === "Enter" && this.current) {
        e.preventDefault();
        void this.hooks.show(this.current.id, this.current.from, this.matchAt(this.current)?.to ?? this.current.from, true);
      }
    });
  }

  get isOpen(): boolean {
    return !this.panel.hidden;
  }

  private get query(): Query {
    return { text: this.input.value, matchCase: this.matchCase.checked, wholeWords: this.wholeWords.checked };
  }

  /** Opens it (Ctrl+Shift+F, or its button), with `text` to find if given, else the last query. */
  show(text?: string) {
    if (text) this.input.value = text;
    if (!this.isOpen) {
      this.panel.hidden = false;
      this.hooks.shown(true);
    }
    this.input.focus();
    this.input.select();
    void this.search();
  }

  /** Another Vault: nothing read or found here carries over. */
  reset() {
    this.close();
    this.closed.clear();
    this.open.clear();
    this.groups = [];
    this.folded.clear();
    this.current = null;
    this.input.value = "";
    this.drawn = "";
    this.list.replaceChildren();
    this.status.hidden = true;
  }

  close() {
    if (!this.isOpen) return;
    clearTimeout(this.timer);
    this.generation++;
    this.panel.hidden = true;
    this.hooks.shown(false);
  }

  /** A Work's text changed in its tab: its group catches up. */
  edited(id: string) {
    if (this.isOpen && this.input.value && this.hooks.works().some((w) => w.id === id)) this.searchSoon();
  }

  /**
   * The Works may have changed (the list was refreshed: new, renamed or trashed Works, or Works
   * synced from another Device): Works already read are checked against their files, and the
   * search runs again if anything differs.
   */
  async refresh() {
    if (!this.isOpen || !this.input.value) return;
    const generation = this.generation;
    const elements = this.hooks.elements();
    const key = elements.map((e) => e.name).join(" ");
    for (const [id, known] of [...this.closed]) {
      if (this.hooks.openText(id)) continue;
      try {
        const answer = await this.hooks.read(id, known.stamp);
        if (answer.kind === "text") this.closed.set(id, this.know(answer.stamp, answer.text, elements, key));
        else if (answer.kind === "unreadable") this.closed.delete(id); // the search says why
      } catch {
        this.closed.delete(id);
      }
    }
    if (generation !== this.generation) return; // a search started meanwhile
    // Unchanged Works are matched from what was read, and the list is only drawn again if it differs.
    await this.search();
  }

  /** The Style library's Story elements changed: texts read the same, but may read differently. */
  libraryChanged() {
    if (this.isOpen && this.input.value) this.searchSoon();
  }

  private searchSoon() {
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.search(), PAUSE_MS);
  }

  /** How a Work reads now: from its tab, from what was read before, or null if it has to be read. */
  private knownNow(id: string, elements: ElementInfo[], key: string): Known | null {
    const doc = this.hooks.openText(id);
    if (doc) {
      const had = this.open.get(id);
      if (had && had.text === doc && had.elements === key) return had;
      const known = this.know(null, doc, elements, key);
      this.open.set(id, known);
      return known;
    }
    this.open.delete(id);
    const had = this.closed.get(id);
    if (!had) return null;
    if (had.elements === key) return had;
    const known = this.know(had.stamp, had.text, elements, key);
    this.closed.set(id, known);
    return known;
  }

  private know(stamp: string | null, text: Text | string, elements: ElementInfo[], key: string): Known {
    const reading = readingOfText(text, elements);
    const plain = typeof text === "string" ? text : text.toString();
    return { stamp, text, elements: key, reading, chapters: chaptersIn(plain) };
  }

  /** Finds the query in every Work, reading the ones never read (and showing how far it's got). */
  private async search() {
    clearTimeout(this.timer);
    const generation = ++this.generation;
    const query = this.query;
    if (!query.text) {
      this.groups = [];
      this.draw(null);
      return;
    }
    const works = this.hooks.works();
    const elements = this.hooks.elements();
    const key = elements.map((e) => e.name).join(" ");
    const toRead = works.filter((w) => !w.problem && !this.hooks.openText(w.id) && !this.closed.has(w.id)).length;
    let read = 0;
    const groups: Group[] = [];
    for (const work of works) {
      if (work.problem) {
        groups.push({ work, unreadable: work.problem });
        continue;
      }
      let known = this.knownNow(work.id, elements, key);
      if (!known) {
        // Read from its History: the list so far is shown meanwhile.
        this.groups = groups;
        this.draw({ read, of: toRead });
        let answer: TextForFinding;
        try {
          answer = await this.hooks.read(work.id, null);
        } catch (e) {
          answer = { kind: "unreadable", reason: String(e) };
        }
        if (generation !== this.generation) return;
        read++;
        if (answer.kind === "unreadable") {
          groups.push({ work, unreadable: answer.reason });
          continue;
        }
        if (answer.kind === "text") this.closed.set(work.id, this.know(answer.stamp, answer.text, elements, key));
        known = this.knownNow(work.id, elements, key);
        if (!known) continue;
      }
      const matches = matchesInWork(known.reading, known.chapters, query);
      if (matches.length > 0) groups.push({ work, total: matches.length, matches });
    }
    if (generation !== this.generation) return;
    this.groups = groups;
    this.draw(null);
  }

  /** The matches as listed: past MAX_SHOWN, the rest are counted but not listed. */
  private listed(): { group: Found; matches: VaultMatch[] }[] {
    let left = MAX_SHOWN;
    const out = [];
    for (const group of this.groups) {
      if (!("matches" in group)) continue;
      const matches = group.matches.slice(0, Math.max(0, left));
      left -= matches.length;
      out.push({ group, matches });
    }
    return out;
  }

  private matchAt(at: { id: string; from: number }): VaultMatch | null {
    const group = this.groups.find((g) => g.work.id === at.id);
    return group && "matches" in group ? (group.matches.find((m) => m.from === at.from) ?? null) : null;
  }

  /** Up and Down: the next match listed (in an unfolded Work), shown in its Work, the focus kept here. */
  private step(direction: 1 | -1) {
    const all = this.listed().filter(({ group }) => !this.folded.has(group.work.id))
      .flatMap(({ group, matches }) => matches.map((m) => ({ id: group.work.id, match: m })));
    if (all.length === 0) return;
    const i = this.current ? all.findIndex((x) => x.id === this.current!.id && x.match.from === this.current!.from) : -1;
    const next = all[i === -1 ? (direction === 1 ? 0 : all.length - 1) : Math.min(all.length - 1, Math.max(0, i + direction))];
    this.pick(next.id, next.match, false);
    this.list.focus();
  }

  private pick(id: string, match: VaultMatch, focus: boolean) {
    this.current = { id, from: match.from };
    for (const el of this.list.querySelectorAll(".vf-match.current")) el.classList.remove("current");
    const el = this.list.querySelector<HTMLElement>(`.vf-work[data-id="${CSS.escape(id)}"] .vf-match[data-from="${match.from}"]`);
    el?.classList.add("current");
    el?.scrollIntoView({ block: "nearest" });
    this.hooks.show(id, match.from, match.to, focus).catch((e) => this.hooks.error(String(e)));
  }

  /** Draws the list (unless it would look the same), keeping where it's scrolled to. */
  private draw(progress: { read: number; of: number } | null) {
    const listed = this.listed();
    const total = this.groups.reduce((n, g) => n + ("total" in g ? g.total : 0), 0);
    const found = this.groups.filter((g) => "total" in g).length;
    const unreadable = this.groups.filter((g): g is Unsearched => "unreadable" in g);
    let line = this.input.value ? summary(total, found) : "";
    if (progress) line = `Reading Works… ${progress.read} of ${progress.of}${total ? ` · ${summary(total, found)}` : ""}`;
    this.status.textContent = line;
    this.status.hidden = !line;

    const signature = JSON.stringify([listed.map(({ group, matches }) => [group.work.id, group.total, matches.map((m) => [m.from, m.to, m.snippet])]),
      unreadable.map((g) => [g.work.id, g.unreadable]), total]);
    if (signature === this.drawn) return;
    this.drawn = signature;
    if (this.current && !this.matchAt(this.current)) this.current = null;

    const scrolled = this.list.scrollTop;
    this.list.replaceChildren();
    for (const { group, matches } of listed) this.list.append(this.drawWork(group, matches));
    const shown = listed.reduce((n, l) => n + l.matches.length, 0);
    if (shown < total) {
      const cap = document.createElement("p");
      cap.className = "vf-cap muted";
      cap.textContent = `Showing ${shown.toLocaleString("en")} of ${total.toLocaleString("en")} matches. Narrow the search to see the rest.`;
      this.list.append(cap);
    }
    if (unreadable.length > 0) {
      const box = document.createElement("div");
      box.className = "vf-unsearched";
      const head = document.createElement("p");
      head.textContent = "Couldn't search:";
      box.append(head);
      for (const g of unreadable) {
        const p = document.createElement("p");
        p.className = "muted";
        p.textContent = `${g.work.name}: ${g.unreadable}`;
        p.title = g.work.id;
        box.append(p);
      }
      this.list.append(box);
    }
    this.list.scrollTop = scrolled;
  }

  private drawWork(group: Found, matches: VaultMatch[]): HTMLElement {
    const { work } = group;
    const section = document.createElement("section");
    section.className = "vf-work";
    section.dataset.id = work.id;
    const folded = this.folded.has(work.id);
    const head = document.createElement("button");
    head.type = "button";
    head.className = "vf-work-head";
    head.setAttribute("aria-expanded", String(!folded));
    head.title = work.id;
    const chevron = document.createElement("span");
    chevron.className = "chevron";
    chevron.textContent = "▸";
    const name = document.createElement("span");
    name.className = "vf-work-name";
    name.textContent = work.name;
    const folder = work.id.slice(0, Math.max(0, work.id.length - work.name.length - 1));
    if (folder) {
      const where = document.createElement("small");
      where.textContent = folder;
      name.append(" ", where);
    }
    const count = document.createElement("span");
    count.className = "vf-count";
    count.textContent = group.total.toLocaleString("en");
    head.append(chevron, name, count);
    const body = document.createElement("div");
    body.className = "vf-matches";
    body.hidden = folded;
    head.addEventListener("click", () => {
      if (this.folded.has(work.id)) this.folded.delete(work.id);
      else this.folded.add(work.id);
      const now = this.folded.has(work.id);
      body.hidden = now;
      head.setAttribute("aria-expanded", String(!now));
    });

    let chapter: string | null | undefined;
    for (const match of matches) {
      if (match.chapter !== chapter) {
        chapter = match.chapter;
        if (chapter !== null) {
          const label = document.createElement("div");
          label.className = "vf-chapter";
          label.textContent = chapter;
          body.append(label);
        }
      }
      const item = document.createElement("button");
      item.type = "button";
      item.className = "vf-match";
      item.dataset.from = String(match.from);
      item.tabIndex = -1;
      const mark = document.createElement("mark");
      mark.textContent = match.snippet.match;
      item.append(match.snippet.before, mark, match.snippet.after);
      if (this.current?.id === work.id && this.current.from === match.from) item.classList.add("current");
      item.addEventListener("click", () => this.pick(work.id, match, true));
      body.append(item);
    }
    const open = document.createElement("button");
    open.type = "button";
    open.className = "link vf-open-find";
    open.textContent = "Open with Find and replace";
    open.title = `Open ${work.name} with these matches in Find and replace, to replace them there`;
    open.addEventListener("click", () => {
      const picked = this.current?.id === work.id ? this.matchAt(this.current) : null;
      this.hooks.openWithFind(work.id, this.query, picked ?? group.matches[0] ?? null).catch((e) => this.hooks.error(String(e)));
    });
    body.append(open);
    section.append(head, body);
    return section;
  }
}
