// Android: "All files access" and the folder browser that picks a Vault (v1 plan, Platforms;
// ADR 0006). MainActivity.kt gives the page `window.WrtierAndroid`; on desktop there's none.

import { invoke } from "@tauri-apps/api/core";

interface Bridge {
  hasAllFilesAccess(): boolean;
  openAllFilesAccessSettings(): void;
  openAppInfo(): void;
  sharedStorage(): string;
}

type Folder = { name: string; path: string; vault: boolean; syncthing: boolean };
type FolderListing = { path: string; parent: string | null; vault: boolean; syncthing: boolean; folders: Folder[] };

const bridge = (window as unknown as { WrtierAndroid?: Bridge }).WrtierAndroid;

/** The app is running on Android. */
export const onAndroid = bridge !== undefined;

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

/** Opens a dialog; resolves with how it was closed. */
function show(dialog: HTMLDialogElement): Promise<string> {
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue), { once: true });
    dialog.returnValue = "";
    dialog.showModal();
  });
}

/**
 * True once the app has "All files access". Without it, a dialog explains and opens the
 * switch in Settings (Android never asks by itself); coming back with it on closes the dialog.
 * False if the Author closes the dialog without turning it on.
 */
export async function ensureFilesAccess(): Promise<boolean> {
  if (!bridge || bridge.hasAllFilesAccess()) return true;
  const dialog = $<HTMLDialogElement>("files-access");
  const check = () => {
    if (document.visibilityState === "visible" && bridge.hasAllFilesAccess()) dialog.close("granted");
  };
  document.addEventListener("visibilitychange", check);
  window.addEventListener("focus", check);
  try {
    await show(dialog);
    return bridge.hasAllFilesAccess();
  } finally {
    document.removeEventListener("visibilitychange", check);
    window.removeEventListener("focus", check);
  }
}

$("files-access-open").addEventListener("click", () => bridge?.openAllFilesAccessSettings());
$("files-access-app-info").addEventListener("click", () => bridge?.openAppInfo());

function row(label: string, tag: string, go: () => void): HTMLLIElement {
  const li = document.createElement("li");
  const button = document.createElement("button");
  button.type = "button";
  button.className = "folder-row";
  const name = document.createElement("span");
  name.textContent = label;
  button.append(name);
  if (tag) {
    const small = document.createElement("small");
    small.textContent = tag;
    button.append(small);
  }
  button.addEventListener("click", go);
  li.append(button);
  return li;
}

function tagOf(folder: { vault: boolean; syncthing: boolean }): string {
  return [folder.vault && "Vault", folder.syncthing && "Syncthing folder"].filter(Boolean).join(" · ");
}

/**
 * The folder browser: starts at the phone's shared storage and lists folders, marking Vaults and
 * Syncthing folders. Resolves with the folder chosen, or null if cancelled.
 */
export async function browseForVault(): Promise<string | null> {
  const dialog = $<HTMLDialogElement>("folders");
  const list = $("folders-list");
  const where = $("folders-path");
  const error = $("folders-error");
  const use = $<HTMLButtonElement>("folders-use");
  const top = bridge?.sharedStorage() ?? "/";
  let current = top;

  async function go(path: string) {
    try {
      const listing = await invoke<FolderListing>("list_folders", { path });
      current = listing.path;
      error.hidden = true;
      where.textContent = listing.path === top ? "Phone storage" : listing.path.slice(top.length + 1);
      const tag = tagOf(listing);
      where.title = tag;
      use.textContent = listing.vault ? "Open this Vault" : "Use this folder";
      const rows: HTMLElement[] = [];
      // Above the shared storage is the system's, which can't be read.
      if (listing.parent && listing.path !== top) rows.push(row("↑ Up", "", () => void go(listing.parent!)));
      rows.push(...listing.folders.map((f) => row(f.name, tagOf(f), () => void go(f.path))));
      if (listing.folders.length === 0) {
        const empty = document.createElement("li");
        empty.className = "muted";
        empty.textContent = "No folders in here.";
        rows.push(empty);
      }
      list.replaceChildren(...rows);
      list.scrollTop = 0;
    } catch (e) {
      error.textContent = String(e);
      error.hidden = false;
    }
  }

  await go(current);
  return (await show(dialog)) === "use" ? current : null;
}
