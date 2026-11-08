// The Preamble dialog: a Work's Looks (in order: the first is its own), its extra Styles, its
// own Story elements, its settings and its raw Typst. It opens from the Work's "Preamble…"
// button, its right-click menu, or Ctrl+Shift+L while writing, open or not.

import { invoke } from "@tauri-apps/api/core";
import type { LibraryItem } from "./library";
import type { WorkState } from "./sync";

// The same shapes as the Rust side's (preamble.rs, vault.rs).
export type Preamble = {
  looks: string[];
  styles: string[];
  elements: string[];
  title: string;
  author: string;
  language: string;
  raw: string;
};
type PreambleView = { preamble: Preamble; rawOther: string | null };

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

/** One list in the dialog: Looks, extra Styles or Story elements. */
class NameList {
  names: string[] = [];

  constructor(
    private readonly list: HTMLElement,
    private readonly picker: HTMLSelectElement,
    addButton: HTMLElement,
    /** Can the order be changed (only the Looks')? */
    private readonly ordered: boolean,
    /** What the library has to add, and what's shown for each. */
    private choices: () => LibraryItem[],
    private readonly empty: string,
  ) {
    addButton.addEventListener("click", () => {
      if (this.picker.value) this.set([...this.names, this.picker.value]);
    });
  }

  set(names: string[]) {
    this.names = names;
    this.render();
  }

  private render() {
    const items = this.choices();
    const title = (name: string) => items.find((i) => i.name === name)?.title ?? name;
    this.list.replaceChildren();
    this.names.forEach((name, i) => {
      const li = document.createElement("li");
      const label = document.createElement("span");
      label.textContent = title(name);
      if (!items.some((item) => item.name === name)) {
        label.classList.add("missing");
        label.title = "Not in the Style library: the Work uses its own copy, if it has one.";
      }
      if (this.ordered && i === 0) {
        const own = document.createElement("small");
        own.textContent = " (this Work's own)";
        label.append(own);
      }
      li.append(label);
      const button = (text: string, hint: string, run: () => void, disabled = false) => {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "link";
        b.textContent = text;
        b.title = hint;
        b.disabled = disabled;
        b.addEventListener("click", run);
        li.append(b);
      };
      if (this.ordered) {
        button("↑", "Move up", () => this.move(i, -1), i === 0);
        button("↓", "Move down", () => this.move(i, 1), i === this.names.length - 1);
      }
      button("Remove", `Remove ${title(name)}`, () => this.set(this.names.filter((_, j) => j !== i)));
      this.list.append(li);
    });
    if (this.names.length === 0) {
      const li = document.createElement("li");
      li.className = "muted";
      li.textContent = this.empty;
      this.list.append(li);
    }
    // What can still be added.
    this.picker.replaceChildren();
    const left = items.filter((i) => !this.names.includes(i.name));
    for (const item of left) {
      const option = document.createElement("option");
      option.value = item.name;
      option.textContent = item.title;
      option.title = item.description;
      this.picker.append(option);
    }
    this.picker.disabled = left.length === 0;
    this.picker.nextElementSibling?.toggleAttribute("disabled", left.length === 0);
  }

  private move(i: number, by: number) {
    const names = [...this.names];
    [names[i], names[i + by]] = [names[i + by], names[i]];
    this.set(names);
  }
}

export class PreambleDialog {
  private library: LibraryItem[] = [];
  private readonly looks: NameList;
  private readonly styles: NameList;
  private readonly elements: NameList;
  /** The built-in Story elements, which every Work has. */
  private builtIn: string[] = [];

  constructor(private readonly ask: (dialog: HTMLDialogElement) => Promise<string>) {
    const of = (kind: LibraryItem["kind"]) => () => this.library.filter((i) => i.kind === kind);
    this.looks = new NameList($("preamble-looks"), $("preamble-add-look"), $("preamble-add-look-button"), true, of("look"),
      "No Look: the Work is set in Typst's plain defaults.");
    this.styles = new NameList($("preamble-styles"), $("preamble-add-style"), $("preamble-add-style-button"), false, of("style"), "None");
    this.elements = new NameList($("preamble-elements"), $("preamble-add-element"), $("preamble-add-element-button"), false,
      () => of("storyElement")().filter((i) => !this.builtIn.includes(i.name)), "None of your own yet");
    $("preamble-use-other").addEventListener("click", () => {
      $<HTMLTextAreaElement>("preamble-raw").value = $<HTMLTextAreaElement>("preamble-raw-other-text").value;
    });
  }

  /**
   * Opens the dialog for a Work. `library`: the Style library's items; `builtIn`: the
   * built-in Story elements' names. Resolves with the open Work's new state if it was saved
   * (null if the Work isn't open), or undefined if cancelled.
   */
  async open(id: string, name: string, library: LibraryItem[], builtIn: string[]): Promise<WorkState | null | undefined> {
    this.library = library;
    this.builtIn = builtIn;
    const view = await invoke<PreambleView>("get_preamble", { id });
    const p = view.preamble;
    $("preamble-heading").textContent = `Preamble of ${name}`;
    const titles = builtIn.map((n) => library.find((i) => i.name === n)?.title ?? n);
    $("preamble-built-in").textContent = `The built-in ones are always there (${titles.join(", ")}). Add your own:`;
    this.looks.set(p.looks);
    this.styles.set(p.styles);
    this.elements.set(p.elements);
    $<HTMLInputElement>("preamble-title").value = p.title;
    $<HTMLInputElement>("preamble-author").value = p.author;
    $<HTMLInputElement>("preamble-language").value = p.language;
    $<HTMLTextAreaElement>("preamble-raw").value = p.raw;
    $("preamble-raw-other").hidden = view.rawOther === null;
    $<HTMLTextAreaElement>("preamble-raw-other-text").value = view.rawOther ?? "";
    for (;;) {
      if ((await this.ask($<HTMLDialogElement>("preamble"))) !== "save") return undefined;
      const preamble: Preamble = {
        looks: this.looks.names,
        styles: this.styles.names,
        elements: this.elements.names,
        title: $<HTMLInputElement>("preamble-title").value,
        author: $<HTMLInputElement>("preamble-author").value,
        language: $<HTMLInputElement>("preamble-language").value.trim().toLowerCase(),
        raw: $<HTMLTextAreaElement>("preamble-raw").value,
      };
      try {
        return await invoke<WorkState | null>("set_preamble", { id, preamble });
      } catch (e) {
        // Keep what was entered and ask again (e.g. a language code that isn't one).
        window.alert(String(e));
      }
    }
  }
}
