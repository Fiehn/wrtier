// The Find and replace bar at the top of the editor (find.ts does the matching). It works in the
// tab in front: a Work, or a Style library file. Ctrl+F opens it on Find, Ctrl+H with Replace
// showing; Enter and Shift+Enter step through the matches, Escape closes it with the current
// match selected. The search stays for the session and is run again in another tab or view.

import { EditorView } from "@codemirror/view";
import { findState, onFindChange, replacementChanges, setCurrent, setFind, type FindMatch, type Query } from "./find";
import { unfoldInclusion } from "./inclusions";
import { replacing } from "./sync";
import { keyLabel } from "./keys";

export type FindHooks = {
  /** The editor in front, and its Work's ID (null: a Style library file); null if no tab is open. */
  target(): { view: EditorView; work: string | null } | null;
  /** "Open Rain" on a match in included text: opens that Work near it. */
  openWork(work: string, at: number): void;
};

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

export class FindBar {
  private readonly bar = $("find-bar");
  private readonly input = $<HTMLInputElement>("find-input");
  private readonly replaceInput = $<HTMLInputElement>("replace-input");
  private readonly replaceRow = $("find-replace-row");
  private readonly toggle = $<HTMLButtonElement>("find-toggle-replace");
  private readonly count = $("find-count");
  private readonly matchCase = $<HTMLInputElement>("find-case");
  private readonly wholeWords = $<HTMLInputElement>("find-words");
  private readonly scopeBox = $<HTMLInputElement>("find-scope");
  private readonly scopeLabel = $("find-scope-label");
  private readonly note = $("find-note");
  private readonly replaceOne = $<HTMLButtonElement>("replace-one");
  private readonly replaceAll = $<HTMLButtonElement>("replace-all");
  /** The editor it's finding in. */
  private view: EditorView | null = null;
  /** "In selection": the selection over several lines when it opened. */
  private scope: { from: number; to: number } | null = null;
  /** What Replace all said, until the search changes. */
  private said: string | null = null;

  constructor(private readonly hooks: FindHooks) {
    this.input.addEventListener("input", () => this.run());
    this.matchCase.addEventListener("change", () => this.run());
    this.wholeWords.addEventListener("change", () => this.run());
    this.scopeBox.addEventListener("change", () => this.run());
    $("find-next").addEventListener("click", () => this.step(1));
    $("find-prev").addEventListener("click", () => this.step(-1));
    $("find-close").addEventListener("click", () => this.close());
    this.toggle.addEventListener("click", () => {
      this.setReplace(this.replaceRow.hidden === true);
      if (!this.replaceRow.hidden) this.replaceInput.focus();
    });
    this.replaceOne.addEventListener("click", () => this.replace());
    this.replaceAll.addEventListener("click", () => this.replaceEverything());
    this.bar.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        this.close();
      } else if (e.key === "Enter" && e.target === this.input) {
        e.preventDefault();
        this.step(e.shiftKey ? -1 : 1);
      } else if (e.key === "Enter" && e.target === this.replaceInput) {
        e.preventDefault();
        if (e.ctrlKey || e.metaKey) this.replaceEverything();
        else this.replace();
      }
    });
    // The matches change as the text does (typing, merges, the other view): the count follows.
    onFindChange((view) => {
      if (view === this.view) this.show();
    });
  }

  get isOpen(): boolean {
    return !this.bar.hidden;
  }

  /**
   * Ctrl+F (or Ctrl+H, `withReplace`). A selection within one line is what to find; one over
   * several lines is where to look ("In selection"). With `query` (Find in Vault's "Open with
   * Find and replace"), that's what to find instead, in the whole Work.
   */
  open(withReplace: boolean, query?: Query) {
    const target = this.hooks.target();
    if (!target) return;
    this.switchTo(target.view);
    const { state } = target.view;
    const selection = state.selection.main;
    this.scope = null;
    if (query) {
      this.input.value = query.text;
      this.matchCase.checked = query.matchCase;
      this.wholeWords.checked = query.wholeWords;
    } else if (!selection.empty) {
      if (state.doc.lineAt(selection.from).number === state.doc.lineAt(selection.to).number) {
        this.input.value = state.sliceDoc(selection.from, selection.to);
      } else {
        this.scope = { from: selection.from, to: selection.to };
      }
    }
    this.scopeLabel.hidden = !this.scope;
    this.scopeBox.checked = !!this.scope;
    this.bar.hidden = false;
    if (withReplace) this.setReplace(true);
    this.showKeys();
    this.run();
    const field = withReplace && this.input.value ? this.replaceInput : this.input;
    field.focus();
    field.select();
  }

  /** Closes the bar; the current match in the text is left selected. */
  close(select = true) {
    if (this.bar.hidden) return;
    this.bar.hidden = true;
    const view = this.view;
    this.view = null;
    if (!view) return;
    const match = this.current(view);
    this.clear(view);
    if (select && match && !match.included) {
      view.dispatch({ selection: { anchor: match.from, head: match.to }, scrollIntoView: true });
    }
    view.focus();
  }

  /** Another tab came to the front (or none): the search runs again there. */
  attach() {
    if (this.bar.hidden) return;
    const target = this.hooks.target();
    if (!target) {
      this.close(false);
      return;
    }
    if (target.view === this.view) return;
    this.switchTo(target.view);
    this.scope = null;
    this.scopeLabel.hidden = true;
    this.scopeBox.checked = false;
    this.run();
  }

  /** Key names in the buttons' hints follow the Author's keys. */
  showKeys() {
    const replaceKey = keyLabel("replace");
    this.toggle.title = replaceKey ? `Replace (${replaceKey})` : "Replace";
  }

  private switchTo(view: EditorView) {
    if (this.view && this.view !== view) this.clear(this.view);
    this.view = view;
  }

  /** Stops finding in an editor (one being closed may be gone already). */
  private clear(view: EditorView) {
    try {
      view.dispatch({ effects: setFind.of({ query: null, scope: null }) });
    } catch {
      // destroyed with its tab
    }
  }

  private setReplace(on: boolean) {
    this.replaceRow.hidden = !on;
    this.toggle.setAttribute("aria-expanded", String(on));
    this.toggle.textContent = on ? "▾" : "▸";
  }

  private query(): Query | null {
    const text = this.input.value;
    return text ? { text, matchCase: this.matchCase.checked, wholeWords: this.wholeWords.checked } : null;
  }

  /** Searches again (the query or its options changed), from the cursor. */
  private run() {
    const view = this.view;
    if (!view) return;
    const before = findState(view.state);
    const scope = this.scopeBox.checked ? (before.scope ?? this.scope) : null;
    if (scope) this.scope = scope;
    this.said = null;
    view.dispatch({ effects: setFind.of({ query: this.query(), scope }) });
    this.show();
    this.reveal();
  }

  private current(view: EditorView): FindMatch | null {
    const { matches, current } = findState(view.state);
    return matches[current] ?? null;
  }

  /** The count, the note, and which buttons work. */
  private show() {
    const view = this.view;
    if (!view) return;
    const { query, matches, current } = findState(view.state);
    this.count.textContent = !query ? "" : matches.length === 0 ? "No matches" : `${current + 1} of ${matches.length.toLocaleString()}`;
    const match = matches[current] ?? null;
    const own = matches.some((m) => !m.included);
    this.replaceOne.disabled = !match || !!match.included;
    this.replaceAll.disabled = !own;
    this.note.replaceChildren();
    if (match?.included) {
      // Included text is changed only in its own Work.
      const { label, work, starts, from } = match.included;
      this.note.append(`In ${label}: open it to change it. `);
      if (work) {
        const open = document.createElement("button");
        open.type = "button";
        open.className = "link";
        open.textContent = `Open ${label}`;
        open.addEventListener("click", () => this.hooks.openWork(work, starts + from));
        this.note.append(open);
      }
    } else if (this.said) {
      this.note.append(this.said);
    }
    this.note.hidden = this.note.childNodes.length === 0;
  }

  /** Brings the current match into view (unfolding its Inclusion, if it's in one). */
  private reveal() {
    const view = this.view;
    const match = view && this.current(view);
    if (!view || !match) return;
    if (!match.included) {
      view.dispatch({ effects: EditorView.scrollIntoView(match.from, { y: "center" }) });
      return;
    }
    view.dispatch({ effects: [unfoldInclusion.of(match.included.at), EditorView.scrollIntoView(match.from, { y: "center" })] });
    // The block is drawn once it's in view; then the match inside it.
    requestAnimationFrame(() => view.dom.querySelector(".cm-inclusion .cm-find-current, .cm-inclusion-raw .cm-find-current")?.scrollIntoView({ block: "center" }));
  }

  private step(delta: number) {
    const view = this.view;
    if (!view) return;
    const { matches, current } = findState(view.state);
    if (matches.length === 0) return;
    view.dispatch({ effects: setCurrent.of(current + delta) });
    this.reveal();
  }

  /** Replace: the current match, then on to the next. */
  private replace() {
    const view = this.view;
    const query = this.query();
    if (!view || !query) return;
    const { matches, current, reading } = findState(view.state);
    const match = matches[current];
    if (!match || match.included || !reading) return;
    const changes = replacementChanges(reading, match, this.replaceInput.value, query.matchCase);
    if (changes.length > 0) {
      const tr = view.state.update({
        changes,
        userEvent: "replace",
        annotations: replacing.of({ find: query.text, with: this.replaceInput.value, count: 1 }),
      });
      view.dispatch(tr);
      // On from just after it (a replacement that matches again isn't found again).
      const after = tr.changes.mapPos(match.to, 1);
      const next = findState(view.state).matches.findIndex((m) => m.from >= after);
      view.dispatch({ effects: setCurrent.of(next === -1 ? 0 : next) });
    } else {
      view.dispatch({ effects: setCurrent.of(current + 1) });
    }
    this.reveal();
  }

  /** Replace all: every match in the Work's own text, as one action. Included text is left alone. */
  private replaceEverything() {
    const view = this.view;
    const query = this.query();
    if (!view || !query) return;
    const { matches, reading } = findState(view.state);
    const own = matches.filter((m) => !m.included);
    if (!reading || own.length === 0) return;
    const changes = own.flatMap((m) => replacementChanges(reading, m, this.replaceInput.value, query.matchCase));
    if (changes.length > 0) {
      view.dispatch({
        changes,
        userEvent: "replace",
        annotations: replacing.of({ find: query.text, with: this.replaceInput.value, count: own.length }),
      });
    }
    const left = new Map<string, number>();
    for (const m of matches) if (m.included) left.set(m.included.label, (left.get(m.included.label) ?? 0) + 1);
    const leftCount = [...left.values()].reduce((a, b) => a + b, 0);
    this.said = changes.length === 0 ? "They already read like that: nothing changed." : `Replaced ${own.length.toLocaleString()}.`;
    if (leftCount > 0) this.said += ` Left ${leftCount.toLocaleString()} in included text alone (${[...left].map(([label, n]) => `${label} ${n}`).join(", ")}).`;
    this.show();
    // The button pressed is now off (nothing left to replace): keep the focus in the bar.
    if (this.replaceAll.disabled) this.replaceInput.focus();
  }
}
