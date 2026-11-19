// What's done to a Work as a whole, from the Vault browser, the History tab or the Page
// preview: its Preamble, its Exports (PDF and .docx), naming a Milestone, and the Story elements
// its editor offers. Also the Style library's buttons for making something new or putting the
// defaults back.

import { invoke } from "@tauri-apps/api/core";
import type { WorkState } from "./sync";
import type { Tabs } from "./tabs";
import type { PagePreview } from "./preview";
import type { LibraryItem, LibraryPanel } from "./library";
import type { PreambleDialog, Preamble } from "./preambledialog";
import type { ReadingPanel } from "./reading";
import type { ElementInfo } from "./elements";
import { $, act, ask, askText, confirmAsk, deviceChoice, keepDeviceChoice, showInfo } from "./ui";

/** What these need from the rest of the window. */
export type PreambleDeps = {
  tabs: Tabs;
  library: LibraryPanel;
  preview: PagePreview;
  readingPanel: ReadingPanel;
  dialog: PreambleDialog;
  /** A Work's Mods follow its Preamble (its language). */
  refreshMods(id?: string): Promise<void>;
  openLibraryFile(path: string): Promise<void>;
};

/** A Milestone an Export can be of: the Work as it was then. */
type MilestoneRef = { id: string; name: string };

/** The Look picker's "No Look (prose only)": the .docx keeps only the prose and its structure. */
const PROSE_ONLY = "";

/** The Story elements the library has, and whether a Work with `own` (the Author's own it has) has each. */
export function elementInfos(items: LibraryItem[], builtIn: string[], own: string[]): ElementInfo[] {
  return items.filter((i) => i.kind === "storyElement").map((i) => ({
    name: i.name,
    title: i.title,
    shownAs: i.shownAs ?? "block",
    inWork: builtIn.includes(i.name) || own.includes(i.name),
  }));
}

/** What an Export asks before going ahead without the Inclusions in `missing` (each "label: why"). */
export function missingQuestion(missing: string[]): string {
  const shown = missing.slice(0, 3).join(" ");
  const more = missing.length > 3 ? ` And ${missing.length - 3} more.` : "";
  const what = missing.length === 1 ? "One Inclusion can't be shown" : `${missing.length} Inclusions can't be shown`;
  return `${what}: ${shown}${more} The Export would have a line saying so in ${missing.length === 1 ? "its" : "their"} place.`;
}

/** The line a PDF Export ends with. */
export function pdfExported(done: { path: string; pages: number; deviceFonts: string[] }, milestone: MilestoneRef | null): string {
  const pages = done.pages === 1 ? "1 page" : `${done.pages} pages`;
  const fonts = done.deviceFonts.length
    ? ` ${done.deviceFonts.join(", ")} came from this Device, not the Vault, so a PDF from another Device would differ.`
    : "";
  const what = milestone ? ` “${milestone.name}”` : "";
  return `Exported${what} ${pages} to ${done.path}.${fonts}`;
}

/** The line "Restore defaults" ends with, from the paths (in `styles/`) it wrote. */
export function restoredLine(restored: string[]): string {
  if (restored.length === 0) return "The defaults were already as this version of the app has them.";
  const names = restored.map((p) => p.replace(/^elements\//, "").replace(/\.(typ|look)$/, "")).join(", ");
  return `Restored ${restored.length === 1 ? "1 default" : `${restored.length} defaults`}: ${names}.`;
}

export class PreambleActions {
  constructor(private deps: PreambleDeps) {
    const { preview, library } = deps;
    $("preview-pdf").addEventListener("click", () => act(async () => {
      if (preview.shown) await this.exportPdf(preview.shown, preview.shownMilestone);
    }));
    $("preview-docx").addEventListener("click", () => act(async () => {
      if (preview.shown) await this.exportDocx(preview.shown, preview.shownMilestone);
    }));
    $("new-style").addEventListener("click", () => act(async () => {
      const name = await askText("What's the new Style called?", "", "Create", "One small thing it does, e.g. Drop caps or Large print.");
      if (name !== null) await library.create("style", name);
    }));
    $("new-look").addEventListener("click", () => act(async () => {
      const name = await askText("What's the new Look called?", "", "Create", "What it's for, e.g. Hardback or Large print.");
      if (name !== null) await library.create("look", name);
    }));
    $("new-element").addEventListener("click", () => act(async () => {
      const field = (id: string) => $<HTMLInputElement>(id);
      for (const id of ["element-name", "element-title", "element-description"]) field(id).value = "";
      const answer = ask($<HTMLDialogElement>("ask-element"));
      field("element-name").focus();
      if ((await answer) !== "ok") return;
      await library.create("storyElement", field("element-name").value.trim(), {
        title: field("element-title").value,
        shownAs: $<HTMLSelectElement>("element-shown").value,
        docx: $<HTMLSelectElement>("element-docx").value,
        description: field("element-description").value,
      });
    }));
    $("restore-defaults").addEventListener("click", () => act(async () => {
      const yes = await confirmAsk(
        "Put the Styles, Looks and Story elements the app comes with back as this version of the app has them? " +
        "Any you've changed go to the Vault's .trash folder first, and any you've removed come back. Your own are left alone. " +
        "Works pick up the new versions as they do any change in the Style library.",
        "Restore");
      if (!yes) return;
      await deps.tabs.idle(); // anything typed in a library file's tab is saved first, then trashed with it
      showInfo(restoredLine(await invoke<string[]>("restore_library_defaults")));
    }));
  }

  async editPreamble(id: string) {
    const { library, dialog, tabs } = this.deps;
    await library.refresh(); // the dialog offers what the library has now
    const name = id.split("/").pop() ?? id;
    const state = await dialog.open(id, name, library.items, library.builtIn);
    if (state === undefined) return; // cancelled
    if (state) tabs.status(id, state);
    await this.preambleChanged(id);
  }

  /**
   * Before an Export: if some of the Work's Inclusions can't be shown (the Work they include is
   * in the trash, not synced yet…), says which and asks. True to go ahead (they become a line
   * saying so in the Export).
   */
  private async inclusionsReady(id: string): Promise<boolean> {
    const missing = await invoke<string[]>("missing_inclusions", { id });
    return missing.length === 0 || confirmAsk(missingQuestion(missing), "Export anyway");
  }

  /**
   * Exports a Work as a PDF, in a Look picked here if it has more than one. `milestone`: export
   * the Work as it was then (what the Page preview shows), not as it is now.
   */
  async exportPdf(id: string, milestone: MilestoneRef | null = null) {
    const { tabs, preview } = this.deps;
    await tabs.settled(id);
    if (!milestone && !(await this.inclusionsReady(id))) return;
    const { preamble } = await invoke<{ preamble: Preamble }>("get_preamble", { id });
    let look: string | null = preamble.looks[0] ?? null;
    if (preamble.looks.length > 1) {
      const select = $<HTMLSelectElement>("ask-export-look");
      select.replaceChildren(...preamble.looks.map((name, i) => {
        const option = document.createElement("option");
        option.value = name;
        option.textContent = i === 0 ? `${name} (the Work's own)` : name;
        return option;
      }));
      select.value = preview.lookFor(id) ?? preamble.looks[0];
      $("ask-export-heading").textContent = `Export ${id.split("/").pop()} as PDF`;
      if ((await ask($<HTMLDialogElement>("ask-export"))) !== "export") return;
      look = select.value;
    }
    type Exported = { path: string; pages: number; deviceFonts: string[] };
    const done = await invoke<Exported | null>("export_pdf", { id, look, milestone: milestone?.id ?? null });
    if (!done) return; // cancelled
    showInfo(pdfExported(done, milestone));
  }

  /**
   * Exports a Work as a .docx in a Look, or as prose only, with its Comments if asked, and lists
   * what's worth knowing about it. `milestone`: the Work as it was then, never with Comments
   * (they're on the text as it is now).
   */
  async exportDocx(id: string, milestone: MilestoneRef | null = null) {
    const { tabs, preview, library } = this.deps;
    await tabs.settled(id);
    if (!milestone && !(await this.inclusionsReady(id))) return;
    const { preamble } = await invoke<{ preamble: Preamble }>("get_preamble", { id });
    const option = (value: string, text: string) => {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = text;
      return o;
    };
    const select = $<HTMLSelectElement>("ask-docx-look");
    select.replaceChildren(
      option(PROSE_ONLY, "No Look (prose only)"),
      ...preamble.looks.map((name, i) => option(name, i === 0 ? `${name} (the Work's own)` : name)),
    );
    select.value = deviceChoice("docx-prose-only") === "yes" ? PROSE_ONLY : (preview.lookFor(id) ?? preamble.looks[0] ?? PROSE_ONLY);
    const comments = $<HTMLInputElement>("ask-docx-comments");
    comments.checked = deviceChoice("docx-comments") === "yes";
    $("ask-docx-comments-row").hidden = milestone !== null;
    // Prose only is in Times New Roman, which there's no need to embed.
    const fonts = $<HTMLInputElement>("ask-docx-fonts");
    fonts.checked = deviceChoice("docx-embed-fonts") !== "no";
    const showFonts = () => ($("ask-docx-fonts-row").hidden = select.value === PROSE_ONLY);
    showFonts();
    select.onchange = showFonts;
    const what = milestone ? `“${milestone.name}” of ` : "";
    $("ask-docx-heading").textContent = `Export ${what}${id.split("/").pop()} as .docx`;
    if ((await ask($<HTMLDialogElement>("ask-docx"))) !== "export") return;
    const plain = select.value === PROSE_ONLY;
    keepDeviceChoice("docx-prose-only", plain ? "yes" : "no");
    if (!milestone) keepDeviceChoice("docx-comments", comments.checked ? "yes" : "no");
    if (!plain) keepDeviceChoice("docx-embed-fonts", fonts.checked ? "yes" : "no");
    type Exported = { path: string; report: { text: string; open: string | null }[] };
    const done = await invoke<Exported | null>("export_docx", {
      id,
      look: plain ? null : select.value,
      plain,
      comments: !milestone && comments.checked,
      embedFonts: !plain && fonts.checked,
      milestone: milestone?.id ?? null,
    });
    if (!done) return; // cancelled
    const exported = `Exported${milestone ? ` “${milestone.name}”` : ""} to ${done.path}.`;
    if (done.report.length === 0) {
      showInfo(exported);
      return;
    }
    $("docx-report-path").textContent = `${exported} Worth knowing:`;
    const dialog = $<HTMLDialogElement>("docx-report");
    // Each line can open the Style library file where it can be changed, if there's one.
    let opening: string | null = null;
    $("docx-report-list").replaceChildren(...done.report.map((line) => {
      const li = document.createElement("li");
      li.append(line.text);
      if (line.open) {
        const path = line.open;
        const button = document.createElement("button");
        button.type = "button";
        button.className = "link";
        button.textContent = `Open ${library.titleOf(path)}`;
        button.addEventListener("click", () => {
          opening = path;
          dialog.close("open");
        });
        li.append(" ", button);
      }
      return li;
    }));
    await ask(dialog);
    if (opening) await this.deps.openLibraryFile(opening);
  }

  /** Names the current point in a Work's History, to come back to or compare with later. */
  async nameMilestone(id: string) {
    const name = await askText(`Name this point in ${id}'s History:`, "", "Name it",
      "E.g. Draft 2, sent to editor. The Page preview can show the Work as it was then, at any time.");
    if (name === null) return;
    const state = await invoke<WorkState | null>("make_milestone", { id, name });
    if (state) this.deps.tabs.status(id, state);
    await this.deps.preview.milestonesChanged(id);
  }

  /** A Work's Preamble changed: its editor's Story elements and its pages follow. */
  async preambleChanged(id: string) {
    await this.refreshElements(id);
    await this.deps.preview.preambleChanged(id);
    await this.deps.refreshMods(id); // its language may have changed (the Dialogue checker is English only)
  }

  /** The Story elements the library has, and whether a Work with these of the Author's own has each. */
  elementInfos(own: string[]): ElementInfo[] {
    return elementInfos(this.deps.library.items, this.deps.library.builtIn, own);
  }

  /** Tells each open Work's editor (or just `only`) which Story elements it has. */
  async refreshElements(only?: string) {
    const { tabs, readingPanel } = this.deps;
    for (const id of tabs.works) {
      if (only && id !== only) continue;
      try {
        const { preamble } = await invoke<{ preamble: Preamble }>("get_preamble", { id });
        tabs.setElements(id, this.elementInfos(preamble.elements));
      } catch {
        // gone or unreadable right now: it keeps what it had
      }
    }
    if (!only || only === readingPanel.work) await readingPanel.elementsChanged();
  }

  /** "Add it?": a Story element from the library goes into the Work's Preamble. */
  async addElement(id: string, name: string) {
    const { preamble } = await invoke<{ preamble: Preamble }>("get_preamble", { id });
    if (!preamble.elements.includes(name)) {
      const state = await invoke<WorkState | null>("set_preamble", { id, preamble: { ...preamble, elements: [...preamble.elements, name] } });
      if (state) this.deps.tabs.status(id, state);
    }
    await this.preambleChanged(id);
  }
}
