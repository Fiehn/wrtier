// The Style library in the sidebar: the Vault's Looks, Styles and Story elements (files in
// `styles/`), the Conflict copies waiting for the Author to pick a version, and making new ones.

import { invoke } from "@tauri-apps/api/core";
import { SYMBOLS, symbol } from "./ui";

// The same shapes as the Rust side's (library.rs).
export type Kind = "style" | "look" | "storyElement";
export type LibraryItem = {
  kind: Kind;
  name: string;
  title: string;
  path: string;
  description: string;
  problems: string[];
  writtenAs: string | null;
  shownAs: string | null;
  docx: string | null;
  styles: string[];
};
type Conflict = { path: string; copy: string };
/** A newer version of a default than the Vault was given (ADR 0012). */
export type Offer = { path: string; kind: Kind; title: string };
/** A line of a default's two versions: in both, only in the Vault's copy, or only in the newer one. */
export type CompareLine = { kind: "same" | "mine" | "newer"; text: string };
type Library = { items: LibraryItem[]; conflicts: Conflict[]; builtIn: string[] };
export type NewElement = { title: string; shownAs: string; docx: string; description: string };

/** What the panel does with the rest of the page. */
export type LibraryHooks = {
  /** Opens a library file in a tab. */
  open(path: string, title: string): Promise<void>;
  /** The plus on a group's line: asks for and makes a new one of that kind. */
  create(kind: Kind): void;
  /** Right-click on a file: what can be done with it (rename, remove), at (x, y). */
  menu(item: LibraryItem, x: number, y: number): void;
  /** Shows a default's two versions; the Author's choice, or null if they just closed it. */
  compare(offer: Offer, lines: CompareLine[]): Promise<"use" | "keep" | null>;
  /** Saves what's typed in open tabs, before a file is replaced. */
  settle(): Promise<void>;
  /** How many newer defaults are on offer, to mark the library with. */
  offered(count: number): void;
  error(message: string): void;
};

const GROUPS: { kind: Kind; label: string; one: string }[] = [
  { kind: "look", label: "Looks", one: "Look" },
  { kind: "style", label: "Styles", one: "Style" },
  { kind: "storyElement", label: "Story elements", one: "Story element" },
];

/** Which groups are open (Device state, in local storage): closed at first. */
const OPEN_KEY = "library-open";

function readOpen(): Set<Kind> {
  try {
    return new Set(JSON.parse(localStorage.getItem(OPEN_KEY) ?? "[]") as Kind[]);
  } catch {
    return new Set();
  }
}

export class LibraryPanel {
  items: LibraryItem[] = [];
  /** The built-in Story elements' names: every Work has them. */
  builtIn: string[] = [];
  /** The groups open in the list, kept as they are when it's drawn again. */
  private open = readOpen();

  constructor(
    private readonly list: HTMLElement,
    private readonly conflicts: HTMLElement,
    private readonly offers: HTMLElement,
    private readonly hooks: LibraryHooks,
  ) {}

  /** The title a library file goes by in its tab. */
  titleOf(path: string): string {
    return this.items.find((i) => i.path === path)?.title ?? path.split("/").pop()!.replace(/\.(typ|look)$/, "");
  }

  async refresh() {
    const library = await invoke<Library>("get_library");
    this.items = library.items;
    this.builtIn = library.builtIn;
    this.showConflicts(library.conflicts);
    this.showOffers(await invoke<Offer[]>("default_offers"));
    this.list.replaceChildren();
    for (const group of GROUPS) {
      const items = library.items.filter((i) => i.kind === group.kind);
      // Each kind is a group of its own that opens and closes, with how many it has.
      const details = document.createElement("details");
      details.className = "library-group";
      details.open = this.open.has(group.kind);
      const summary = document.createElement("summary");
      summary.textContent = group.label;
      const count = document.createElement("span");
      count.className = "library-count";
      count.textContent = String(items.length);
      summary.append(" ", count, this.newButton(group.kind, group.one));
      // A problem inside a closed group still shows.
      if (items.some((i) => i.problems.length > 0)) {
        summary.classList.add("has-problem");
        summary.title = "Something in here has a problem: hover over the marked one for what.";
      }
      details.addEventListener("toggle", () => this.setOpen(group.kind, details.open));
      const ul = document.createElement("ul");
      for (const item of items) ul.append(this.itemElement(item));
      if (items.length === 0) {
        const none = document.createElement("li");
        none.className = "muted";
        none.textContent = "None yet";
        ul.append(none);
      }
      details.append(summary, ul);
      this.list.append(details);
    }
  }

  /** The plus at the right of a group's line, for making a new one of its kind. */
  private newButton(kind: Kind, one: string): HTMLButtonElement {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "library-add";
    b.append(symbol(SYMBOLS.plus, 14));
    b.title = `New ${one}…`;
    b.setAttribute("aria-label", `New ${one}…`);
    b.dataset.kind = kind;
    b.addEventListener("click", (e) => {
      e.preventDefault(); // the group stays open or closed as it is
      e.stopPropagation();
      this.hooks.create(kind);
    });
    return b;
  }

  private setOpen(kind: Kind, open: boolean) {
    if (open) this.open.add(kind);
    else this.open.delete(kind);
    try {
      localStorage.setItem(OPEN_KEY, JSON.stringify([...this.open]));
    } catch {
      // not remembered, then: no harm done
    }
  }

  private itemElement(item: LibraryItem): HTMLElement {
    const li = document.createElement("li");
    const button = document.createElement("button");
    button.type = "button";
    button.className = "library-item";
    button.textContent = item.title;
    if (item.kind === "storyElement") {
      const how = document.createElement("code");
      how.textContent = `#${item.name}`;
      button.append(" ", how);
    }
    const lines = [item.description];
    if (item.kind === "look" && item.styles.length) lines.push(`Styles: ${item.styles.join(", ")}`);
    if (item.problems.length) {
      button.classList.add("has-problem");
      lines.push(...item.problems.map((p) => `Problem: ${p}`));
    }
    button.title = lines.filter(Boolean).join("\n");
    button.addEventListener("click", () => this.hooks.open(item.path, item.title).catch((e) => this.hooks.error(String(e))));
    button.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      e.stopPropagation(); // not the Vault's own menu
      this.hooks.menu(item, e.clientX, e.clientY);
    });
    li.append(button);
    return li;
  }

  /** A library file edited on two Devices: the Author picks which version to keep. */
  private showConflicts(conflicts: Conflict[]) {
    this.conflicts.replaceChildren();
    for (const c of conflicts) {
      const box = document.createElement("div");
      box.className = "library-conflict";
      const text = document.createElement("p");
      text.textContent = `${c.path} was changed on two Devices. Which version do you keep?`;
      const button = (label: string, run: () => Promise<unknown>) => {
        const b = document.createElement("button");
        b.type = "button";
        b.textContent = label;
        b.addEventListener("click", () => run().then(() => this.refresh()).catch((e) => this.hooks.error(String(e))));
        return b;
      };
      box.append(text,
        button("This one", () => invoke("resolve_library", { copy: c.copy, keep: "main" })),
        button("The other", () => invoke("resolve_library", { copy: c.copy, keep: "copy" })),
        button("Open both", async () => {
          await this.hooks.open(c.path, this.titleOf(c.path));
          await this.hooks.open(c.copy, `${this.titleOf(c.path)} (other version)`);
        }));
      this.conflicts.append(box);
    }
  }

  /** Newer versions of the app's defaults: the Author compares, uses or keeps their own. */
  private showOffers(offers: Offer[]) {
    this.offers.replaceChildren();
    this.hooks.offered(offers.length);
    const kinds: Record<Kind, string> = { style: "Style", look: "Look", storyElement: "Story element" };
    for (const offer of offers) {
      const box = document.createElement("div");
      box.className = "library-offer";
      const text = document.createElement("p");
      text.textContent = `A newer version of the ${kinds[offer.kind]} ${offer.title} is available.`;
      const button = (label: string, run: () => Promise<unknown>) => {
        const b = document.createElement("button");
        b.type = "button";
        b.textContent = label;
        b.addEventListener("click", () => run().then(() => this.refresh()).catch((e) => this.hooks.error(String(e))));
        return b;
      };
      const use = async () => {
        await this.hooks.settle(); // what's typed in its tab goes to the trash with it
        await invoke("use_newer_default", { path: offer.path });
      };
      const keep = () => invoke("keep_default", { path: offer.path });
      box.append(text,
        button("Compare", async () => {
          const choice = await this.hooks.compare(offer, await invoke<CompareLine[]>("compare_default", { path: offer.path }));
          if (choice === "use") await use();
          else if (choice === "keep") await keep();
        }),
        button("Use it", use),
        button("Keep mine", keep));
      this.offers.append(box);
    }
  }

  /** Makes a new library file and opens it. Returns its path. */
  async create(kind: Kind, name: string, element?: NewElement) {
    const path = await invoke<string>("create_library", { kind, name, element: element ?? null });
    await this.refresh();
    await this.hooks.open(path, this.titleOf(path));
    return path;
  }
}
