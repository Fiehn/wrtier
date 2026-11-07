// A Style library file open in a tab. It has no History: it's saved to its plain file shortly
// after each pause in typing, and taken in again when it changes on disk (another editor,
// another Device) while nothing typed here is waiting to be saved.

import { invoke } from "@tauri-apps/api/core";
import { Annotation } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

/** Marks the editor's own changes from disk, so they aren't saved back. */
export const fromDisk = Annotation.define<boolean>();

/** How long after the last keystroke the file is saved. */
const SAVE_MS = 800;

/** A library tab's ID: the file's path in the Vault, marked so it's never taken for a Work's. */
export const libraryId = (path: string) => `library:${path}`;
export const libraryPath = (id: string) => (id.startsWith("library:") ? id.slice("library:".length) : null);

export class LibraryFile {
  /** Typed here and not saved yet. */
  private dirty = false;
  private timer: number | undefined;
  view: EditorView | null = null;

  constructor(readonly path: string, private readonly error: (message: string) => void) {}

  get id() {
    return libraryId(this.path);
  }

  /** The Author typed in it. */
  edited() {
    this.dirty = true;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.flush(), SAVE_MS);
  }

  /** Saves now whatever is waiting. */
  async flush() {
    clearTimeout(this.timer);
    if (!this.dirty || !this.view) return;
    this.dirty = false;
    try {
      await invoke("write_library", { path: this.path, text: this.view.state.doc.toString() });
    } catch (e) {
      this.dirty = true;
      this.error(`${this.path} couldn't be saved: ${e}`);
    }
  }

  /**
   * The library changed on disk: takes the file in again, unless something typed here is
   * waiting (then this tab's version is saved over it in a moment). False if the file is gone.
   */
  async reload(): Promise<boolean> {
    if (this.dirty || !this.view) return true;
    let text: string;
    try {
      text = await invoke<string>("read_library", { path: this.path });
    } catch {
      return false;
    }
    const view = this.view;
    if (this.dirty || text === view.state.doc.toString()) return true;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, annotations: fromDisk.of(true) });
    return true;
  }
}
