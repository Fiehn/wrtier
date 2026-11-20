// Marks (CONTEXT.md, the v2 plan): wavy underlines under text where a Mod found something, in
// three kinds shared by every Mod: a Mistake (red), a Rule break (yellow) and a Query (blue).
// They're the app's, not the Mods': a Mod only says where a Mark goes, its kind, why, its
// Suggestions and any actions of its own (`markSources`). Everything else is here, the same
// whichever Mod made a Mark: drawing it, the card (hover, click, right-click, tap, Ctrl+.), accepting a
// Suggestion (an ordinary edit by the Author, as if typed: one undo step), dismissing it (kept by
// the Vault, ADR 0018), going from one to the next (F8), and counting them.
//
// A Mod looks at the whole Work, but only once the typing pauses, so a Mark never flashes while
// a sentence is half written. Marks the typing since touched are dropped until then. A Mod may
// answer later (the Spellchecker asks Rust, ADR 0019); an answer about text that has changed
// since is dropped, and the Mods look again once the typing pauses. A Mark's Suggestions may
// also come later, asked for only when its card shows them.

import { Facet, Prec, StateEffect, StateField, type EditorState, type Extension, type Transaction } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, hoverTooltip, keymap, showTooltip, ViewPlugin, type Tooltip, type ViewUpdate } from "@codemirror/view";
import type { DismissedMark } from "./sync";

export type MarkLevel = "mistake" | "rule" | "query";

/** What a Mark's kind is called, in plain words. */
export const LEVEL_NAMES: Record<MarkLevel, string> = { mistake: "Mistake", rule: "Rule break", query: "Query" };

export type MarkEdit = { from: number; to: number; insert: string };

/** What the text reads like after it (`label`), and its edits. */
export type MarkSuggestion = { label: string; edits: MarkEdit[] };

export type MarkAction = { label: string; run: () => void };

export type Mark = {
  /** The Mod that made it (mods.ts) and its rule: together, what a dismissal is kept under. */
  mod: string;
  rule: string;
  level: MarkLevel;
  /** The characters under it; never empty. */
  from: number;
  to: number;
  /** One line in plain words. */
  why: string;
  suggestions: MarkSuggestion[];
  /**
   * Its Suggestions, if the Mod works them out only when they're shown (they take a while):
   * for the Mark where it is now in `state`. Once they're in, `suggestions` holds them.
   */
  suggest?: (mark: Mark, state: EditorState) => Promise<MarkSuggestion[]>;
  /** The Mod's own actions ("Treat “grinned” as a Speech verb", "Allow in Work"). */
  actions?: MarkAction[];
  /** What a dismissal is kept under instead of its rule, when several rules share one (the Spellchecker's). */
  dismissAs?: string;
  /** Dismissed only on exactly its characters, none of them deleted and none typed in (the Spellchecker's: a changed word is a new word). Otherwise on any of them. */
  dismissExact?: boolean;
  /** What Dismiss is called on its card, if it needs saying ("Dismiss here only"). */
  dismissLabel?: string;
};

/** A Mod that makes Marks: the Marks it finds in the editor's text, now or later. */
export type MarkSource = { mod: string; find(state: EditorState): Mark[] | Promise<Mark[]> };

/** The Mods making Marks in an editor. */
export const markSources = Facet.define<MarkSource>();

/** What the app does for the Marks (tabs.ts): dismissing goes to the Vault; counts go to the count line. */
export type MarkHooks = {
  dismiss(mark: Mark): void;
  /** Stops checking a Mark's rule in this Work (the setting "<mod>.rule.<rule>", "off"). */
  stopRule?(mark: Mark): void;
  counted?(counts: MarkCounts): void;
  /** The Marks shown changed (found, dismissed, moved by typing, Suggestions came in): the Marks panel follows. */
  changed?(): void;
};
export const markHooks = Facet.define<MarkHooks, MarkHooks | null>({ combine: (all) => all[0] ?? null });

export type MarkCounts = Record<MarkLevel, number>;

/** How long the typing must pause before the Mods look again. */
const SETTLE_MS = 800;

// ---- State ----

/** What the Mods found, as of their last look. (Exported for the tests.) */
export const setFound = StateEffect.define<Mark[]>();
/** Where the Dismissed Marks are, as the Vault last said (and any dismissed since). */
export const setDismissed = StateEffect.define<DismissedMark[]>();
const addDismissed = StateEffect.define<DismissedMark>();
/** A Mark's Suggestions came in (`Mark.suggest`). */
const setSuggestions = StateEffect.define<{ mark: Mark; suggestions: MarkSuggestion[] }>();

const keyOf = (m: Mark) => `${m.mod}/${m.rule}`;
const sameMark = (a: Mark, b: Mark) => keyOf(a) === keyOf(b) && a.from === b.from && a.to === b.to;

/** What dismissing a Mark is kept under: "<mod>/<rule>" (or the rule it's dismissed as). */
export const dismissKey = (m: Mark) => `${m.mod}/${m.dismissAs ?? m.rule}`;

/** The Marks, mapped through edits since the Mods last looked; one an edit touched is dropped until they look again. */
const foundField = StateField.define<Mark[]>({
  create: () => [],
  update(marks, tr) {
    for (const e of tr.effects) {
      if (e.is(setFound)) return e.value;
      if (e.is(setSuggestions)) {
        const { mark, suggestions } = e.value;
        marks = marks.map((m) => (sameMark(m, mark) ? { ...m, suggestions, suggest: undefined } : m));
      }
    }
    if (!tr.docChanged || marks.length === 0) return marks;
    const out: Mark[] = [];
    for (const m of marks) {
      let touched = false;
      tr.changes.iterChangedRanges((fromA, toA) => {
        if (fromA <= m.to && toA >= m.from) touched = true;
      });
      if (touched) continue;
      out.push(mapMark(m, tr));
    }
    return out;
  },
});

function mapMark(m: Mark, tr: Transaction): Mark {
  const map = (p: number, side: number) => tr.changes.mapPos(p, side);
  return {
    ...m,
    from: map(m.from, 1),
    to: map(m.to, -1),
    suggestions: m.suggestions.map((s) => ({ ...s, edits: s.edits.map((e) => ({ ...e, from: map(e.from, 1), to: Math.max(map(e.from, 1), map(e.to, -1)) })) })),
  };
}

const dismissedField = StateField.define<DismissedMark[]>({
  create: () => [],
  update(list, tr) {
    for (const e of tr.effects) {
      if (e.is(setDismissed)) list = e.value;
      if (e.is(addDismissed)) list = [...list, e.value];
    }
    if (!tr.docChanged) return list;
    return list.map((d) => {
      // Something deleted from it, or typed inside it: no longer whole (for exact dismissals).
      let whole = d.whole !== false;
      if (whole) {
        tr.changes.iterChangedRanges((fromA, toA) => {
          for (const [f, t] of d.ranges) if (fromA < t && toA > f && (toA > fromA || fromA > f)) whole = false;
        });
      }
      return {
        ...d,
        whole,
        ranges: d.ranges
          .map(([f, t]): [number, number] => [tr.changes.mapPos(f, 1), tr.changes.mapPos(t, -1)])
          .filter(([f, t]) => f < t),
      };
    });
  },
});

/**
 * Is this Mark dismissed: the same Mod and rule (or what it's dismissed as), on any of the same
 * characters; or, for an exact dismissal, on exactly them, all still there and nothing typed in.
 */
const isDismissed = (m: Mark, dismissed: DismissedMark[]) =>
  dismissed.some((d) => d.key === dismissKey(m) && (m.dismissExact
    ? d.whole !== false && d.ranges.length === 1 && d.ranges[0][0] === m.from && d.ranges[0][1] === m.to
    : d.ranges.some(([f, t]) => f < m.to && t > m.from)));

/** The Marks shown: those found, less those dismissed. In reading order. */
const shownField = StateField.define<Mark[]>({
  create: () => [],
  update(shown, tr) {
    const found = tr.state.field(foundField);
    const dismissed = tr.state.field(dismissedField);
    if (found === tr.startState.field(foundField) && dismissed === tr.startState.field(dismissedField)) return shown;
    return found.filter((m) => !isDismissed(m, dismissed)).sort((a, b) => a.from - b.from || a.to - b.to);
  },
  provide: (f) => EditorView.decorations.from(f, (marks): DecorationSet =>
    Decoration.set(marks.map((m) => LOOK[m.level].range(m.from, m.to)), true)),
});

const LOOK: Record<MarkLevel, Decoration> = {
  mistake: Decoration.mark({ class: "cm-mark cm-mark-mistake" }),
  rule: Decoration.mark({ class: "cm-mark cm-mark-rule" }),
  query: Decoration.mark({ class: "cm-mark cm-mark-query" }),
};

/** The Marks shown in an editor, in reading order. */
export const marksOf = (state: EditorState): Mark[] => state.field(shownField, false) ?? [];

/** The Mark at `pos` (the innermost, if several), or null. */
export function markAt(state: EditorState, pos: number): Mark | null {
  let best: Mark | null = null;
  for (const m of marksOf(state)) {
    if (m.from <= pos && pos <= m.to && (!best || m.to - m.from < best.to - best.from)) best = m;
  }
  return best;
}

export function countsOf(state: EditorState): MarkCounts {
  const counts: MarkCounts = { mistake: 0, rule: 0, query: 0 };
  for (const m of marksOf(state)) counts[m.level]++;
  return counts;
}

// ---- Looking ----

/** Asks every Mod for its Marks once the typing has paused (and at once when they change). */
const looker = ViewPlugin.fromClass(class {
  timer: number | undefined;
  lastCounts = "";

  constructor(readonly view: EditorView) {
    this.schedule(0);
  }

  update(u: ViewUpdate) {
    if (u.state.field(shownField) !== u.startState.field(shownField)) u.state.facet(markHooks)?.changed?.();
    if (u.docChanged) this.schedule(SETTLE_MS);
    else if (u.startState.facet(markSources) !== u.state.facet(markSources)) this.schedule(0);
    const counts = countsOf(u.state);
    const key = JSON.stringify(counts);
    if (key !== this.lastCounts) {
      this.lastCounts = key;
      u.state.facet(markHooks)?.counted?.(counts);
    }
  }

  schedule(ms: number) {
    clearTimeout(this.timer);
    this.looks++; // an answer still to come from an earlier look is no longer wanted
    this.timer = window.setTimeout(() => this.look(), ms);
  }

  /** How many looks have started: an answer that comes after a later look started is dropped. */
  looks = 0;
  destroyed = false;

  look() {
    const state = this.view.state;
    const look = ++this.looks;
    const fits = (m: Mark) => m.from < m.to && m.to <= state.doc.length;
    const failed = (mod: string, e: unknown) => {
      console.error(`The ${mod} Mod couldn't look at the text:`, e);
      return [] as Mark[];
    };
    const answers = state.facet(markSources).map((source): Mark[] | Promise<Mark[]> => {
      try {
        const found = source.find(state);
        return found instanceof Promise ? found.then((ms) => ms.filter(fits), (e) => failed(source.mod, e)) : found.filter(fits);
      } catch (e) {
        return failed(source.mod, e);
      }
    });
    const show = (all: Mark[][]) => {
      // The text changed meanwhile: these places are stale, and the Mods look again once the typing pauses.
      if (this.destroyed || look !== this.looks || this.view.state.doc !== state.doc) return;
      this.view.dispatch({ effects: setFound.of(all.flat()) });
    };
    if (answers.some((a) => a instanceof Promise)) void Promise.all(answers).then(show);
    else show(answers as Mark[][]);
  }

  destroy() {
    this.destroyed = true;
    clearTimeout(this.timer);
    this.view.state.facet(markHooks)?.counted?.({ mistake: 0, rule: 0, query: 0 });
  }
});

// ---- Acting on a Mark ----

/**
 * Accepts a Suggestion: an edit by the Author, exactly as if typed, one undo step. `focus`: the
 * editor takes the focus (not from the Marks panel, which works away from it).
 */
export function accept(view: EditorView, mark: Mark, n: number, focus = true) {
  const s = mark.suggestions[n];
  if (!s) return;
  view.dispatch({ changes: s.edits, userEvent: "input.type", effects: closeCard.of(null) });
  if (focus) view.focus();
}

/** Dismisses a Mark: it goes now, and the Vault keeps it (ADR 0018). */
export function dismiss(view: EditorView, mark: Mark, focus = true) {
  view.dispatch({ effects: [addDismissed.of({ key: dismissKey(mark), ranges: [[mark.from, mark.to]], whole: true }), closeCard.of(null)] });
  view.state.facet(markHooks)?.dismiss(mark);
  if (focus) view.focus();
}

/** Runs one of the Mod's own actions on a Mark, and closes its card. */
export function runAction(view: EditorView, action: MarkAction) {
  action.run();
  view.dispatch({ effects: closeCard.of(null) });
}

/** Is `mark` still shown where it was (no typing since moved or dropped it)? */
export const stillShown = (state: EditorState, mark: Mark) => marksOf(state).some((m) => sameMark(m, mark));

/**
 * The Mark with its Suggestions: at once if it has them, else asked of its Mod, and kept with it
 * (so the card's keys and the menu offer them). A Mod that can't answer offers none.
 */
export async function withSuggestions(view: EditorView, mark: Mark): Promise<Mark> {
  if (!mark.suggest) return mark;
  let suggestions: MarkSuggestion[] = [];
  try {
    suggestions = await mark.suggest(mark, view.state);
  } catch (e) {
    console.error(`The ${mark.mod} Mod couldn't suggest anything:`, e);
  }
  const done = { ...mark, suggestions, suggest: undefined };
  // Still where it was (no typing meanwhile): kept with it.
  if (marksOf(view.state).some((m) => sameMark(m, mark))) view.dispatch({ effects: setSuggestions.of({ mark, suggestions }) });
  return done;
}

// ---- The card ----

/** The card open on a Mark (by click, Ctrl+. or F8); `keys`: opened from the keyboard, so 1, 2, D and Esc act on it. */
type Card = { mark: Mark; keys: boolean };
const openCard = StateEffect.define<Card>();
const closeCard = StateEffect.define<null>();

const cardField = StateField.define<Card | null>({
  create: () => null,
  update(card, tr) {
    for (const e of tr.effects) {
      if (e.is(openCard)) return e.value;
      if (e.is(closeCard)) return null;
    }
    if (!card) return null;
    if (tr.docChanged) return null;
    // Still shown? (A new look may have dropped it or moved it.)
    const now = marksOf(tr.state).find((m) => sameMark(m, card.mark));
    if (!now) return null;
    const head = tr.state.selection.main.head;
    if (tr.selection && (head < now.from || head > now.to)) return null;
    return now === card.mark ? card : { ...card, mark: now };
  },
  provide: (f) => showTooltip.from(f, (card): Tooltip | null => card && {
    pos: card.mark.from,
    above: false,
    create: (view) => ({ dom: cardDom(view, card.mark, card.keys) }),
  }),
});

function cardDom(view: EditorView, mark: Mark, keys: boolean): HTMLElement {
  const dom = document.createElement("div");
  dom.className = `cm-mark-card cm-mark-card-${mark.level}`;
  const head = document.createElement("p");
  head.className = "cm-mark-why";
  const kind = document.createElement("strong");
  kind.textContent = `${LEVEL_NAMES[mark.level]}: `;
  head.append(kind, mark.why);
  dom.append(head);
  const suggestions = document.createElement("div");
  suggestions.className = "cm-mark-suggestions";
  const fill = (m: Mark) => {
    suggestions.replaceChildren(...m.suggestions.map((s, i) => {
      const b = button(s.label, () => accept(view, m, i), "cm-mark-suggestion");
      if (keys && i < 9) b.prepend(keyHint(String(i + 1)));
      return b;
    }));
  };
  if (mark.suggest) {
    // Asked for now; the card shows them when they come (a keyboard card is drawn again then).
    suggestions.append(line("cm-mark-looking", "Looking for suggestions…"));
    void withSuggestions(view, mark).then((m) => {
      if (!dom.isConnected) return;
      fill(m);
      if (m.suggestions.length === 0) suggestions.append(line("cm-mark-looking", "No suggestions."));
    });
  } else fill(mark);
  dom.append(suggestions);
  const row = document.createElement("div");
  row.className = "cm-mark-row";
  for (const action of mark.actions ?? []) row.append(button(action.label, () => runAction(view, action), "cm-mark-action"));
  const d = button(mark.dismissLabel ?? "Dismiss", () => dismiss(view, mark), "cm-mark-dismiss");
  if (keys) d.prepend(keyHint("D"));
  row.append(d);
  dom.append(row);
  const hooks = view.state.facet(markHooks);
  if (hooks?.stopRule) {
    const stop = button("Stop checking this rule in this Work", () => {
      view.dispatch({ effects: closeCard.of(null) });
      hooks.stopRule!(mark);
    }, "cm-mark-stop");
    dom.append(stop);
  }
  return dom;
}

function button(label: string, run: () => void, cls: string): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.className = cls;
  b.textContent = label;
  // Keep the editor's selection: act on mousedown's click without taking the focus first.
  b.addEventListener("mousedown", (e) => e.preventDefault());
  b.addEventListener("click", (e) => {
    e.preventDefault();
    run();
  });
  return b;
}

function line(cls: string, text: string): HTMLElement {
  const p = document.createElement("p");
  p.className = cls;
  p.textContent = text;
  return p;
}

function keyHint(key: string): HTMLElement {
  const k = document.createElement("kbd");
  k.textContent = key;
  return k;
}

/** Hovering a Mark (with no card open) shows the same card, without its keys. */
const hover = hoverTooltip((view, pos, side) => {
  if (view.state.field(cardField)) return null;
  const mark = markAt(view.state, pos);
  if (!mark || (pos === mark.to && side > 0) || (pos === mark.from && side < 0)) return null;
  return { pos: mark.from, end: mark.to, above: false, create: (v) => ({ dom: cardDom(v, mark, false) }) };
}, { hideOnChange: true, hoverTime: 400 });

/** Opens the card on the Mark at the cursor (Ctrl+.). False if there's none. */
export function openCardAtCursor(view: EditorView): boolean {
  const head = view.state.selection.main.head;
  const mark = markAt(view.state, head);
  if (!mark) return false;
  view.dispatch({ effects: openCard.of({ mark, keys: true }) });
  return true;
}

/** Right-click on a Mark at `pos`: the cursor goes there and its card opens, as a click opens it. False if there's no Mark. */
export function openCardAt(view: EditorView, pos: number): boolean {
  const mark = markAt(view.state, pos);
  if (!mark) return false;
  view.dispatch({ selection: { anchor: pos }, effects: openCard.of({ mark, keys: false }) });
  return true;
}

/** Goes to a Mark in the editor and opens its card (from the Marks panel). */
export function showMark(view: EditorView, mark: Mark) {
  view.dispatch({
    selection: { anchor: mark.from },
    effects: [openCard.of({ mark, keys: true }), EditorView.scrollIntoView(mark.from, { y: "center" })],
  });
  view.focus();
}

/** Goes to the next (or previous) Mark of any Mod, or of one kind, and opens its card (F8). False if there are none. */
export function stepMark(view: EditorView, dir: 1 | -1, level?: MarkLevel): boolean {
  const marks = marksOf(view.state).filter((m) => !level || m.level === level);
  if (marks.length === 0) return false;
  const head = view.state.selection.main.head;
  const next = dir > 0
    ? marks.find((m) => m.from > head) ?? marks[0]
    : [...marks].reverse().find((m) => m.from < head) ?? marks[marks.length - 1];
  view.dispatch({
    selection: { anchor: next.from },
    effects: [openCard.of({ mark: next, keys: true }), EditorView.scrollIntoView(next.from, { y: "center" })],
  });
  view.focus();
  return true;
}

/** While a card opened from the keyboard is up: 1–9 accept, D dismisses, Esc closes. */
const cardKeys = Prec.highest(keymap.of([
  ...Array.from({ length: 9 }, (_, i) => ({
    key: String(i + 1),
    run: (view: EditorView) => {
      const card = view.state.field(cardField);
      if (!card?.keys || !card.mark.suggestions[i]) return false;
      accept(view, card.mark, i);
      return true;
    },
  })),
  {
    key: "d",
    run: (view: EditorView) => {
      const card = view.state.field(cardField);
      if (!card?.keys) return false;
      dismiss(view, card.mark);
      return true;
    },
  },
  {
    key: "Escape",
    run: (view: EditorView) => {
      if (!view.state.field(cardField)) return false;
      view.dispatch({ effects: closeCard.of(null) });
      return true;
    },
  },
]));

/** A click (or a tap) on a Mark opens its card. */
const clicks = EditorView.domEventHandlers({
  click(e, view) {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return false;
    const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
    if (pos === null) return false;
    const mark = markAt(view.state, pos);
    if (!mark || !view.state.selection.main.empty) return false;
    view.dispatch({ effects: openCard.of({ mark, keys: false }) });
    return false;
  },
});

/**
 * Marks in an editor: always there in a Work's editor, and quiet until a Mod adds a source.
 * `hooks`: what the app does with dismissals and counts.
 */
export function marks(hooks: MarkHooks): Extension {
  return [foundField, dismissedField, shownField, cardField, looker, hover, cardKeys, clicks, markHooks.of(hooks)];
}
