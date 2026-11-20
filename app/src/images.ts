// Images in the editor (ADR 0021). In the Formatted view, a line that's only an image, as Insert
// Image writes it (`#image("map.png", width: 80%)`), shows the picture while the cursor isn't on
// it; an image inside other markup (a `#figure`, a Story element) shows as a small chip in place
// of its name. A name the Work has no Image of shows a placeholder saying so. The Raw view shows
// the text only. Only how the text looks changes: the text and its positions stay as typed.
//
// Picture files dropped on the text from the system come here too: they're added as Images and
// written in where they're dropped.

import { Facet, StateField, type EditorState, type Extension, type Range, type Text } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";

/** An Image's picture, to show: an address the browser can show it from (none for a PDF). */
export type Picture = { name: string; url: string | null; kind: string; standIn: boolean };

/** The pictures of the Work in an editor, by name in lowercase; `known` false until they've been asked for. */
export type Pictures = { known: boolean; byName: Map<string, Picture> };

const NONE: Pictures = { known: false, byName: new Map() };

/** The pictures the editor shows (set with `tabs.setPictures`). */
export const picturesFacet = Facet.define<Pictures, Pictures>({ combine: (values) => values[values.length - 1] ?? NONE });

export function pictures(value: Pictures): Extension {
  return picturesFacet.of(value);
}

/** Where `image("…")` calls in some Typst name their picture: each string's contents (`from..to`), as Rust's `images::named_in` finds them. */
export function namedIn(text: string, offset = 0): { from: number; to: number; name: string }[] {
  const found: { from: number; to: number; name: string }[] = [];
  let at = text.indexOf("image(");
  while (at !== -1) {
    const before = at > 0 ? text[at - 1] : "";
    let k = at + "image(".length;
    if (!/[A-Za-z0-9_-]/.test(before)) {
      while (k < text.length && /[ \t\r\n]/.test(text[k])) k++;
      if (text[k] === '"') {
        const start = k + 1;
        let end = start;
        while (end < text.length && text[end] !== '"' && text[end] !== "\n") end += text[end] === "\\" ? 2 : 1;
        if (text[end] === '"') {
          found.push({ from: offset + start, to: offset + end, name: text.slice(start, end) });
          k = end + 1;
        }
      }
    }
    at = text.indexOf("image(", k);
  }
  return found;
}

type Named = { from: number; to: number; name: string };

/** The lines `from..to` of the text, whole. */
function linesAround(doc: Text, from: number, to: number): [number, number] {
  return [doc.lineAt(from).from, doc.lineAt(Math.min(to, doc.length)).to];
}

/** Where the text names pictures, kept up to date by looking again only at the lines an edit touched. */
const namedField = StateField.define<Named[]>({
  create: (state) => namedIn(state.doc.toString()),
  update(named, tr) {
    if (!tr.docChanged) return named;
    const touched: [number, number][] = [];
    tr.changes.iterChangedRanges((_fromA, _toA, fromB, toB) => touched.push(linesAround(tr.state.doc, fromB, toB)));
    const inTouched = (from: number, to: number) => touched.some(([a, b]) => from <= b && to >= a);
    const kept: Named[] = [];
    for (const n of named) {
      const from = tr.changes.mapPos(n.from, 1);
      const to = tr.changes.mapPos(n.to, -1);
      if (to < from || inTouched(from, to)) continue;
      kept.push({ ...n, from, to });
    }
    for (const [a, b] of touched) kept.push(...namedIn(tr.state.doc.sliceString(a, b), a));
    const seen = new Set<number>();
    return kept.filter((n) => !seen.has(n.from) && seen.add(n.from)).sort((x, y) => x.from - y.from);
  },
});

/** The Images named in an editor's text, as found there. */
export function namedImages(state: EditorState): Named[] {
  return state.field(namedField, false) ?? namedIn(state.doc.toString());
}

/** What shows in place of a name the Work has no Image of, or an Image whose picture can't be shown here. */
function placeholder(dom: HTMLElement, name: string, picture: Picture | undefined) {
  dom.classList.add("missing");
  dom.textContent = picture ? `${picture.kind}: ${picture.name}` : `No Image called “${name}”`;
  dom.title = picture ? "Shown on the pages" : "This Work has no Image by this name: the pages show a grey box. Add one with Insert Image, or fix the name.";
}

/** A line that's only an image: the picture, with its name beneath. Clicking it shows the markup. */
class ImageBlock extends WidgetType {
  constructor(readonly name: string, readonly picture: Picture | undefined, readonly at: number) {
    super();
  }

  eq(other: ImageBlock) {
    return other.name === this.name && other.picture?.url === this.picture?.url && other.picture?.standIn === this.picture?.standIn && other.at === this.at;
  }

  toDOM(view: EditorView) {
    const dom = document.createElement("div");
    dom.className = "cm-image-block";
    const picture = this.picture;
    if (picture?.url) {
      const img = document.createElement("img");
      img.src = picture.url;
      img.alt = picture.name;
      img.draggable = false;
      // The picture arrives after the editor measured its line: measured again once it's in,
      // or the editor still places the lines below where they'd be without it.
      img.addEventListener("load", () => view.requestMeasure());
      dom.append(img);
      const label = document.createElement("span");
      label.className = "cm-image-name";
      label.textContent = picture.standIn ? `${picture.name} (the picture it had then was deleted: showing the one on hand)` : picture.name;
      dom.append(label);
    } else {
      placeholder(dom, this.name, picture);
    }
    // A click anywhere on it puts the cursor on its line, which shows the markup. Handled here
    // alone: the editor would place the click by its own measure of where the lines are. The
    // cursor moves only once the button is up: the picture then gives way to its shorter line,
    // and a button still down would put the cursor on whatever text moved up under it.
    dom.addEventListener("mousedown", (e) => {
      if (e.button === 0) e.preventDefault();
    });
    dom.addEventListener("click", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      view.focus();
      view.dispatch({ selection: { anchor: this.at }, scrollIntoView: true });
    });
    return dom;
  }

  /** The editor leaves its clicks to it (see `toDOM`); other events, such as a right-click, are the editor's. */
  ignoreEvent(event: Event) {
    return event.type === "mousedown" || event.type === "click";
  }
}

/** An image inside other markup: a small chip with its picture in place of its quoted name. */
class ImageChip extends WidgetType {
  constructor(readonly name: string, readonly picture: Picture | undefined) {
    super();
  }

  eq(other: ImageChip) {
    return other.name === this.name && other.picture?.url === this.picture?.url;
  }

  toDOM() {
    const dom = document.createElement("span");
    dom.className = "cm-image-chip";
    const picture = this.picture;
    if (picture?.url) {
      const img = document.createElement("img");
      img.src = picture.url;
      img.alt = "";
      dom.append(img, document.createTextNode(picture.name));
    } else {
      placeholder(dom, this.name, picture);
    }
    return dom;
  }
}

/** A line holding nothing but one image call, as Insert Image writes it. */
const ALONE = /^\s*#(std\.)?image\(\s*"[^"\n]*"[^\n]*\)\s*$/;

function decorations(state: EditorState): DecorationSet {
  const pictures = state.facet(picturesFacet);
  if (!pictures.known) return Decoration.none;
  const { from: selFrom, to: selTo } = state.selection.main;
  const touches = (a: number, b: number) => selFrom <= b && selTo >= a;
  const ranges: Range<Decoration>[] = [];
  let lastLine = -1;
  for (const n of state.field(namedField)) {
    if (n.name.includes("/")) continue; // not one of the Work's own (a path written by hand)
    const picture = pictures.byName.get(n.name.toLowerCase());
    const line = state.doc.lineAt(n.from);
    if (ALONE.test(line.text) && namedIn(line.text).length === 1) {
      if (line.number === lastLine || touches(line.from, line.to)) continue;
      lastLine = line.number;
      ranges.push(Decoration.replace({ widget: new ImageBlock(n.name, picture, n.from), block: true }).range(line.from, line.to));
    } else if (!touches(n.from - 1, n.to + 1)) {
      ranges.push(Decoration.replace({ widget: new ImageChip(n.name, picture) }).range(n.from - 1, n.to + 1));
    }
  }
  return Decoration.set(ranges, true);
}

const decorationField = StateField.define<DecorationSet>({
  create: (state) => decorations(state),
  update(deco, tr) {
    const same = !tr.docChanged && tr.startState.selection.eq(tr.state.selection) && tr.startState.facet(picturesFacet) === tr.state.facet(picturesFacet);
    return same ? deco : decorations(tr.state);
  },
  provide: (field) => EditorView.decorations.from(field),
});

/** The Formatted view's pictures (part of `viewExtension`). */
export function imageWidgets(): Extension {
  return [namedField, decorationField];
}

/** The kinds of file a picture can be, by extension. */
const PICTURE = /\.(png|jpe?g|gif|webp|svg|pdf)$/i;

/** Is this dropped file a picture an Image can hold? */
export function isPictureFile(file: File): boolean {
  return PICTURE.test(file.name) || /^image\/(png|jpeg|gif|webp|svg\+xml)$|^application\/pdf$/.test(file.type);
}

/** Picture files dropped on the text from the system: `dropped` is told which, and where in the text. Other drops are left alone. */
export function pictureDrops(dropped: (files: File[], pos: number) => void): Extension {
  const pictureFiles = (e: DragEvent) => [...(e.dataTransfer?.files ?? [])].filter(isPictureFile);
  return EditorView.domEventHandlers({
    dragover(e) {
      if (!e.dataTransfer?.types.includes("Files")) return false;
      e.preventDefault(); // so it can be dropped here
      e.dataTransfer.dropEffect = "copy";
      return true;
    },
    drop(e, view) {
      const files = pictureFiles(e);
      if (files.length === 0) return false;
      e.preventDefault();
      const pos = view.posAtCoords({ x: e.clientX, y: e.clientY }) ?? view.state.selection.main.head;
      dropped(files, pos);
      return true;
    },
  });
}

/** Where to write in an image at `pos`: on a line of its own, splitting the line there if need be. */
export function imageLine(doc: Text, pos: number, name: string): { from: number; insert: string; cursor: number } {
  const line = doc.lineAt(pos);
  const call = `#image("${name}")`;
  const before = pos > line.from && doc.sliceString(line.from, pos).trim() !== "" ? "\n" : "";
  const after = pos < line.to && doc.sliceString(pos, line.to).trim() !== "" ? "\n" : "";
  const insert = `${before}${call}${after}`;
  return { from: pos, insert, cursor: pos + insert.length };
}

/** The decorations an editor's Formatted view has for its Images (for tests). */
export function imageDecorations(state: EditorState): DecorationSet {
  return state.field(decorationField);
}
