import { describe, expect, it } from "vitest";
import { buildTree, folderOf, foldersOf, hiddenFolderAt, withoutHidden, type Folder } from "./tree";

/** The tree as indented lines: folders end with a slash. */
function lines(folder: Folder<{ id: string }>, depth = 0): string[] {
  const pad = "  ".repeat(depth);
  return [
    ...folder.folders.flatMap((f) => [`${pad}${f.name}/`, ...lines(f, depth + 1)]),
    ...folder.items.map((item) => `${pad}${item.id.split("/").pop()}`),
  ];
}

describe("the Vault browser's tree", () => {
  it("groups Works by folder, folders first, in name order", () => {
    const items = ["The Long Night", "Short stories/Rain", "Short stories/Old/Storm", "Anna", "Short stories/Draft 10", "Short stories/Draft 9"]
      .map((id) => ({ id }));
    expect(lines(buildTree(items, ["Empty", "Short stories/Old"]))).toEqual([
      "Empty/",
      "Short stories/",
      "  Old/",
      "    Storm",
      "  Draft 9",
      "  Draft 10",
      "  Rain",
      "Anna",
      "The Long Night",
    ]);
  });

  it("knows a folder's path", () => {
    const tree = buildTree([{ id: "a/b/c" }], []);
    expect(tree.folders[0].folders[0].path).toBe("a/b");
  });

  it("names the folders a Work is inside", () => {
    expect(foldersOf("Short stories/Old/Rain")).toEqual(["Short stories", "Short stories/Old"]);
    expect(foldersOf("Rain")).toEqual([]);
  });
});

describe("Hidden folders", () => {
  it("finds the outermost Hidden folder a folder is, or is inside", () => {
    const hidden = ["Old drafts/2023", "Old drafts"];
    expect(hiddenFolderAt("Old drafts", hidden)).toBe("Old drafts");
    expect(hiddenFolderAt("Old drafts/2023/Spring", hidden)).toBe("Old drafts");
    expect(hiddenFolderAt("Old drafts copy", hidden)).toBeNull();
    expect(hiddenFolderAt("", hidden)).toBeNull();
    expect(folderOf("Short stories/Old/Rain")).toBe("Short stories/Old");
    expect(folderOf("Rain")).toBe("");
  });

  it("leaves out what's in them, and counts each shut one once", () => {
    const items = ["Rain", "Old drafts/Rain v1", "Old drafts/2023/Fog", "Old drafts copy/Storm"].map((id) => ({ id }));
    const folders = ["Old drafts", "Old drafts/2023", "Old drafts copy", "Notes"];
    // One inside another, and one gone from the Vault: neither counted.
    const shown = withoutHidden(items, folders, ["Old drafts", "Old drafts/2023", "Gone"]);
    expect(shown.items.map((e) => e.id)).toEqual(["Rain", "Old drafts copy/Storm"]);
    expect(shown.folders).toEqual(["Old drafts copy", "Notes"]);
    expect(shown.shut).toEqual(["Old drafts"]);
    expect(lines(buildTree(shown.items, shown.folders))).toEqual(["Notes/", "Old drafts copy/", "  Storm", "Rain"]);
  });
});
