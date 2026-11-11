// Android: "All files access" and the folder browser that picks a Vault (v1 plan, Platforms;
// ADR 0006). MainActivity.kt gives the page `window.WrtierAndroid`; on desktop there's none.

import { invoke } from "@tauri-apps/api/core";

interface Bridge {
  hasAllFilesAccess(): boolean;
  openAllFilesAccessSettings(): void;
  openAppInfo(): void;
  sharedStorage(): string;
  canInstallApps(): boolean;
  openInstallAppsSettings(): void;
  installApk(path: string): void;
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
 * A switch only Settings can turn on (Android never asks by itself): a dialog explains and
 * opens it, and coming back with it on closes the dialog. False if the Author closes the
 * dialog without turning it on.
 */
async function ensureSwitch(dialog: HTMLDialogElement, isOn: () => boolean): Promise<boolean> {
  if (isOn()) return true;
  const check = () => {
    if (document.visibilityState === "visible" && isOn()) dialog.close("granted");
  };
  document.addEventListener("visibilitychange", check);
  window.addEventListener("focus", check);
  try {
    await show(dialog);
    return isOn();
  } finally {
    document.removeEventListener("visibilitychange", check);
    window.removeEventListener("focus", check);
  }
}

/** True once the app has "All files access". */
export async function ensureFilesAccess(): Promise<boolean> {
  return !bridge || ensureSwitch($("files-access"), () => bridge.hasAllFilesAccess());
}

/** True once the app may open Android's installer on an update ("Install unknown apps"). */
export async function ensureInstallAccess(): Promise<boolean> {
  return !bridge || ensureSwitch($("install-access"), () => bridge.canInstallApps());
}

/** Opens Android's installer on a downloaded update. */
export function installApk(path: string) {
  bridge?.installApk(path);
}

$("files-access-open").addEventListener("click", () => bridge?.openAllFilesAccessSettings());
$("files-access-app-info").addEventListener("click", () => bridge?.openAppInfo());
$("install-access-open").addEventListener("click", () => bridge?.openInstallAppsSettings());

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
