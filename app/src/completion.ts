// Completions: typing `#` lists what can go there, narrowing as you type. The list comes from
// Typst's own autocompleter on the Rust side, so it knows everything in scope at the cursor.
// The Work's Story elements come first; Typst's own functions follow in their own group.

import { invoke } from "@tauri-apps/api/core";
import { autocompletion, snippet, type Completion, type CompletionContext, type CompletionResult } from "@codemirror/autocomplete";

// The same shapes as the Rust side's (typeset.rs).
type Item = { label: string; apply: string; detail: string | null; kind: string; storyElement: boolean };
type Completions = { from: number; items: Item[] };

/** What the completions need from the Work they're in. */
export type CompletionHooks = {
  /** The Work's ID now (it changes when the Work is renamed). */
  work(): string;
  /** Resolves once everything typed has reached Rust, so it completes the text as it is. */
  settled(): Promise<void>;
};

const STORY = { name: "Story elements", rank: 0 };
const TYPST = { name: "Typst", rank: 1 };

/** A `#` and what's been typed after it: only then is Rust asked (not on every keystroke of prose). */
const AFTER_HASH = /#[\p{L}\p{N}_.-]*$/u;

/** One item as CodeMirror shows it. Typst's templates use the same `${}` fields as CodeMirror's snippets. */
export function toCompletion(item: Item): Completion {
  const hasFields = item.apply.includes("${") && !item.apply.includes("#{");
  return {
    label: item.label,
    detail: item.storyElement ? "Story element" : undefined,
    info: item.detail ?? undefined,
    type: item.kind === "function" ? "function" : item.kind === "syntax" ? "keyword" : item.kind,
    section: item.storyElement ? STORY : TYPST,
    boost: item.storyElement ? 50 : 0,
    apply: hasFields ? snippet(item.apply) : item.apply.replace(/\$\{[^}]*\}/g, ""),
  };
}

/** Typst's completions at the cursor, after a `#` (or anywhere when asked with Ctrl+Space). */
function source(hooks: CompletionHooks) {
  return async (context: CompletionContext): Promise<CompletionResult | null> => {
    if (!context.explicit && !context.matchBefore(AFTER_HASH)) return null;
    await hooks.settled();
    if (context.aborted) return null;
    const found = await invoke<Completions | null>("complete_at", { id: hooks.work(), at: context.pos, explicit: context.explicit });
    if (!found || found.items.length === 0 || context.aborted) return null;
    // Typing more letters narrows the list here, without asking Rust again.
    return { from: found.from, options: found.items.map(toCompletion), validFor: /^[\p{L}\p{N}_-]*$/u };
  };
}

/** The editor extension: the list opens by itself after a `#`. */
export function completions(hooks: CompletionHooks) {
  return autocompletion({
    override: [source(hooks)],
    activateOnTyping: true,
    icons: false,
    maxRenderedOptions: 80,
  });
}
