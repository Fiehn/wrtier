// The parts split out of main.ts: what each works out by itself, without the window.

import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
// (android.ts sets up its folder browser in the page as it loads.)
vi.mock("./android", () => ({ onAndroid: false, browseForVault: vi.fn(), ensureFilesAccess: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

import { elementInfos, missingQuestion, pdfExported, restoredLine } from "./preambleactions";
import { entryNote, folderPath, isDisabled, pickedAfterFolderMove, pickedAfterRename, vaultName, type Entry } from "./vaultbrowser";
import { dropFolder, RefusedLine } from "./movingworks";
import { emptyLine, runAppCommand } from "./appkeys";
import { settingsTabs, sharedValueText, tabAfter } from "./settingsdialog";
import { problemsTitle } from "./ui";
import type { LibraryItem } from "./library";

const key = (k: string, mods: Partial<{ ctrlKey: boolean; shiftKey: boolean; altKey: boolean; metaKey: boolean }> = {}) =>
  ({ key: k, code: `Key${k.toUpperCase()}`, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods });

describe("a Work's Preamble and Exports", () => {
  it("lists the library's Story elements, and which the Work has", () => {
    const items = [
      { kind: "storyElement", name: "part", title: "Part", shownAs: "heading" },
      { kind: "storyElement", name: "letter", title: "Letter" },
      { kind: "style", name: "Drop caps", title: "Drop caps" },
    ] as unknown as LibraryItem[];
    expect(elementInfos(items, ["part"], ["letter"])).toEqual([
      { name: "part", title: "Part", shownAs: "heading", inWork: true },
      { name: "letter", title: "Letter", shownAs: "block", inWork: true },
    ]);
    expect(elementInfos(items, [], []).map((e) => e.inWork)).toEqual([false, false]);
  });

  it("asks before exporting without Inclusions that can't be shown, naming three at most", () => {
    expect(missingQuestion(["Rain: in the trash."])).toBe("One Inclusion can't be shown: Rain: in the trash. The Export would have a line saying so in its place.");
    const four = missingQuestion(["A.", "B.", "C.", "D."]);
    expect(four).toBe("4 Inclusions can't be shown: A. B. C. And 1 more. The Export would have a line saying so in their place.");
  });

  it("says what a PDF Export made, and which fonts came from this Device", () => {
    expect(pdfExported({ path: "C:/Rain.pdf", pages: 1, deviceFonts: [] }, null)).toBe("Exported 1 page to C:/Rain.pdf.");
    expect(pdfExported({ path: "Rain.pdf", pages: 12, deviceFonts: ["Garamond"] }, { id: "m", name: "Draft 2" }))
      .toBe("Exported “Draft 2” 12 pages to Rain.pdf. Garamond came from this Device, not the Vault, so a PDF from another Device would differ.");
  });

  it("names the defaults put back", () => {
    expect(restoredLine([])).toBe("The defaults were already as this version of the app has them.");
    expect(restoredLine(["Manuscript.typ"])).toBe("Restored 1 default: Manuscript.");
    expect(restoredLine(["elements/part.typ", "Paperback.look"])).toBe("Restored 2 defaults: part, Paperback.");
  });
});

describe("the Vault browser", () => {
  const entry = (e: Partial<Entry> & Pick<Entry, "kind">) => ({ id: "Rain", name: "Rain", ...e }) as Entry;

  it("says why an entry can't simply be opened", () => {
    expect(entryNote(entry({ kind: "work" }))).toBe("");
    expect(entryNote(entry({ kind: "leftover", of: "Old/Rain" }))).toBe("Text file left over from Old/Rain");
    expect(entryNote(entry({ kind: "blocked", reason: "Rain.db belongs to another program." }))).toBe("Rain.db belongs to another program.");
    expect(entryNote(entry({ kind: "newWork" }))).toMatch(/opening it starts its History/);
    expect(isDisabled(entry({ kind: "work" }))).toBe(false);
    expect(isDisabled(entry({ kind: "newWork" }))).toBe(false);
    expect(isDisabled(entry({ kind: "newer", version: 9 }))).toBe(true);
  });

  it("keeps the Works picked for a Collection when they're renamed or their folder moves", () => {
    expect(pickedAfterRename(["Rain", "Snow"], "Rain", "Old/Rain")).toEqual(["Old/Rain", "Snow"]);
    expect(pickedAfterFolderMove(["Old/Rain", "Older/Snow", "Sun"], "Old", "Stories/Old")).toEqual(["Stories/Old/Rain", "Older/Snow", "Sun"]);
    expect(folderPath(" Stories / Old ")).toBe("Stories/Old");
  });

  it("names the Vault after its folder", () => {
    expect(vaultName("C:\\Users\\me\\White Vault")).toBe("White Vault");
    expect(vaultName("/home/me/White Vault/")).toBe("White Vault");
    expect(vaultName("C:/Fake Vault")).toBe("Fake Vault");
  });
});

describe("moving Works and folders", () => {
  it("drops into the folder of the row it's over, or the top of the Vault", () => {
    expect(dropFolder({ work: "Stories/Rain" })).toBe("Stories");
    expect(dropFolder({ work: "Rain" })).toBe("");
    expect(dropFolder({ folder: "Stories/Old" })).toBe("Stories/Old");
    expect(dropFolder(null)).toBe("");
  });

  it("says why it can't go somewhere once, until the reason changes", () => {
    const line = new RefusedLine();
    expect(line.next("Rain is there already.")).toBe("Rain is there already.");
    expect(line.next("Rain is there already.")).toBeNull();
    expect(line.next(null)).toBeNull();
    expect(line.next("Rain is there already.")).toBe("Rain is there already.");
    line.reset();
    expect(line.next("Rain is there already.")).toBe("Rain is there already.");
  });
});

describe("the app-wide keys", () => {
  it("runs the first command whose key it is that applies now", () => {
    const ran: string[] = [];
    const commands = {
      closeTab: () => (ran.push("closeTab"), false), // doesn't apply: the key goes on
      quickSwitcher: () => (ran.push("quickSwitcher"), true),
    };
    expect(runAppCommand(key("o", { ctrlKey: true }), commands)).toBe("quickSwitcher");
    expect(runAppCommand(key("w", { ctrlKey: true }), commands)).toBeNull();
    expect(ran).toEqual(["quickSwitcher", "closeTab"]);
    expect(runAppCommand(key("q"), commands)).toBeNull();
  });

  it("names the Quick switcher's key in an empty window, if it has one", () => {
    expect(emptyLine("Ctrl+O")).toBe("Open a Vault, then choose a Work on the left, or press Ctrl+O.");
    expect(emptyLine("")).toBe("Open a Vault, then choose a Work on the left.");
  });
});

describe("Settings", () => {
  const shared = { theme: "vault:paper.css", fontSize: 17, lineNumbers: true, highlightLine: false, previewFollows: true, commentHighlights: true };

  it("says what the other Devices use", () => {
    expect(sharedValueText("fontSize", shared, () => undefined)).toBe("17");
    expect(sharedValueText("highlightLine", shared, () => undefined)).toBe("off");
    expect(sharedValueText("theme", shared, (v) => (v === "vault:paper.css" ? "Paper" : undefined))).toBe("Paper");
    expect(sharedValueText("theme", shared, () => undefined)).toBe("paper.css");
  });

  it("has only the App tab with no Vault open, and arrows go round the tabs", () => {
    expect(settingsTabs(false)).toEqual(["app"]);
    expect(settingsTabs(true)).toEqual(["editor", "reviewers", "vault", "shortcuts", "app"]);
    expect(tabAfter("editor", "ArrowRight")).toBe("reviewers");
    expect(tabAfter("app", "ArrowRight")).toBe("editor");
    expect(tabAfter("editor", "ArrowLeft")).toBe("app");
  });

  it("counts Sync problems", () => {
    expect(problemsTitle(1)).toBe("1 Sync problem");
    expect(problemsTitle(3)).toBe("3 Sync problems");
  });
});
