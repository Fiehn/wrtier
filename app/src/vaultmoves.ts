// Moving Works and folders in the Vault browser, by dragging one onto a folder or with
// "Move to…". A move is a rename into another folder (vault/moves.rs), so it takes everything
// with it: History, Comments, Inclusions, Mods and colour. It's never confirmed; it's undone by
// moving it back. These say where a move goes, or why it can't, before anything is asked of Rust.

import { folderOf } from "./tree";

/** What can be moved: one Work (by its ID) or one folder (by its path). */
export type Movable = { kind: "work"; id: string } | { kind: "folder"; path: string };

/** Where it would go; that it's there already; or why it can't go there. */
export type MoveResult = { to: string } | { same: true } | { refused: string };

/** What's in the Vault now, to tell a move that would take a name already used. */
export type Taken = { works: string[]; folders: string[] };

export const pathOf = (item: Movable): string => (item.kind === "work" ? item.id : item.path);

/** Is `folder` the folder `path`, or inside it? */
const within = (folder: string, path: string) => folder === path || folder.startsWith(`${path}/`);

// The Vault's file systems ignore case on Windows and Android's shared storage.
const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** Where `item` goes when dropped into the folder `into` ("" for the top of the Vault). */
export function whereTo(item: Movable, into: string, taken: Taken): MoveResult {
  const path = pathOf(item);
  if (folderOf(path) === into) return { same: true };
  if (item.kind === "folder" && within(into, path)) return { refused: "A folder can't go inside itself." };
  const name = path.split("/").pop()!;
  const to = into ? `${into}/${name}` : name;
  if (item.kind === "work" && taken.works.some((w) => same(w, to))) return { refused: `There's already a Work called ${to}.` };
  if (item.kind === "folder" && taken.folders.some((f) => same(f, to))) return { refused: `There's already a folder called ${to}.` };
  return { to };
}

/**
 * The folders "Move to…" offers (`folders`: those the Vault browser shows, so no Hidden
 * folders): "" for the top of the Vault first, then each folder, less the one it's in now and,
 * for a folder, itself and the folders inside it.
 */
export function destinations(item: Movable, folders: string[]): string[] {
  const path = pathOf(item);
  return ["", ...folders].filter((f) => f !== folderOf(path) && !(item.kind === "folder" && within(f, path)));
}

/** The open folders after a folder moved from `from` to `to`: those at or inside it, renamed along. */
export function movedFolders(open: Iterable<string>, from: string, to: string): string[] {
  return [...open].map((f) => (within(f, from) ? to + f.slice(from.length) : f));
}
