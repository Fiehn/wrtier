// The Style library in the sidebar: the Vault's Looks, Styles and Story elements (files in
// `styles/`), the Conflict copies waiting for the Author to pick a version, and making new ones.

import { invoke } from "@tauri-apps/api/core";

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
type Library = { items: LibraryItem[]; conflicts: Conflict[]; builtIn: string[] };
export type NewElement = { title: string; shownAs: string; docx: string; description: string };

/** What the panel does with the rest of the page. */
export type LibraryHooks = {
  /** Opens a library file in a tab. */
  open(path: string, title: string): Promise<void>;
  error(message: string): void;
};

const GROUPS: { kind: Kind; label: string }[] = [
  { kind: "look", label: "Looks" },
  { kind: "style", label: "Styles" },
  { kind: "storyElement", label: "Story elements" },
];

export class LibraryPanel {
  items: LibraryItem[] = [];
  /** The built-in Story elements' names: every Work has them. */
  builtIn: string[] = [];

  constructor(
    private readonly list: HTMLElement,
    private readonly conflicts: HTMLElement,
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
    this.list.replaceChildren();
    for (const group of GROUPS) {
      const items = library.items.filter((i) => i.kind === group.kind);
      const heading = document.createElement("h3");
      heading.textContent = group.label;
      const ul = document.createElement("ul");
      for (const item of items) ul.append(this.itemElement(item));
      if (items.length === 0) {
        const none = document.createElement("li");
        none.className = "muted";
        none.textContent = "None yet";
        ul.append(none);
      }
      this.list.append(heading, ul);
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

  /** Makes a new library file and opens it. Returns its path. */
  async create(kind: Kind, name: string, element?: NewElement) {
    const path = await invoke<string>("create_library", { kind, name, element: element ?? null });
    await this.refresh();
    await this.hooks.open(path, this.titleOf(path));
    return path;
  }
}
