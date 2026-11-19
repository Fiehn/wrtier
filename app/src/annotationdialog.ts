// An Annotation's own small editor (ADR 0020): its text, Typst markup in the Formatted view,
// saved whole as one Change when it's closed with Save. One another Work made can only be read
// here, and annotated in turn (the Editors on one of Truant's notes).

import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { history, historyKeymap, standardKeymap } from "@codemirror/commands";
import { formattedView } from "./formatted";
import { dismissKey, marks, setDismissed } from "./marks";
import type { DismissedMark } from "./sync";

export type AnnotationAnswer =
  | { kind: "save"; body: string }
  | { kind: "delete" }
  /** Annotate this one (another Work's) in turn. */
  | { kind: "annotate" }
  /** A Comment on the whole Annotation. */
  | { kind: "comment" };

export type AnnotationAsk = {
  /** "Annotation on “the Navidson Record”", "Truant's Annotation". */
  title: string;
  /** What it covers, or the Annotation it's on. */
  quote: string;
  body: string;
  /** It can be changed here (this Work's own, or a new one). */
  editable: boolean;
  /** It exists already (so it can be deleted). */
  existing: boolean;
  /** Another Work's: offer to annotate it. */
  canAnnotate?: boolean;
  /** Offer a Comment on it. */
  canComment?: boolean;
  /** The Work's Mods in its editor (its language's Spellchecker, the Dialogue checker), as in the Work's text. */
  mods?: Extension;
};

function button(label: string, onClick: () => void, primary = false): HTMLButtonElement {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = label;
  if (primary) b.className = "primary";
  b.addEventListener("click", onClick);
  return b;
}

/** Shows the Annotation; resolves with what the Author did (null: closed without saving). */
export function askAnnotation(ask: AnnotationAsk): Promise<AnnotationAnswer | null> {
  return new Promise((resolve) => {
    const dialog = document.createElement("dialog");
    dialog.className = "annotation-dialog";
    const heading = document.createElement("p");
    heading.className = "annotation-title";
    heading.textContent = ask.title;
    dialog.append(heading);
    if (ask.quote) {
      const quote = document.createElement("blockquote");
      quote.className = "annotation-quote";
      quote.textContent = ask.quote.length > 240 ? `${ask.quote.slice(0, 240)}…` : ask.quote;
      dialog.append(quote);
    }
    let answer: AnnotationAnswer | null = null;
    const finish = (a: AnnotationAnswer | null) => {
      answer = a;
      dialog.close();
    };
    let view: EditorView | null = null;
    const dismissed: DismissedMark[] = [];
    if (ask.editable) {
      const host = document.createElement("div");
      host.className = "annotation-editor editor";
      dialog.append(host);
      const save = () => {
        finish({ kind: "save", body: view!.state.doc.toString() });
        return true;
      };
      view = new EditorView({
        parent: host,
        state: EditorState.create({
          doc: ask.body,
          extensions: [
            history(),
            keymap.of([{ key: "Mod-Enter", run: save }, ...historyKeymap, ...standardKeymap]),
            EditorView.lineWrapping,
            formattedView(),
            // The Work's Mods check it as a passage of its own. A dismissal holds while it's
            // open: an Annotation's text has no characters of the Work's to keep it on.
            ask.mods ? [marks({ dismiss: (mark) => {
              dismissed.push({ key: dismissKey(mark), ranges: [[mark.from, mark.to]] });
              view?.dispatch({ effects: setDismissed.of([...dismissed]) });
            } }), ask.mods] : [],
            EditorView.contentAttributes.of({ "aria-label": "The Annotation's text", spellcheck: "false" }),
          ],
        }),
      });
      const hint = document.createElement("p");
      hint.className = "muted annotation-hint";
      hint.textContent = "Printed as a footnote after what it covers. Ctrl+Enter saves.";
      dialog.append(hint);
    } else {
      const body = document.createElement("p");
      body.className = "annotation-body";
      body.textContent = ask.body;
      dialog.append(body);
    }
    const menu = document.createElement("menu");
    if (ask.editable && ask.existing) menu.append(button("Delete", () => finish({ kind: "delete" })));
    if (ask.canComment) menu.append(button("Comment…", () => finish({ kind: "comment" })));
    if (ask.canAnnotate) menu.append(button("Annotate this note…", () => finish({ kind: "annotate" })));
    const spacer = document.createElement("span");
    spacer.className = "spacer";
    menu.append(spacer);
    menu.append(button(ask.editable ? "Cancel" : "Close", () => finish(null)));
    if (ask.editable) menu.append(button(ask.existing ? "Save" : "Annotate", () => finish({ kind: "save", body: view!.state.doc.toString() }), true));
    dialog.append(menu);
    dialog.addEventListener("close", () => {
      view?.destroy();
      dialog.remove();
      resolve(answer);
    }, { once: true });
    document.body.append(dialog);
    dialog.showModal();
    if (view) {
      view.focus();
      view.dispatch({ selection: { anchor: view.state.doc.length } });
    }
  });
}
