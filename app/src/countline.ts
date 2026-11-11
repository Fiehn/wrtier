// The word and character count Mod: a line under the editor with the Work's words and
// characters, and the selection's while text is selected. It catches up after a short pause, so
// a novel isn't counted again on every key. Included text (ADR 0015) counts as if it were
// written there, and the line says how many of the words are included.

import { countsLine, countsOf, wordsIn, type Counts } from "./counts";

/** Text as a reader sees it (Inclusions written out), and the included part on its own. */
export type ReaderText = { text: string; included: string };

export type CountHooks = {
  /** The Work's text as its editor has it now (null if it isn't open). */
  textOf(id: string): ReaderText | null;
  /** The selected text (empty if nothing is selected). */
  selectedText(id: string): ReaderText | null;
};

const SETTLE_MS = 300;

export class CountLine {
  /** The Work counted (in front, with the Mod in use here), or null: hidden. */
  private work: string | null = null;
  private all: Counts | null = null;
  /** Words in its Inclusions. */
  private included = 0;
  private timer: number | undefined;
  private selectionTimer: number | undefined;

  constructor(private readonly el: HTMLElement, private readonly hooks: CountHooks) {}

  show(work: string | null) {
    this.work = work;
    this.el.hidden = work === null;
    this.recount();
  }

  /** A Work's text changed. */
  edited(id: string) {
    if (id !== this.work) return;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => this.recount(), SETTLE_MS);
  }

  /** The cursor or selection moved. */
  selected(id: string) {
    if (id !== this.work) return;
    clearTimeout(this.selectionTimer);
    this.selectionTimer = window.setTimeout(() => this.draw(), 100);
  }

  private recount() {
    clearTimeout(this.timer);
    const read = this.work ? this.hooks.textOf(this.work) : null;
    this.all = read === null ? null : countsOf(read.text);
    this.included = read === null ? 0 : wordsIn(read.included);
    this.draw();
  }

  private draw() {
    if (!this.work || !this.all) {
      this.el.textContent = "";
      return;
    }
    const selection = this.hooks.selectedText(this.work);
    const selected = selection && selection.text ? countsOf(selection.text) : null;
    this.el.textContent = countsLine(this.all, selected, selected ? 0 : this.included);
    const without = (selected ?? this.all).charactersNoSpaces.toLocaleString();
    const included = this.included > 0 ? " Included text counts as if it were written here." : "";
    this.el.title = `Characters with spaces; ${without} without. Markup, comments and line breaks aren't counted.${included}`;
  }
}
