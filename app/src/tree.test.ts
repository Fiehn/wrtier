import { describe, expect, it } from "vitest";
import { buildTree, foldersOf, type Folder } from "./tree";

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
