// Dragging a selection moves it (with Ctrl held, it copies), the same in every web engine.
//
// CodeMirror decides on drop whether the drag started in this editor, from a note it clears
// when the mouse button goes up or the drag ends. WebKitGTK (Debian) reports those before the
// drop, so CodeMirror forgets and copies. This keeps its own note of what's being dragged and
// does the move itself; drops from elsewhere, and copies, are left to CodeMirror.
//
// What's dragged is the selection as it was when the mouse button went down on it. CodeMirror
// waits about 10 pixels for the drag to start, then takes the mouse for a new selection from
// where it was pressed; WebKit can start the drag later than that, and the selection has then
// already changed (found by the WebKit tests: "She walked home." became "he walked home.\n").

import { type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

type Dragged = { from: number; to: number; text: string };

export function dragToMove(): Extension {
  let dragged: Dragged | null = null;
  /** The selection the mouse button last went down on, if it went down on one. */
  let pressed: Dragged | null = null;
  return EditorView.domEventHandlers({
    mousedown(event, view) {
      const { from, to } = view.state.selection.main;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      const onIt = event.button === 0 && from < to && pos !== null && pos >= from && pos <= to;
      pressed = onIt ? { from, to, text: view.state.sliceDoc(from, to) } : null;
      return false; // CodeMirror handles the press as usual
    },
    dragstart(_event, view) {
      const { from, to } = view.state.selection.main;
      dragged = pressed ?? (from < to ? { from, to, text: view.state.sliceDoc(from, to) } : null);
      pressed = null;
      return false; // CodeMirror still sets the drag up
    },
    drop(event, view) {
      const drag = dragged;
      dragged = null;
      const copying = event.ctrlKey || event.metaKey || event.altKey;
      // From another program, a copy, or text that changed meanwhile: CodeMirror's own way.
      if (!drag || copying || view.state.sliceDoc(drag.from, drag.to) !== drag.text) return false;
      event.preventDefault();
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY }, false);
      if (pos >= drag.from && pos <= drag.to) return true; // dropped onto itself: nothing moves
      const changes = view.state.changes([{ from: drag.from, to: drag.to }, { from: pos, insert: drag.text }]);
      view.focus();
      view.dispatch({
        changes,
        selection: { anchor: changes.mapPos(pos, -1), head: changes.mapPos(pos, 1) },
        userEvent: "move.drop", // recorded as one Move, so the text keeps its History
      });
      return true;
    },
  });
}
