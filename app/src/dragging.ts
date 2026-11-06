// Dragging a selection moves it (with Ctrl held, it copies), the same in every web engine.
//
// CodeMirror decides on drop whether the drag started in this editor, from a note it clears
// when the mouse button goes up or the drag ends. WebKitGTK (Debian) reports those before the
// drop, so CodeMirror forgets and copies. This keeps its own note of what's being dragged and
// does the move itself; drops from elsewhere, and copies, are left to CodeMirror.

import { type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

export function dragToMove(): Extension {
  let dragged: { from: number; to: number; text: string } | null = null;
  return EditorView.domEventHandlers({
    dragstart(_event, view) {
      const { from, to } = view.state.selection.main;
      dragged = from < to ? { from, to, text: view.state.sliceDoc(from, to) } : null;
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
