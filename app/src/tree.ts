// The Vault browser's tree: Works grouped by the folders they're in.

export type Folder<E> = {
  /** Its own name ("Old"), and its path in the Vault ("Short stories/Old"; "" for the Vault itself). */
  name: string;
  path: string;
  folders: Folder<E>[];
  items: E[];
};

const byName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "base", numeric: true });

/**
 * Groups `items` (each with an `id` like "Short stories/Rain") by folder. `folders` adds
 * folders with nothing in them yet. Folders come before Works, each in name order.
 */
export function buildTree<E extends { id: string }>(items: E[], folders: string[]): Folder<E> {
  const root: Folder<E> = { name: "", path: "", folders: [], items: [] };
  const folderAt = (path: string): Folder<E> => {
    let node = root;
    if (path === "") return node;
    for (const part of path.split("/")) {
      const childPath = node.path ? `${node.path}/${part}` : part;
      let child = node.folders.find((f) => f.name === part);
      if (!child) {
        child = { name: part, path: childPath, folders: [], items: [] };
        node.folders.push(child);
      }
      node = child;
    }
    return node;
  };
  for (const path of folders) folderAt(path);
  for (const item of items) {
    const slash = item.id.lastIndexOf("/");
    folderAt(slash === -1 ? "" : item.id.slice(0, slash)).items.push(item);
  }
  const sort = (folder: Folder<E>) => {
    folder.folders.sort((a, b) => byName(a.name, b.name));
    folder.items.sort((a, b) => byName(a.id, b.id));
    folder.folders.forEach(sort);
  };
  sort(root);
  return root;
}

/** The folders a Work is inside, outermost first ("Short stories/Old/Rain" → "Short stories", "Short stories/Old"). */
export function foldersOf(id: string): string[] {
  const parts = id.split("/").slice(0, -1);
  return parts.map((_, i) => parts.slice(0, i + 1).join("/"));
}

/** The folder a Work is in ("Short stories/Old/Rain" → "Short stories/Old"; "" for none). */
export function folderOf(id: string): string {
  const slash = id.lastIndexOf("/");
  return slash === -1 ? "" : id.slice(0, slash);
}

/** The outermost of the `hidden` folders that `folder` is, or is inside; null if none. */
export function hiddenFolderAt(folder: string, hidden: string[]): string | null {
  if (folder === "" || hidden.length === 0) return null;
  return foldersOf(`${folder}/`).find((f) => hidden.includes(f)) ?? null;
}

/**
 * What the Vault browser shows: the Works and folders, without those at or inside a Hidden
 * folder. `shut` is the Hidden folders that are in the Vault and not inside another Hidden
 * folder (so each is counted once, and one that's gone isn't counted).
 */
export function withoutHidden<E extends { id: string }>(items: E[], folders: string[], hidden: string[]): { items: E[]; folders: string[]; shut: string[] } {
  return {
    items: items.filter((e) => hiddenFolderAt(folderOf(e.id), hidden) === null),
    folders: folders.filter((f) => hiddenFolderAt(f, hidden) === null),
    shut: hidden.filter((h) => folders.includes(h) && hiddenFolderAt(folderOf(h), hidden) === null),
  };
}
