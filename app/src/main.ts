import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { redo, undo } from "@codemirror/commands";
import type { EditorView } from "@codemirror/view";
import { type WorkState } from "./sync";
import { Tabs, type OpenedWork } from "./tabs";
import type { Folder } from "./tree";
import { showMenu, type MenuItem } from "./menu";
import { PagePreview, type Milestone } from "./preview";
import { LibraryPanel, type CompareLine, type LibraryItem, type Offer } from "./library";
import { libraryId, libraryPath } from "./libraryfile";
import { PreambleDialog, type Preamble } from "./preambledialog";
import type { Reviewer } from "./reviewers";
import { SidePanel, type PanelTab } from "./sidepanel";
import { CommentsPanel } from "./commentspanel";
import { FootnotesPanel, type DetachedRow } from "./footnotespanel";
import { MarksPanel } from "./markspanel";
import { ImagesPanel, sizeText, type ImageRow, type ImagesView, type PastPicture } from "./imagespanel";
import { imageLine } from "./images";
import { findAnnotation, insertFootnote } from "./footnotes";
import { askAnnotation } from "./annotationdialog";
import { inclusionsOf } from "./inclusions";
import { HistoryPanel, type ComparisonView, type HistoryView, type RestorePreview, type ReturnPreview, type RevertPreview, type SnippetsView } from "./historypanel";
import { COMMANDS, keyLabel, withKey, type CommandId } from "./keys";
import { ensureFilesAccess, onAndroid } from "./android";
import { PhoneLayout } from "./phone";
import { PanelSizes } from "./panelsizes";
import { openMods } from "./modsdialog";
import { inUse, modExtensions, modNamed, noSettings, type ModContext, type ModSettings, type ModsState } from "./mods";
import { LEVEL_NAMES, type MarkCounts, type MarkLevel } from "./marks";
import { ChapterList } from "./chapterlist";
import { CountLine } from "./countline";
import type { ChapterChoice } from "./inclusions";
import { SyncPanel } from "./syncpanel";
import { UpdateDialog } from "./updates";
import { UninstallDialog } from "./uninstall";
import { FindBar } from "./findbar";
import { FindInVault, type TextForFinding, type VaultWork } from "./findinvault";
import { ReadingPanel } from "./reading";
import { $, act, ask, askText, clearError, confirmAsk, offer, onUpdatesAsked, showError, showProblems, SYMBOLS, symbolButton, wireCloseButtons, closeDialogsOnClickOutside } from "./ui";
import { PreambleActions } from "./preambleactions";
import { VaultBrowser, type Entry } from "./vaultbrowser";
import { MovingWorks } from "./movingworks";
import { AppKeys } from "./appkeys";
import { SettingsDialog } from "./settingsdialog";
import { WindowFrame } from "./windowframe";

// ---- Messages and dialogs (ui.ts) ----

wireCloseButtons();
closeDialogsOnClickOutside();

/** How the Vault syncs through Syncthing: the line at the top of the sidebar and its dialog. */
const syncPanel = new SyncPanel((message) => showError(message), (text, yes) => confirmAsk(text, yes));

// ---- Find and replace (in the tab in front) ----

const findBar = new FindBar({
  target: () => (readingPanel.hasFocus() ? { view: readingPanel.view, work: readingPanel.work, reading: true } : tabs.frontEditor()),
  readingSlot: () => readingPanel.findSlot,
  openWork: (work, at) => act(async () => {
    await openWork(work, at);
    await tabs.reveal(work, at, at); // also when it was open already
  }),
});

// ---- Tabs ----

const tabs = new Tabs($("tabbar"), $("panes"), { notice: $("notice"), status: $("status"), moved: $("moved"), style: $("style-notice") }, {
  changed: () => {
    $("empty").hidden = tabs.active !== null;
    // A library file is plain Typst: no pages of its own (and no Formatted view, in the ⌄ menu).
    $("panel-toggle").hidden = !tabs.activeIsWork;
    $("comment-on-work").hidden = !tabs.activeIsWork;
    $("tab-menu").hidden = tabs.active === null;
    tabs.bar.querySelector(".tab.active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
    findBar.attach();
    showToolbar();
    preview.show(tabs.activeIsWork ? tabs.active : null);
    commentsPanel.render();
    footnotesPanel.workChanged();
    imagesPanel.workChanged();
    historyPanel.workChanged();
    showModViews(true);
    marksPanel.workChanged();
    rememberSoon();
    browser.highlightOpen();
    // The Work being read got a tab: the Reading panel mirrors it from now on.
    if (readingPanel.work && tabs.has(readingPanel.work)) readingPanel.tabOpened(readingPanel.work);
  },
  error: showError,
  askName: (question, initial) => askText(question, initial),
  edited: (id) => {
    preview.edited(id);
    chapterList.edited(id);
    countLine.edited(id);
    findInVault.edited(id);
    footnotesPanel.edited(id);
  },
  committed: (id) => preview.committed(id),
  cursor: (id, at) => {
    preview.cursor(id);
    chapterList.cursor(id, at);
    countLine.selected(id);
  },
  closed: (id) => {
    preview.forget(id);
    modsOf.delete(id);
    readingPanel.tabClosed(id);
  },
  renamed: (from, to) => readingPanel.renamed(from, to),
  tabMenu: (id, x, y) => showMenu(x, y, [
    { label: "Read beside", run: () => readBeside(id, tabs.cursorOf(id) ?? 0) },
    "separator",
    { label: "Close", run: () => act(() => tabs.close(id)) },
  ]),
  addElement: (id, name) => act(() => preambleActions.addElement(id, name)),
  openComments: (_id, ids) => {
    openTab("comments");
    commentsPanel.focus(ids);
  },
  highlights: (id) => {
    if (id === tabs.active) commentsPanel.render();
  },
  addComment: (id) => startComment(id),
  footnote: (id) => footnoteIn(id),
  insertImage: (id) => insertImage(id),
  dropPictures: (id, files, pos) => dropPictures(id, files, pos),
  annotation: (id, _at, annotation, owner) => openAnnotation(id, annotation, owner),
  selectionMenu: (id, x, y) => showMenu(x, y, selectionMenu(id)),
  historyGrew: (id) => {
    historyPanel.grew(id);
    imagesPanel.changed(id);
  },
  textHistory: (id) => textHistory(id),
  notices: (count) => {
    showCount($("panel-tab-history"), "History", count);
  },
  modCommand: (id, command) => modCommand(id, command),
  cursorMenu: (id, x, y, included) => showMenu(x, y, cursorMenu(id, included)),
  longPressSelects: onAndroid,
  stopRule: (id, mark) => act(async () => {
    await invoke("set_mod_setting", { scope: "work", id, key: `${mark.mod}.rule.${mark.rule}`, value: "off" });
    await refreshMods(id);
  }),
  markCounts: (id, counts) => {
    markCountsOf.set(id, counts);
    if (id === tabs.active) showMarkCounts();
  },
  marksChanged: (id) => {
    if (id === tabs.active) marksPanel.changed();
  },
  dropWork: (id, work, pos) => act(() => includeChecked(id, work, null, pos)),
  inclusionStyles: (id, at, styles, x, y) => {
    const all = library.items.filter((i) => i.kind === "style");
    if (all.length === 0) {
      showError("The Style library has no Styles yet.");
      return;
    }
    const set = (next: string[]) => act(async () => {
      await tabs.settled(id);
      await invoke("set_inclusion_styles", { id, at, styles: next });
      tabs.refreshInclusionsOf(id);
    });
    showMenu(x, y, [
      ...all.map((s): MenuItem => {
        const on = styles.includes(s.name);
        return { label: `${on ? "✓ " : "    "}${s.title}`, run: () => set(on ? styles.filter((n) => n !== s.name) : [...styles, s.name]) };
      }),
      ...(styles.length > 0 ? ["separator" as const, { label: "No Styles of its own", run: () => set([]) }] : []),
    ]);
  },
  openWork: (id, at) => act(async () => {
    await openWork(id, at);
    await tabs.reveal(id, at, at); // also when it was open already
  }),
});

// ---- The Reading panel: one Work's text, read beside the one being written ----

const readingPanel: ReadingPanel = new ReadingPanel({
  tabView: (id) => tabs.viewOf(id),
  tabHighlights: (id) => tabs.highlightsOf(id),
  elements: async (id, names) => preambleActions.elementInfos(names ?? (await invoke<{ preamble: Preamble }>("get_preamble", { id })).preamble.elements),
  writeIn: (id, at) => act(async () => {
    await openWork(id, at);
    await tabs.reveal(id, at, at); // also when it was open already
  }),
  openComments: (id, ids) => act(async () => {
    await openWork(id);
    openTab("comments");
    commentsPanel.focus(ids);
  }),
  changed: () => rememberSoon(),
  shown: (on) => {
    $("phone-read").hidden = !on;
    if (!on) {
      findBar.forget(readingPanel.view);
      if (phone.screen === "read") phone.show("write");
    }
  },
  error: showError,
});
tabs.watch((id, update) => readingPanel.tabUpdated(id, update));

/** "Read beside": a Work in the Reading panel, at `top` (on the phone, its screen). */
function readBeside(id: string, top = 0) {
  act(async () => {
    await readingPanel.open(id, null, top);
    phone.show("read");
  });
}

// ---- The side panel (Ctrl+Shift+P): the Page preview and the Comments ----

const preview = new PagePreview($("preview-bar"), $("preview-pages"), $<HTMLSelectElement>("preview-look"), $<HTMLSelectElement>("preview-version"), $<HTMLSelectElement>("preview-zoom"), {
  settled: (id) => tabs.settled(id),
  reveal: (id, from, to) => {
    phone.toWrite();
    void tabs.reveal(id, from, to);
  },
  cursorOf: (id) => tabs.cursorOf(id),
  looksOf: async (id) => (await invoke<{ preamble: Preamble }>("get_preamble", { id })).preamble.looks,
  milestonesOf: (id) => invoke<Milestone[]>("get_milestones", { id }),
  error: showError,
});

// ---- The History ----

const historyPanel = new HistoryPanel($("history-list"), {
  work: () => (tabs.activeIsWork ? tabs.active : null),
  load: (work, limit, filtered) => invoke<HistoryView>("get_history", { id: work, limit, filtered }),
  snippets: (work, changes, filtered) => invoke<SnippetsView>("get_snippets", { id: work, changes, filtered }),
  compare: (work, from, to) => invoke<ComparisonView>("compare_versions", { id: work, from, to }),
  previewRevert: (work, changes) => invoke<RevertPreview>("preview_revert", { id: work, changes }),
  previewRestore: (work, changes) => invoke<RestorePreview>("preview_restore", { id: work, changes }),
  previewReturn: (work, milestone) => invoke<ReturnPreview>("preview_return", { id: work, milestone }),
  act: (work, op) => {
    clearError();
    tabs.history(work, op);
  },
  locate: (work, first, last) => {
    phone.toWrite();
    return tabs.locateText(work, first, last);
  },
  nameMilestone: (work) => preambleActions.nameMilestone(work),
  renameMilestone: async (work, milestone, name) => {
    tabs.status(work, await invoke<WorkState>("rename_milestone", { id: work, milestone, name }));
    await preview.milestonesChanged(work);
  },
  setMilestoneDeleted: async (work, milestone, deleted) => {
    tabs.status(work, await invoke<WorkState>("set_milestone_deleted", { id: work, milestone, deleted }));
    await preview.milestonesChanged(work);
  },
  dismissNotice: async (work, key) => {
    tabs.status(work, await invoke<WorkState>("dismiss_notice", { id: work, key }));
  },
  openPreamble: (work) => act(() => preambleActions.editPreamble(work)),
  error: showError,
});
$("history-name-milestone").addEventListener("click", () => historyPanel.nameMilestone());
$("history-compare").addEventListener("click", () => historyPanel.startCompare());

// "Colour text by Session": on or off on this Device (off at first). While it's off, nothing
// is worked out for it.
const tintSwitch = $<HTMLInputElement>("history-tint");
try {
  tintSwitch.checked = localStorage.getItem("session-colours") === "on";
} catch {
  // not remembered: off
}
tabs.setTinting(tintSwitch.checked);
tintSwitch.addEventListener("change", () => {
  tabs.setTinting(tintSwitch.checked);
  try {
    localStorage.setItem("session-colours", tintSwitch.checked ? "on" : "off");
  } catch {
    // not remembered, then: no harm done
  }
});

/** Its key (Ctrl+Shift+H at first) or "History of this text": the History tab narrows to the
 *  selection, or to the paragraph the cursor is in. */
function textHistory(id: string) {
  act(async () => {
    const quote = await tabs.stretch(id);
    if (quote === null) throw new Error("Select some text, or put the cursor in a paragraph, to see its History.");
    openTab("history");
    historyPanel.showTextHistory(id, quote);
  });
}

// ---- Footnotes ----

// The Footnotes tab: offered only while the Work in front has a Footnote (sidepanel.ts).
const footnotesPanel = new FootnotesPanel($("footnotes-list"), {
  work: () => (tabs.activeIsWork ? tabs.active : null),
  view: (work) => tabs.viewOf(work),
  go: (work, at) => {
    const view = tabs.viewOf(work);
    if (!view) return;
    phone.toWrite();
    view.dispatch({ selection: { anchor: Math.min(at, view.state.doc.length) }, scrollIntoView: true });
    view.focus();
  },
  openAnnotation: (work, annotation, owner) => openAnnotation(work, annotation, owner),
  detached: (work) => invoke<DetachedRow[]>("detached_annotations", { id: work }),
  deleteAnnotations: (work, ids) => act(async () => {
    const what = ids.length === 1 ? "this Detached annotation" : `these ${ids.length} Detached annotations`;
    if (!(await confirmAsk(`Delete ${what}? Undo brings each back.`, "Delete"))) return;
    await tabs.settled(work);
    for (const annotation of ids) await invoke("delete_annotation", { id: work, annotation });
    annotationsChanged(work);
  }),
  available: (has, now) => {
    panel.setAvailable("footnotes", has, now);
    const button = $("phone-footnotes");
    if (has || now || phone.screen !== "footnotes") button.hidden = !has;
    if (!has && now && phone.screen === "footnotes") phone.show("write");
  },
});

/** Its key (Ctrl+Alt+F at first), the toolbar's Note, or "Insert footnote": `#footnote[]` at the cursor or around the selection. */
function footnoteIn(id: string, included = tabs.includedSelectionOf(id)) {
  const view = tabs.viewOf(id);
  if (!view) return;
  // On included text, which can't be changed here: a note on it is an Annotation (ADR 0020).
  if (included) {
    annotateIncluded(id, included);
    return;
  }
  view.dispatch({ ...insertFootnote(view.state), scrollIntoView: true, userEvent: "input.footnote" });
  view.focus();
}

/** Short enough to name in a title. */
const clipped = (text: string, n = 60) => (text.length > n ? `${text.slice(0, n).trimEnd()}…` : text);

/** A new Annotation on text selected in an Inclusion: written in its own small editor, then kept in this Work. */
function annotateIncluded(id: string, included: { at: number; from: number; to: number; quote: string; label: string }) {
  act(async () => {
    const answer = await askAnnotation({ title: `Annotation on “${clipped(included.quote.replace(/\s+/g, " "))}”`, quote: "", body: "", editable: true, existing: false, mods: annotationMods(id) });
    if (answer?.kind !== "save" || !answer.body.trim()) return;
    await tabs.settled(id);
    await invoke("annotate", { id, at: included.at, from: included.from, to: included.to, body: answer.body });
    annotationsChanged(id);
  });
}

/** An Annotation's number clicked (or its row in the Footnotes tab): this Work's own to change, another's to read. */
function openAnnotation(id: string, annotation: string, owner: string) {
  const view = tabs.viewOf(id);
  if (!view) return;
  const found = findAnnotation(inclusionsOf(view.state), annotation, owner);
  if (!found) {
    showError("That Annotation isn't shown here any more.");
    return;
  }
  const { note, on } = found;
  act(async () => {
    const whose = note.mine ? "" : `${note.ownerLabel || "Another Work"}'s `;
    const answer = await askAnnotation({
      title: on ? `${whose}Annotation on a note` : `${whose}Annotation on “${clipped(note.quote.replace(/\s+/g, " "))}”`,
      quote: on ? on.body : "",
      body: note.body,
      editable: note.mine,
      existing: true,
      canAnnotate: !note.mine,
      canComment: note.mine,
      mods: annotationMods(id),
    });
    if (!answer) return;
    await tabs.settled(id);
    if (answer.kind === "save") {
      if (!answer.body.trim()) {
        if (await confirmAsk("Delete this Annotation? Undo brings it back.", "Delete")) await invoke("delete_annotation", { id, annotation });
      } else {
        await invoke("set_annotation", { id, annotation, body: answer.body });
      }
    } else if (answer.kind === "delete") {
      await invoke("delete_annotation", { id, annotation });
    } else if (answer.kind === "comment") {
      openTab("comments");
      await commentsPanel.startAddingAnnotation(id, annotation, note.body);
      return;
    } else if (answer.kind === "annotate") {
      const more = await askAnnotation({ title: `Annotation on ${whose}note`, quote: note.body, body: "", editable: true, existing: false, mods: annotationMods(id) });
      if (more?.kind !== "save" || !more.body.trim()) return;
      await invoke("annotate_annotation", { id, owner, on: annotation, body: more.body });
    }
    annotationsChanged(id);
  });
}

/** The Work's Mods in use here, for an Annotation's editor: they check its text in the Work's language. */
function annotationMods(id: string) {
  const state = modsOf.get(id);
  return state ? modExtensions(inUse(state), modContext(id)) : [];
}

/** An Annotation was made, changed or deleted in `id`: its Inclusions and the Footnotes tab follow. */
function annotationsChanged(id: string) {
  tabs.refreshInclusionsOf(id);
  footnotesPanel.edited(id);
}

// ---- Images (ADR 0021) ----

/** A picture kept in the Work, ready to be given to an Image (vault/images.rs `KeptPicture`). */
type KeptPicture = { hash: string; name: string; kind: string; size: number; clash: string | null; otherName: string; same: boolean };

// The Images tab: offered only while the Work in front (or a Work it includes) has an Image.
const imagesPanel = new ImagesPanel($("images-list"), {
  work: () => (tabs.activeIsWork ? tabs.active : null),
  go: (work, at) => {
    const view = tabs.viewOf(work);
    if (!view) return;
    phone.toWrite();
    view.dispatch({ selection: { anchor: Math.min(at, view.state.doc.length) }, scrollIntoView: true });
    view.focus();
  },
  insert: (work) => insertImage(work),
  replace: (work, image) => replaceImage(work, image),
  rename: (work, image) => renameImage(work, image),
  remove: (work, image) => deleteImage(work, image),
  usePast: (work, image, past) => act(async () => {
    await tabs.settled(work);
    tabs.image(work, { op: "setImage", name: image.name, hash: past.hash });
    await imagesChanged(work);
  }),
  deletePast: (work, image, past) => deletePastPictures(work, image, past),
  openPast: (work) => act(() => invoke("open_past_pictures", { id: work })),
  openWork: (work) => act(() => openWork(work)),
  pictures: (work, pictures) => tabs.setPictures(work, pictures),
  available: (has, now) => {
    panel.setAvailable("images", has, now);
    const button = $("phone-images");
    if (has || now || phone.screen !== "images") button.hidden = !has;
    if (!has && now && phone.screen === "images") phone.show("write");
  },
  error: showError,
});

/** After an Image changed: the pages, the Images tab and the editor's pictures follow. */
async function imagesChanged(id: string) {
  await tabs.settled(id);
  imagesPanel.changed(id, true);
  preview.edited(id);
}

/**
 * Gives a kept picture to an Image of the Work `id` (asking first if one has its name: replace
 * it, or keep both), and writes it in on a line of its own at `pos` (else the cursor). Returns
 * where the text it wrote ends; undefined if the Author cancelled.
 */
async function placeImage(id: string, kept: KeptPicture, pos: number | null): Promise<number | undefined> {
  if (!tabs.viewOf(id)) return undefined;
  let name = kept.name;
  let set = true;
  if (kept.clash && kept.same) {
    name = kept.clash; // that very picture: nothing to replace
    set = false;
  } else if (kept.clash) {
    $("image-clash-text").textContent = `This Work has an Image called “${kept.clash}” already. Replace its picture (every place that uses it shows the new one), or keep both, calling the new one “${kept.otherName}”?`;
    const choice = await ask($<HTMLDialogElement>("image-clash"));
    if (choice === "replace") name = kept.clash;
    else if (choice === "both") name = kept.otherName;
    else return undefined;
  }
  const view = tabs.viewOf(id);
  if (!view) return undefined;
  if (set) tabs.image(id, { op: "setImage", name, hash: kept.hash });
  const at = Math.min(pos ?? view.state.selection.main.head, view.state.doc.length);
  const line = imageLine(view.state.doc, at, name);
  view.dispatch({ changes: { from: line.from, insert: line.insert }, selection: { anchor: line.cursor }, scrollIntoView: true, userEvent: "input.paste" });
  view.focus();
  await imagesChanged(id);
  return line.cursor;
}

/** Its key (Ctrl+Alt+I at first), the toolbar's Image, or "Insert Image…": a picture file, picked, at the cursor. */
function insertImage(id: string) {
  act(async () => {
    if (tabs.includedSelectionOf(id)) throw new Error("An Image can't go in an Inclusion's text: that's changed in its own Work.");
    const kept = await invoke<KeptPicture | null>("pick_image", { id });
    if (kept) await placeImage(id, kept, null);
  });
}

/** Picture files dragged from the system onto the text: each an Image, written in one after another there. */
function dropPictures(id: string, files: File[], pos: number) {
  act(async () => {
    let at = pos;
    for (const file of files) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const kept = await invoke<KeptPicture>("keep_image_bytes", bytes, { headers: { work: encodeURIComponent(id), name: encodeURIComponent(file.name) } });
      const after = await placeImage(id, kept, at);
      if (after !== undefined) at = after;
    }
  });
}

/** Replace…: a new picture for an Image. One of another kind renames it (its uses too), as its name says its kind. */
function replaceImage(id: string, image: ImageRow) {
  act(async () => {
    const kept = await invoke<KeptPicture | null>("pick_replacement", { id });
    if (!kept) return;
    await tabs.settled(id);
    if (kept.kind === image.kind) {
      tabs.image(id, { op: "setImage", name: image.name, hash: kept.hash });
    } else {
      const name = `${image.name.replace(/\.[^.]+$/, "")}${kept.name.slice(kept.name.lastIndexOf("."))}`;
      const view = await invoke<ImagesView>("get_images", { id });
      if (view.own.some((r) => r.name.toLowerCase() === name.toLowerCase())) {
        throw new Error(`The new picture is a ${kept.kind}, so “${image.name}” would become “${name}”, but there's an Image called that already. Rename one of them first.`);
      }
      const yes = await confirmAsk(`The new picture is a ${kept.kind}, so “${image.name}” becomes “${name}”, and every place in the text that uses it is renamed too.`, "Replace and rename");
      if (!yes) return;
      tabs.image(id, { op: "renameImage", from: image.name, to: name });
      tabs.image(id, { op: "setImage", name, hash: kept.hash });
    }
    await imagesChanged(id);
  });
}

/** Rename…: the Image and every place in the text that uses it, as one action. */
function renameImage(id: string, image: ImageRow) {
  act(async () => {
    const uses = image.uses.length;
    const also = uses === 0 ? "No place in the text uses it." : uses === 1 ? "The place in the text that uses it is renamed too." : `The ${uses} places in the text that use it are renamed too.`;
    const extension = image.name.slice(image.name.lastIndexOf("."));
    const to = await askText(`Rename “${image.name}”? ${also} Undo takes it all back.`, image.name, "Rename", `It's a ${image.kind}, so its name ends in ${extension}.`);
    if (!to || to.trim() === image.name) return;
    await tabs.settled(id);
    tabs.image(id, { op: "renameImage", from: image.name, to: to.trim() });
    await imagesChanged(id);
  });
}

/** Deletes Past pictures of an Image for good, after asking: what needed one shows the picture on hand instead. */
function deletePastPictures(id: string, image: ImageRow, past: PastPicture[]) {
  act(async () => {
    const what = past.length === 1 ? "this Past picture" : `these ${past.length} Past pictures`;
    const room = sizeText(past.reduce((n, p) => n + p.size, 0));
    const yes = await confirmAsk(
      `Delete ${what} of “${image.name}” (${room})? ${past.length === 1 ? "It's" : "They're"} gone for good, on every Device: undo can't bring ${past.length === 1 ? "it" : "them"} back. A Milestone that had ${past.length === 1 ? "it" : "one"} shows the picture the Image has now instead, and says so.`,
      "Delete for good",
    );
    if (!yes) return;
    await invoke("delete_past_pictures", { id, hashes: past.map((p) => p.hash) });
    imagesPanel.changed(id, true);
  });
}

/** Delete: the Image goes (undo brings it back); text still using its name shows a grey box. */
function deleteImage(id: string, image: ImageRow) {
  act(async () => {
    const uses = image.uses.length;
    const where = uses === 0 ? "" : uses === 1 ? " The place in the text that uses it will show a grey box." : ` The ${uses} places in the text that use it will show a grey box.`;
    if (!(await confirmAsk(`Delete the Image “${image.name}”?${where} Its picture is kept as a Past picture, and undo brings it back.`, "Delete"))) return;
    await tabs.settled(id);
    tabs.image(id, { op: "deleteImage", name: image.name });
    await imagesChanged(id);
  });
}

/** A count on one of the panel's tabs (open Comments, History notices): a number beside its
 *  symbol, and in its name. */
function showCount(tab: HTMLElement, name: string, count: number) {
  const named = count > 0 ? `${name} (${count})` : name;
  tab.title = named;
  tab.setAttribute("aria-label", named);
  tab.querySelector(".panel-count")!.textContent = count > 0 ? String(count) : "";
}

// Shown or not, and which tab, are remembered on this Device. The pages only compile while
// their tab is showing, and the History only loads while its tab is.
// Every Mark in the Work in front, by kind (markspanel.ts).
const marksPanel = new MarksPanel($("marks-list"), {
  view: () => (tabs.activeIsWork ? tabs.viewOf(tabs.active!) : null),
  marking: () => tabs.activeIsWork && marking(tabs.active!),
});

const panel = new SidePanel(
  $("side-panel"),
  [$("panel-tab-pages"), $("panel-tab-comments"), $("panel-tab-marks"), $("panel-tab-footnotes"), $("panel-tab-images"), $("panel-tab-history")],
  { pages: $("preview"), comments: $("comments-pane"), marks: $("marks-pane"), footnotes: $("footnotes-pane"), images: $("images-pane"), history: $("history-pane") },
  $("panel-toggle"),
  (pagesInView, inView) => {
    preview.setOpen(pagesInView);
    historyPanel.setVisible(inView === "history");
    // (Not while the panel is being made: the Footnotes tab asks it what's offered.)
    queueMicrotask(() => {
      footnotesPanel.setVisible(inView === "footnotes");
      imagesPanel.setVisible(inView === "images");
      marksPanel.setVisible(inView === "marks");
    });
  },
);

// On a phone (or any narrow window): one screen at a time, with a bar at the bottom.
const phone = new PhoneLayout(panel);

/** Opens one of the panel's tabs: on the phone, that tab's screen. */
function openTab(tab: PanelTab) {
  if (phone.active) phone.show(tab);
  else panel.setTab(tab);
}

// Resolved Comments' highlights: shown or not on this Device.
const showResolved = $<HTMLInputElement>("comments-show-resolved");
try {
  showResolved.checked = localStorage.getItem("comments-resolved") === "shown";
} catch {
  // not remembered: hidden
}
tabs.setCommentOptions({ ...tabs.commentOptions, resolved: showResolved.checked });
showResolved.addEventListener("change", () => {
  tabs.setCommentOptions({ ...tabs.commentOptions, resolved: showResolved.checked });
  commentsPanel.render();
  try {
    localStorage.setItem("comments-resolved", showResolved.checked ? "shown" : "hidden");
  } catch {
    // not remembered, then: no harm done
  }
});

// ---- Comments ----

const commentsPanel = new CommentsPanel($("comments-list"), $("comments-new"), {
  work: () => (tabs.activeIsWork ? tabs.active : null),
  highlights: (work) => tabs.highlightsOf(work),
  includedMarks: (work) => tabs.includedHighlightsOf(work),
  // A Comment may belong to a Work that isn't open (one shown on an Inclusion): then it's
  // changed without opening a tab, and the Inclusions in front follow.
  act: (work, op) => {
    if (tabs.has(work)) {
      tabs.comment(work, op);
      return;
    }
    act(async () => {
      await invoke("comment_in", { work, ops: [op] });
      if (tabs.active) tabs.refreshInclusionsOf(tabs.active);
    });
  },
  addIncluded: async (work, target, sender, body) => {
    await invoke("comment_on_inclusion", { id: work, at: target.at, from: target.from, to: target.to, sender, body });
    tabs.refreshInclusionsOf(work);
  },
  show: (work, comment, part) => {
    phone.toWrite();
    if (tabs.has(work)) return tabs.showComment(work, comment, part);
    // One on an Inclusion: its own Work opens, at it.
    act(async () => {
      await openWork(work);
      tabs.showComment(work, comment, part);
    });
    return 1;
  },
  showAnnotation: (work, annotation) => {
    phone.toWrite();
    openAnnotation(work, annotation, "");
  },
  setPending: (work, range) => tabs.setPending(work, range),
  pendingRange: (work) => tabs.pendingRange(work),
  reviewers: () => invoke<Reviewer[]>("get_reviewers"),
  authorName: async (work) => {
    try {
      const author = (await invoke<{ preamble: Preamble }>("get_preamble", { id: work })).preamble.author.trim();
      return author || "Author";
    } catch {
      return "Author";
    }
  },
  newReviewer: async () => {
    const name = await askText("Who is the new Reviewer?", "", "Add", "They're added to the Vault, so they can comment on any Work. Their role and colour can be changed under Reviewers.");
    if (name === null || !name.trim()) return null;
    return invoke<Reviewer>("add_reviewer", { name, role: "" });
  },
  showResolved: () => showResolved.checked,
  counted: (open) => {
    showCount($("panel-tab-comments"), "Comments", open);
  },
  error: showError,
});

/** Its key (Ctrl+Shift+M at first) or "Add Comment": a new Comment on the selection, or the word at the cursor. */
/** A Comment on the Work in front as a whole: written in the Comments panel, which opens for it. */
function commentOnWork() {
  if (!tabs.activeIsWork) return;
  const work = tabs.active!;
  openTab("comments");
  act(() => commentsPanel.startAddingWork(work));
}
$("comment-on-work").addEventListener("click", commentOnWork);

function startComment(id: string, included = tabs.includedSelectionOf(id)) {
  // Text selected in an Inclusion: the Comment goes to the Work it includes (ADR 0015).
  if (included) {
    openTab("comments");
    act(() => commentsPanel.startAddingIncluded(id, included));
    return;
  }
  const target = tabs.commentTarget(id);
  if (!target) {
    showError("Select some text to comment on, or put the cursor in a word.");
    return;
  }
  openTab("comments");
  act(() => commentsPanel.startAdding(id, target));
}

/** Right-click inside a selection: Add Comment, and the editing the system menu would have. */
function selectionMenu(id: string): MenuItem[] {
  const view = tabs.viewOf(id);
  if (!view) return [];
  // In an Inclusion's text, which can't be changed here: Add Comment and Copy. (Read now: the
  // selection may not survive clicking the menu.)
  const included = tabs.includedSelectionOf(id);
  if (included) {
    return [
      { label: "Add Comment", run: () => startComment(id, included) },
      { label: "Annotate", run: () => footnoteIn(id, included) },
      { label: "Copy", run: () => act(() => navigator.clipboard.writeText(included.quote)) },
    ];
  }
  // Cut and Copy go through the editor, so a cut pasted back is still a Move.
  const command = (name: "cut" | "copy") => () => {
    view.focus();
    document.execCommand(name);
  };
  return [
    { label: "Add Comment", run: () => startComment(id) },
    { label: "Insert footnote", run: () => footnoteIn(id) },
    { label: "Insert Image…", run: () => insertImage(id) },
    { label: "History of this text", run: () => textHistory(id) },
    { label: "Find and replace…", run: () => findBar.open(false) },
    "separator",
    { label: "Cut", run: command("cut") },
    { label: "Copy", run: command("copy") },
    { label: "Paste", run: () => pasteInto(view) },
  ];
}

/** Paste from a menu: the clipboard's text in place of the selection (or at the cursor). */
function pasteInto(view: EditorView) {
  act(async () => {
    let text: string;
    try {
      text = await navigator.clipboard.readText();
    } catch {
      throw new Error("The clipboard can't be read from this menu here. Use Ctrl+V.");
    }
    const { from, to } = view.state.selection.main;
    view.focus();
    view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length }, userEvent: "input.paste", scrollIntoView: true });
  });
}

/** Right-click with nothing selected there, the cursor moved to the click: what can be put there,
 *  and Undo and Redo. In an Inclusion's text only a Comment on the Work; in a Style library file
 *  (no Comments, footnotes or Images) Paste, Undo and Redo. */
function cursorMenu(id: string, included: boolean): MenuItem[] {
  const view = tabs.viewOf(id);
  if (!view) return [];
  if (included) return [{ label: "Comment on this Work…", run: commentOnWork }];
  // A Work's undo goes through its History; a library file's editor has its own.
  const work = tabs.activeIsWork;
  const editing: MenuItem[] = [
    { label: "Paste", run: () => pasteInto(view) },
    "separator",
    { label: "Undo", run: () => (work ? tabs.runCommand("undo") : undo(view)) },
    { label: "Redo", run: () => (work ? tabs.runCommand("redo") : redo(view)) },
  ];
  if (!work) return editing;
  return [
    { label: "Insert Image…", run: () => insertImage(id) },
    { label: "Insert footnote", run: () => footnoteIn(id) },
    { label: "Comment on this Work…", run: commentOnWork },
    "separator",
    ...editing,
  ];
}

$("panel-toggle").addEventListener("click", () => panel.toggle());

/** The tabs (and cursors) are remembered for next time, a moment after they change. */
let rememberTimer: number | undefined;
function rememberSoon() {
  clearTimeout(rememberTimer);
  rememberTimer = window.setTimeout(remember, 500);
}
function remember() {
  if (!browser.isOpen) return;
  invoke("save_layout", { layout: { ...tabs.layout(), reading: readingPanel.place() } }).catch(showError);
}

// ---- The Style library ----

const library: LibraryPanel = new LibraryPanel($("library-list"), $("library-conflicts"), $("library-offers"), {
  open: (path, title) => openLibraryFile(path, title),
  create: (kind) => act(() => preambleActions.newItem(kind)),
  menu: (item, x, y) => libraryMenu(item, x, y),
  compare: compareDefault,
  settle: () => tabs.idle(),
  offered: (count) => {
    const summary = $("library").querySelector("summary")!;
    summary.classList.toggle("has-offer", count > 0);
    summary.title = count === 0 ? "" : `${count === 1 ? "A newer version of a default is" : "Newer versions of defaults are"} available inside.`;
  },
  error: showError,
});

/** A default's two versions, line by line: the Author uses the newer one, keeps theirs, or closes it. */
async function compareDefault(offer: Offer, lines: CompareLine[]): Promise<"use" | "keep" | null> {
  $("compare-default-title").textContent = `${offer.title}: yours and the newer one`;
  const list = $("compare-default-lines");
  list.replaceChildren(...lines.map((line) => {
    const row = document.createElement("div");
    row.className = line.kind;
    row.textContent = `${line.kind === "mine" ? "−" : line.kind === "newer" ? "+" : " "} ${line.text}`;
    return row;
  }));
  const answer = await ask($<HTMLDialogElement>("compare-default"));
  return answer === "use" || answer === "keep" ? answer : null;
}

const KIND_NAMES: Record<LibraryItem["kind"], string> = { style: "Style", look: "Look", storyElement: "Story element" };

/** Right-click on a Style, Look or Story element: open, rename or remove it. */
function libraryMenu(item: LibraryItem, x: number, y: number) {
  const run = (f: () => Promise<unknown>) => () => act(f);
  const items: MenuItem[] = [
    { label: "Open", run: run(() => openLibraryFile(item.path, item.title)) },
    "separator",
    { label: "Rename…", run: run(() => renameLibraryItem(item)) },
  ];
  // The built-in Story elements are part of every Work, so they stay.
  if (!(item.kind === "storyElement" && library.builtIn.includes(item.name))) {
    items.push({ label: "Remove…", run: run(() => removeLibraryItem(item)) });
  }
  showMenu(x, y, items);
}

/**
 * Renaming changes the name shown (the title in the file's first line). The file keeps its
 * name, which is what Works and Looks find it by, so nothing using it loses it.
 */
async function renameLibraryItem(item: LibraryItem) {
  const what = KIND_NAMES[item.kind];
  const hint = item.kind === "storyElement"
    ? `It's still written as #${item.name} in the text; only the name shown changes.`
    : `Only the name shown changes: its file stays ${item.path.split("/").pop()}, so the Works and Looks using it still find it.`;
  const title = await askText(`Rename the ${what} ${item.title} to:`, item.title, "Rename", hint);
  if (title === null || title.trim() === item.title) return;
  await tabs.settled(libraryId(item.path)); // anything typed in its tab is saved first
  await invoke("retitle_library", { path: item.path, title });
  await library.refresh();
  tabs.libraryRetitled(item.path, library.titleOf(item.path));
  await tabs.libraryChanged();
}

/** Removing moves the file to the Vault's trash folder; Works that recorded it keep their copy. */
async function removeLibraryItem(item: LibraryItem) {
  const what = KIND_NAMES[item.kind];
  const lines = [`Remove the ${what} ${item.title} from the Style library?`];
  if (item.kind === "style") {
    const looks = library.items.filter((i) => i.kind === "look" && i.styles.includes(item.name)).map((i) => i.title);
    if (looks.length) lines.push(`${looks.length === 1 ? "The Look" : "The Looks"} ${looks.join(", ")} ${looks.length === 1 ? "uses" : "use"} it and will say it's missing.`);
  }
  lines.push(`Works that use it keep the copy they recorded of it, so their pages don't change. The file goes to the Vault's .trash folder.`);
  if (!(await confirmAsk(lines.join(" "), "Remove"))) return;
  await tabs.close(libraryId(item.path)); // saved first, then closed
  await invoke("trash_library", { path: item.path });
  await library.refresh();
  await preambleActions.refreshElements();
}

async function openLibraryFile(path: string, title: string = library.titleOf(path)): Promise<void> {
  const text = await invoke<string>("read_library", { path });
  await tabs.addLibrary({ path, title, text });
}

// The library changed: in a tab here, in another editor, or from another Device. The Page
// preview uses the library's versions, so it's compiled again too.
void listen("library-changed", () => {
  library.refresh().then(() => preambleActions.refreshElements()).then(() => findInVault.libraryChanged()).catch(showError);
  void tabs.libraryChanged();
  void preview.preambleChanged(tabs.activeIsWork ? tabs.active : null);
});

// ---- A Work's Mods ----

/** Each open Work's Mods, as last asked (its own, and which this Device hides). */
const modsOf = new Map<string, ModsState>();

/** Is this Mod at work in this Work on this Device? */
const modInUse = (id: string, mod: string) => inUse(modsOf.get(id) ?? { work: [], newWorks: [], vault: [], hidden: [] }).some((m) => m.id === mod);
/** Does a Work have a Mod in use here that makes Marks (the Marks tab is offered)? */
const marking = (id: string) => inUse(modsOf.get(id) ?? { work: [], newWorks: [], vault: [], hidden: [] }).some((m) => m.marks);

/** Each open Work's Mods' settings, and its language, as last asked: what its Mods are told. */
const modSettingsOf = new Map<string, ModSettings>();
const languageOf = new Map<string, string>();

function setModsOf(id: string, state: ModsState) {
  modsOf.set(id, state);
  tabs.setMods(id, inUse(state), modContext(id));
  if (id === tabs.active) showModViews();
}

/** What a Work's Mods are told: its language, the settings, and how to change one. */
function modContext(id: string): ModContext {
  return {
    language: languageOf.get(id) ?? "",
    settings: modSettingsOf.get(id) ?? noSettings(),
    setSetting: (scope, key, value) => act(async () => {
      await invoke("set_mod_setting", { scope, id: scope === "work" ? id : null, key, value });
      await refreshMods(scope === "work" ? id : undefined); // the Vault's settings are every Work's
    }),
  };
}

/** Tells each open Work's editor (or just `only`) which Mods are in use in it on this Device, and what they need to know. */
async function refreshMods(only?: string) {
  for (const id of tabs.works) {
    if (only && id !== only) continue;
    try {
      const [mods, settings, { preamble }] = await Promise.all([
        invoke<ModsState>("get_mods", { id }),
        invoke<ModSettings>("get_mod_settings", { id }),
        invoke<{ preamble: Preamble }>("get_preamble", { id }),
      ]);
      modSettingsOf.set(id, settings);
      languageOf.set(id, preamble.language);
      setModsOf(id, mods);
    } catch {
      // renamed or gone meanwhile: its tab says so, and the next refresh finds it
    }
  }
}

// ---- Marks (marks.ts): how many of each kind the Work in front has ----

const markCountsOf = new Map<string, MarkCounts>();

function showMarkCounts() {
  const el = $("mark-counts");
  const id = tabs.activeIsWork ? tabs.active : null;
  const counts = id ? markCountsOf.get(id) : undefined;
  const levels: MarkLevel[] = ["mistake", "rule", "query"];
  const shown = levels.filter((l) => (counts?.[l] ?? 0) > 0);
  el.hidden = !id || shown.length === 0;
  el.replaceChildren(...shown.map((level) => {
    const n = counts![level];
    const b = document.createElement("button");
    b.type = "button";
    b.className = `mark-count mark-count-${level}`;
    b.textContent = String(n);
    b.title = `${n} ${LEVEL_NAMES[level]}${n === 1 ? "" : "s"}: go to the next (${keyLabel("nextMark")} goes through every Mark)`;
    b.addEventListener("click", () => id && tabs.stepMark(id, level));
    return b;
  }));
  // Every one of them in the Marks panel, at the first kind there is (not on the phone: no Marks screen).
  if (shown.length > 0 && !phone.active) {
    el.append(symbolButton("See them all in the Marks panel", SYMBOLS.marks, () => {
      openTab("marks");
      marksPanel.showKind(shown[0]);
    }));
  }
  // The Marks tab's count: the Mistakes.
  const mistakes = counts?.mistake ?? 0;
  const tab = $("panel-tab-marks");
  const named = mistakes > 0 ? `Marks (${mistakes} ${mistakes === 1 ? "Mistake" : "Mistakes"})` : "Marks";
  tab.title = named;
  tab.setAttribute("aria-label", named);
  tab.querySelector(".panel-count")!.textContent = mistakes > 0 ? String(mistakes) : "";
}

// A Dictionary was downloaded, updated or removed (spellmod.ts): every Spellchecker looks again.
window.addEventListener("wrtier-dictionaries-changed", () => {
  tabs.forgetMods();
  void refreshMods();
});

async function editMods(id: string) {
  await openMods(id, id.split("/").pop() ?? id, languageOf.get(id) ?? "", (state) => setModsOf(id, state), () => {
    // The Vault's settings (the Speech verbs) are every Work's; Dismissed Marks shown again come back.
    void refreshMods();
    tabs.requestDismissed(id);
  });
  await refreshMods(); // hiding a Mod on this Device changes every Work that has it
}

/**
 * A Mod's command's key, pressed while writing in a Work: if the Work has the Mod (and this
 * Device doesn't hide it), it runs; otherwise a line offers to add it, or to show it here.
 */
function modCommand(id: string, command: CommandId): boolean {
  const mod = modNamed(COMMANDS.find((c) => c.id === command)?.mod ?? "");
  if (!mod) return false;
  if (modInUse(id, mod.id)) {
    switch (command) {
      case "nextChapter": return chapterList.step(id, 1);
      case "previousChapter": return chapterList.step(id, -1);
      case "chapterUp": return chapterList.shift(id, -1);
      case "chapterDown": return chapterList.shift(id, 1);
      default: return false;
    }
  }
  const name = id.split("/").pop() ?? id;
  if (modsOf.get(id)?.work.includes(mod.id)) {
    offer(`The ${mod.name} is hidden on this device.`, "Show it", async () => {
      await invoke("set_mod_hidden", { name: mod.id, hidden: false });
      await refreshMods();
    });
  } else {
    offer(`The ${mod.name} isn't on ${name}.`, "Add it", async () => {
      setModsOf(id, await invoke<ModsState>("set_mod", { id, name: mod.id, on: true }));
    });
  }
  return true;
}

// ---- The Chapter list (a Mod) ----

const chapterList = new ChapterList($("chapters"), $("chapters-list"), $("chapters-none"), {
  textOf: (id) => tabs.textOf(id),
  cursorOf: (id) => tabs.cursorOf(id),
  includedOf: (id) => tabs.includedOf(id),
  goTo: (id, at) => {
    phone.toWrite();
    void tabs.reveal(id, at, at);
  },
  move: (id, moves, cursor) => tabs.moveText(id, moves, cursor),
});

// ---- The word and character count (a Mod) ----

const countLine = new CountLine($("counts"), {
  textOf: (id) => tabs.readerTextOf(id),
  selectedText: (id) => tabs.readerTextOf(id, true),
});

/** The Mods that show the Work in front (the Chapter list, the counts) follow it. */
/** What the Mods in use show for the Work in front. `now`: another Work came to the front. */
function showModViews(now = false) {
  const id = tabs.activeIsWork ? tabs.active : null;
  chapterList.show(id && modInUse(id, "chapters") ? id : null);
  countLine.show(id && modInUse(id, "counts") ? id : null);
  panel.setAvailable("marks", id !== null && marking(id), now);
  if (!now) marksPanel.workChanged(); // a Mod that makes Marks added or taken away
  showMarkCounts();
}

// ---- Inclusions (ADR 0015) ----

/**
 * Includes the Work `work` at the cursor in the Work `into`: the whole of it, or (asked with a
 * menu at `at` if it has Chapters) one of its Chapters.
 */
async function includeIn(into: string, work: string, at: { x: number; y: number }) {
  // Refused before the menu, if it would make a Work include itself.
  await invoke("check_inclusion", { id: into, work });
  const chapters = await invoke<ChapterChoice[]>("chapters_of", { work });
  if (chapters.length === 0) {
    tabs.include(into, work, null);
    return;
  }
  showMenu(at.x, at.y, [
    { label: "The whole Work", run: () => tabs.include(into, work, null) },
    "separator",
    ...chapters.map((c): MenuItem => ({ label: `Chapter: ${c.title}`, run: () => tabs.include(into, work, c.at) })),
  ]);
}

/**
 * Includes `work` (or its Chapter at `chapter`) in the Work `into`, at the cursor or near `near`,
 * once Rust has said it can be: not if that would make a Work include itself.
 */
async function includeChecked(into: string, work: string, chapter: number | null, near?: number) {
  await invoke("check_inclusion", { id: into, work });
  tabs.include(into, work, chapter, near);
}

// The Work in front follows the Works it includes as they're written (in other tabs, or arriving
// from other Devices).
window.setInterval(() => tabs.refreshInclusions(), 2000);

// ---- A Work's Preamble ----

const preambleActions: PreambleActions = new PreambleActions({
  tabs,
  library,
  preview,
  readingPanel,
  dialog: new PreambleDialog(ask),
  refreshMods: (id) => refreshMods(id),
  openLibraryFile: (path) => openLibraryFile(path),
});

async function openWork(id: string, cursor?: number, restoring = false) {
  if (!restoring) phone.show("write");
  const path = libraryPath(id);
  if (path) return openLibraryFile(path);
  if (tabs.has(id)) return tabs.activate(id);
  const work = await invoke<OpenedWork>("open_work", { id });
  await tabs.add(work, cursor);
  await preambleActions.refreshElements(work.id);
  await refreshMods(work.id);
  if (work.problems.length > 0) showProblems(work.problems);
  await browser.refresh(); // a Text file may have become a Work
}

// The Rust side's timer closed a burst, saved, or noticed a change from outside.
void listen<{ id: string; state: WorkState }>("work-status", (event) => tabs.status(event.payload.id, event.payload.state));

// ---- The Vault (vaultbrowser.ts), and moving Works and folders in it (movingworks.ts) ----

const browser: VaultBrowser = new VaultBrowser({
  tabs,
  readingPanel,
  library,
  phone,
  syncPanel,
  preamble: preambleActions,
  findInVault: { reset: () => findInVault.reset(), refresh: () => findInVault.refresh() },
  loadSettings: () => settings.load(),
  openWork: (id, cursor, restoring) => openWork(id, cursor, restoring),
  readBeside: (id) => readBeside(id),
  includeIn: (into, work, at) => includeIn(into, work, at),
  editMods: (id) => editMods(id),
  startMove: (item) => moving.start(item),
  endMove: () => moving.end(),
  moveToItem: (item, at) => moving.menuItem(item, at),
});

const moving = new MovingWorks({ browser, findInVault: { refresh: () => findInVault.refresh() } });

// Going to the background on Android (another app, the screen off): Android may end the app
// there without warning, so everything typed goes to disk at once (the open burst closes, a cut
// waiting for its paste becomes a deletion) and Syncthing can send it. Nothing compiles meanwhile.
// Coming back, the Vault is checked for changes from outside at once.
document.addEventListener("visibilitychange", () => {
  const hidden = document.visibilityState === "hidden";
  preview.setOpen(!hidden && panel.pagesInView);
  if (!browser.isOpen) return;
  if (hidden) {
    if (!onAndroid) return;
    act(async () => {
      await tabs.idle();
      for (const { id, state } of await invoke<{ id: string; state: WorkState }[]>("app_hidden")) tabs.status(id, state);
    });
  } else {
    void refreshQuietly();
  }
});

// The Vault list follows what Syncthing brings (new Works, Conflict copies of Works that
// aren't open): every 10 seconds, and whenever the window gets focus. The tabs and cursors are
// remembered then too.
async function refreshQuietly() {
  if (!browser.isOpen || document.querySelector("dialog[open]")) return;
  try {
    await browser.refresh();
    await refreshMods(); // another Device may have changed them
    void findInVault.refresh(); // Works synced in, renamed or trashed: its list catches up
    remember();
  } catch (e) {
    showError(e);
  }
}
setInterval(refreshQuietly, 10_000);
window.addEventListener("focus", refreshQuietly);

// Leaving the window: the Text file is brought up to date for other programs.
window.addEventListener("blur", () => {
  remember();
  void tabs.leave();
});

// ---- Find in Vault (Ctrl+Shift+F): every Work's text, in place of the Works list ----

/** The Works to find in, in the Works list's order (folders first, as it shows them). */
function vaultWorks(): VaultWork[] {
  const out: VaultWork[] = [];
  const walk = (folder: Folder<Entry>) => {
    for (const sub of folder.folders) walk(sub);
    for (const e of folder.items) {
      if (e.kind === "work" || e.kind === "newWork") out.push({ id: e.id, name: e.name, problem: null, colour: browser.shownColour(browser.colours.works[e.id]) });
      else if (e.kind === "newer") out.push({ id: e.id, name: e.name, problem: "made by a newer version of the app" });
      else if (e.kind === "unreadable") out.push({ id: e.id, name: e.name, problem: e.reason });
      // A leftover or blocked Text file isn't a Work.
    }
  };
  walk(browser.shownTree());
  return out;
}

/** Shows a match found: its Work in a passing tab if it isn't open, the match selected. */
async function showFound(id: string, from: number, to: number, focus: boolean) {
  if (!tabs.has(id)) {
    const work = await invoke<OpenedWork>("open_work", { id });
    await tabs.addPassing(work, focus);
    await preambleActions.refreshElements(work.id);
    await refreshMods(work.id);
    if (work.problems.length > 0) showProblems(work.problems);
    if (browser.entries.some((e) => e.id === id && e.kind === "newWork")) await browser.refresh(); // it has a History now
  }
  await tabs.reveal(id, from, to, focus);
  if (focus) phone.toWrite();
}

const findInVault = new FindInVault({
  works: vaultWorks,
  hiddenFolders: () => browser.hiddenCount(),
  openText: (id) => tabs.docOf(id),
  read: (id, known) => invoke<TextForFinding>("text_for_finding", { id, known }),
  elements: () => library.items.filter((i) => i.kind === "storyElement").map((i) => ({ name: i.name, title: i.title, shownAs: i.shownAs ?? "block", inWork: true })),
  show: showFound,
  openWithFind: async (id, query, match) => {
    await openWork(id);
    tabs.keep(id);
    if (match) await tabs.reveal(id, match.from, match.to);
    findBar.open(true, query);
  },
  shown: (open) => $("sidebar-vault").classList.toggle("finding", open),
  error: showError,
});

/** Ctrl+Shift+F, or the button: the Vault browser shows (the Works screen on the phone), finding. */
function openFindInVault() {
  if (!browser.isOpen) return;
  setSidebar(true);
  phone.show("works");
  // A selection within one line in the editor is what to find, as with Ctrl+F.
  const front = tabs.frontEditor();
  let text: string | undefined;
  if (front?.view.hasFocus) {
    const { state } = front.view;
    const s = state.selection.main;
    if (!s.empty && state.doc.lineAt(s.from).number === state.doc.lineAt(s.to).number) text = state.sliceDoc(s.from, s.to);
  }
  findInVault.show(text);
}
$("open-find-in-vault").addEventListener("click", openFindInVault);

// ---- Quick switcher (Ctrl+O) ----

/** Does `query` appear in `text`, in order (not necessarily together)? */
function matches(text: string, query: string) {
  let i = 0;
  const t = text.toLowerCase();
  for (const c of query.toLowerCase()) {
    i = t.indexOf(c, i);
    if (i === -1) return false;
    i++;
  }
  return true;
}

let switcherChoice = 0;
function fillSwitcher() {
  const query = $<HTMLInputElement>("switcher-input").value.trim();
  const found = browser.entries.filter((e) => (e.kind === "work" || e.kind === "newWork") && matches(e.id, query)).slice(0, 12);
  const list = $("switcher-list");
  list.replaceChildren();
  const choices: { label: string; run: () => Promise<unknown> }[] = found.map((e) => ({ label: e.id, run: () => openWork(e.id) }));
  if (query && !found.some((e) => e.id.toLowerCase() === query.toLowerCase())) {
    choices.push({ label: `Create “${query}”`, run: () => browser.newWork(query) });
  }
  switcherChoice = Math.min(switcherChoice, Math.max(choices.length - 1, 0));
  choices.forEach((choice, i) => {
    const item = document.createElement("li");
    item.textContent = choice.label;
    item.classList.toggle("chosen", i === switcherChoice);
    item.addEventListener("click", () => {
      $<HTMLDialogElement>("switcher").close();
      act(choice.run);
    });
    list.append(item);
  });
  return choices;
}

async function openSwitcher() {
  if (!browser.isOpen) return;
  const dialog = $<HTMLDialogElement>("switcher");
  const input = $<HTMLInputElement>("switcher-input");
  input.value = "";
  switcherChoice = 0;
  fillSwitcher();
  dialog.showModal();
  input.focus();
}

$("switcher-input").addEventListener("input", () => {
  switcherChoice = 0;
  fillSwitcher();
});
$("switcher-input").addEventListener("keydown", (e) => {
  const choices = fillSwitcher();
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    switcherChoice = (switcherChoice + (e.key === "ArrowDown" ? 1 : choices.length - 1)) % Math.max(choices.length, 1);
    fillSwitcher();
  } else if (e.key === "Enter") {
    e.preventDefault();
    $<HTMLDialogElement>("switcher").close();
    const choice = choices[switcherChoice];
    if (choice) act(choice.run);
  }
});
$("open-switcher").addEventListener("click", () => void openSwitcher());
// A new Work or folder at the top of the Vault (inside a folder: its right-click menu).
$("new-work").addEventListener("click", () => act(() => browser.newWork()));
$("new-folder").addEventListener("click", () => act(() => browser.newFolder()));

// ---- Hiding the Vault browser, for writing with nothing else on screen (remembered here) ----

function setSidebar(shown: boolean) {
  $("app").classList.toggle("sidebar-hidden", !shown);
  $("sidebar-toggle").title = withKey(shown ? "Hide the Works" : "Show the Works", "toggleWorks");
  $("sidebar-toggle").setAttribute("aria-expanded", String(shown));
  try {
    localStorage.setItem("sidebar", shown ? "shown" : "hidden");
  } catch {
    // not remembered, then: no harm done
  }
}

try {
  setSidebar(localStorage.getItem("sidebar") !== "hidden");
} catch {
  setSidebar(true);
}
$("sidebar-toggle").addEventListener("click", () => setSidebar($("app").classList.contains("sidebar-hidden")));

// ---- The panels' widths: dragged by their edges, remembered on this Device ----

new PanelSizes({
  vault: {
    element: $("sidebar"),
    handle: $("sidebar-resizer"),
    shown: () => !$("app").classList.contains("sidebar-hidden"),
    setShown: setSidebar,
  },
  reading: {
    element: $("reading-panel"),
    handle: $("reading-resizer"),
    shown: () => readingPanel.isReading,
    // Never hidden by dragging (panelsizes.ts): it's only closed, with its ×.
    setShown: (shown) => {
      if (!shown) readingPanel.close();
    },
  },
  side: {
    element: $("side-panel"),
    handle: $("panel-resizer"),
    shown: () => panel.shown,
    setShown: (shown) => panel.setShown(shown),
  },
});


// ---- Formatted and Raw view (Ctrl+E), remembered on this Device ----

function setFormatted(on: boolean) {
  tabs.setFormatted(on);
  readingPanel.setFormatted(on);
  $("toolbar-view").textContent = on ? "Raw" : "Formatted";
  try {
    localStorage.setItem("view", on ? "formatted" : "raw");
  } catch {
    // not remembered, then: no harm done
  }
}

try {
  setFormatted(localStorage.getItem("view") !== "raw");
} catch {
  setFormatted(true);
}

// ---- The tab row's ⌄: what can be done with the tab in front ----

/** Its items: Find for any tab; a Comment on the Work, the view, the Preamble and the Mods for a
 *  Work's (any Work's Preamble and Mods are also in its right-click menu in the Vault browser); on
 *  the phone, the Chapter list's sheet. Names only: the keys are in Settings → Shortcuts. */
function tabMenuItems(): MenuItem[] {
  const items: MenuItem[] = [];
  if (phone.chaptersShown) items.push({ label: "Chapters", run: () => phone.openChapters() });
  if (tabs.active !== null) items.push({ label: "Find and replace", run: () => findBar.open(false) });
  if (tabs.activeIsWork) {
    items.push(
      { label: "Comment on this Work…", run: commentOnWork },
      { label: tabs.formatted ? "Raw view" : "Formatted view", run: () => setFormatted(!tabs.formatted) },
      { label: "Preamble…", run: () => act(() => preambleActions.editPreamble(tabs.active!)) },
      { label: "Mods…", run: () => act(() => editMods(tabs.active!)) },
    );
  }
  return items;
}

// A press outside the menu closes it before the click arrives: pressing ⌄ again only closes it.
let tabMenuWasOpen = false;
window.addEventListener("pointerdown", (e) => {
  if ($("tab-menu").contains(e.target as Node)) tabMenuWasOpen = document.getElementById("menu") !== null;
}, true);
$("tab-menu").addEventListener("click", () => {
  if (tabMenuWasOpen) {
    tabMenuWasOpen = false;
    return;
  }
  const box = $("tab-menu").getBoundingClientRect();
  showMenu(box.left, box.bottom + 2, tabMenuItems());
});

// Many tabs scroll sideways, with the mouse wheel too.
$("tabbar").addEventListener("wheel", (e) => {
  if (e.deltaY === 0 || e.deltaX !== 0) return;
  e.preventDefault();
  $("tabbar").scrollLeft += e.deltaY;
}, { passive: false });

// ---- The phone's toolbar under the text: each button does what its key does ----

const toolbar = $("write-toolbar");
for (const button of toolbar.querySelectorAll<HTMLButtonElement>("button")) {
  // The editor keeps the focus, so the keyboard stays open.
  button.addEventListener("pointerdown", (e) => e.preventDefault());
  button.addEventListener("mousedown", (e) => e.preventDefault());
}
for (const button of toolbar.querySelectorAll<HTMLButtonElement>("button[data-command]")) {
  button.addEventListener("click", () => tabs.runCommand(button.dataset.command as CommandId));
}
$("toolbar-view").addEventListener("click", () => setFormatted(!tabs.formatted));

/** Shown with a tab in front; Comments, the History and the views are a Work's only. */
function showToolbar() {
  toolbar.hidden = tabs.active === null;
  for (const id of ["addComment", "footnote", "insertImage", "textHistory"]) {
    toolbar.querySelector<HTMLButtonElement>(`[data-command="${id}"]`)!.disabled = !tabs.activeIsWork;
  }
  $<HTMLButtonElement>("toolbar-view").disabled = !tabs.activeIsWork;
}

// ---- Keyboard shortcuts: the app-wide ones (the editor's are in each editor's keymap) ----

new AppKeys({
  tabs,
  vaultOpen: () => browser.isOpen,
  sidebarHidden: () => $("app").classList.contains("sidebar-hidden"),
  setSidebar,
  openSwitcher,
  setFormatted,
  togglePanel: () => panel.toggle(),
  editPreamble: (id) => preambleActions.editPreamble(id),
  editMods,
  openFind: (replace) => findBar.open(replace),
  openFindInVault,
  zoom: preview.zoom,
  keysChanged: () => {
    panel.showToggle();
    findBar.showKeys();
    commentsPanel.render();
  },
});

// ---- Settings (settingsdialog.ts) ----

const settings = new SettingsDialog({ tabs, readingPanel, preview, browser, findInVault: { refresh: () => findInVault.refresh() } });

// ---- App updates (ADR 0014) ----

/** Everything typed goes to disk before an update replaces the app. */
async function saveEverything() {
  await tabs.idle();
  if (!browser.isOpen) return;
  for (const { id, state } of await invoke<{ id: string; state: WorkState }[]>("app_hidden")) tabs.status(id, state);
}

const updates = new UpdateDialog(saveEverything);
onUpdatesAsked(() => updates.open());
$("check-updates").addEventListener("click", () => act(() => updates.open()));
const uninstaller = new UninstallDialog(saveEverything);
$("open-uninstall").addEventListener("click", () => act(() => uninstaller.open()));

// ---- The window's frame and title (windowframe.ts) ----

const frame = WindowFrame.start();

/** The window's title follows the tab in front and the open Vault (their names as shown). */
function showTitle() {
  const vault = browser.name;
  void frame.then((f) => f.title(tabs.activeName, vault));
}
const titleWatch = new MutationObserver(showTitle);
titleWatch.observe($("tabbar"), { subtree: true, childList: true, characterData: true, attributeFilter: ["class"] });
titleWatch.observe($("vault-name"), { subtree: true, childList: true, characterData: true });
showTitle();

invoke<{ version: string; canUninstall: boolean; canChooseFrame: boolean; nativeFrame: boolean }>("app_info").then(
  (info) => {
    $("app-version").textContent = `Wrtier ${info.version}`;
    $("open-uninstall").hidden = !info.canUninstall;
    void frame.then((f) => f.showSetting(info.canChooseFrame, info.nativeFrame));
  },
  () => {},
);

// ---- Start ----

window.addEventListener("DOMContentLoaded", () => act(async () => {
  const { lastVault } = await invoke<{ lastVault: string | null }>("startup");
  // On Android the access can be turned off in Settings while the app is closed.
  if (lastVault && (await ensureFilesAccess())) await browser.open(lastVault);
}));
