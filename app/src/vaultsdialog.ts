// Manage Vaults (v3 plan, Vault management; ADR 0022). On the left, this Device's Known Vaults:
// a click switches to one; ⋯ (or a right-click) offers Rename…, Move…, Open in Explorer, Remove
// from list, and Locate… for one whose folder is missing. On the right, Create a new Vault and
// Open a folder as a Vault, each with a tick to sync it with Syncthing, shown only while Syncthing
// runs on this Device and the folder isn't synced already.

import { invoke } from "@tauri-apps/api/core";
import { showMenu, type MenuItem } from "./menu";
import { browseForVault, ensureFilesAccess, onAndroid } from "./android";
import { $, act, askText, confirmAsk, showError, symbolButton, SYMBOLS } from "./ui";
import type { KnownVault, VaultBrowser } from "./vaultbrowser";

type Synced = { kind: "notSynced" } | { kind: "own"; id: string; label: string } | { kind: "inside"; id: string; label: string; path: string };
type SyncthingAt = { found: boolean; running: boolean; synced: Synced | null };

/** The folder that holds `path` (where a new Vault goes beside the open one). */
export function parentOf(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (cut <= 0) return trimmed;
  // "C:\Writing" → "C:\", not "C:".
  return /^[A-Za-z]:$/.test(trimmed.slice(0, cut)) ? trimmed.slice(0, cut + 1) : trimmed.slice(0, cut);
}

/** What a Vault's folder is shown in: Explorer on Windows, the file manager elsewhere. */
const SHOW_FOLDER = navigator.userAgent.includes("Windows") ? "Open in Explorer" : "Show in Files";

/** The Syncthing tick, or the note in its place, for a folder Syncthing may sync. */
class SyncTick {
  private asked = 0;

  constructor(private readonly row: HTMLElement, private readonly box: HTMLInputElement, private readonly note: HTMLElement) {}

  /** Shows the tick for a Vault that will be at (or inside) `path`; `ticked`: whether it starts ticked. */
  async show(path: string | null, ticked: boolean) {
    const asked = ++this.asked;
    this.row.hidden = true;
    this.note.hidden = true;
    if (!path) return;
    const at = await invoke<SyncthingAt>("syncthing_at", { path }).catch(() => null);
    if (asked !== this.asked || !at?.running || !at.synced) return;
    if (at.synced.kind === "notSynced") {
      this.box.checked = ticked;
      this.row.hidden = false;
      return;
    }
    this.note.textContent = at.synced.kind === "own"
      ? `Already synced by Syncthing (“${at.synced.label}”).`
      : `Syncthing syncs it as part of the folder “${at.synced.label}”.`;
    this.note.hidden = false;
  }

  /** It's shown and ticked: the new Vault becomes a Syncthing folder of its own. */
  get wanted(): boolean {
    return !this.row.hidden && this.box.checked;
  }
}

export class VaultsDialog {
  private readonly dialog = $<HTMLDialogElement>("vaults");
  private readonly list = $("vaults-list");
  private location: string | null = null;
  private chosen: string | null = null;
  /** The open Vault is synced with Syncthing: then a new one starts ticked to be too. */
  private openSynced = false;
  private readonly createTick = new SyncTick($("new-vault-sync-row"), $<HTMLInputElement>("new-vault-sync"), $("new-vault-sync-note"));
  private readonly openTick = new SyncTick($("open-vault-sync-row"), $<HTMLInputElement>("open-vault-sync"), $("open-vault-sync-note"));

  constructor(private readonly browser: VaultBrowser) {
    $("new-vault-browse").addEventListener("click", () => act(async () => {
      const picked = await this.pickFolder("Where should the new Vault go?");
      if (picked) this.setLocation(picked);
    }));
    $("new-vault-name").addEventListener("input", () => this.showNewFolder());
    $("new-vault-create").addEventListener("click", () => act(() => this.create()));
    $("open-vault-browse").addEventListener("click", () => act(async () => {
      const picked = await this.pickFolder("Open a folder as a Vault");
      if (!picked) return;
      this.chosen = picked;
      $("open-vault-path").textContent = picked;
      $("open-vault-path").classList.remove("muted");
      $<HTMLButtonElement>("open-vault-open").disabled = false;
      void this.openTick.show(picked, false);
    }));
    $("open-vault-open").addEventListener("click", () => act(() => this.openChosen()));
  }

  /** Opens the dialog, its list and its forms as they are now. */
  async open() {
    const current = this.browser.path;
    $<HTMLInputElement>("new-vault-name").value = "";
    this.setLocation(current ? parentOf(current) : null);
    this.chosen = null;
    $("open-vault-path").textContent = "No folder chosen";
    $("open-vault-path").classList.add("muted");
    $<HTMLButtonElement>("open-vault-open").disabled = true;
    void this.openTick.show(null, false);
    $("new-vault-error").hidden = true;
    await this.fill();
    if (!this.dialog.open) this.dialog.showModal();
    if (current) {
      const at = await invoke<SyncthingAt>("syncthing_at", { path: current }).catch(() => null);
      this.openSynced = at?.synced !== null && at?.synced !== undefined && at.synced.kind !== "notSynced";
      void this.createTick.show(this.location, this.openSynced);
    }
  }

  // ---- The Known Vaults ----

  private async fill() {
    const known = await invoke<KnownVault[]>("known_vaults");
    if (known.length === 0) {
      const none = document.createElement("li");
      none.className = "muted";
      none.textContent = "No Vaults yet on this Device. Create one, or open a folder as one.";
      this.list.replaceChildren(none);
      return;
    }
    this.list.replaceChildren(...known.map((k) => this.row(k)));
  }

  /** A Known Vault: its name and folder; clicked, it's opened; ⋯ for what can be done with it. */
  private row(k: KnownVault): HTMLElement {
    const li = document.createElement("li");
    li.className = `vault-row${k.open ? " open" : ""}${k.found ? "" : " missing"}`;
    const pick = document.createElement("button");
    pick.type = "button";
    pick.className = "vault-pick";
    pick.title = k.open ? "The Vault open now" : k.found ? `Switch to ${k.name}` : "Its folder isn't there: Locate it with ⋯";
    const name = document.createElement("span");
    name.className = "vault-row-name";
    name.textContent = k.open ? `✓ ${k.name}` : k.found ? k.name : `${k.name} (not found)`;
    const path = document.createElement("span");
    path.className = "vault-row-path";
    path.textContent = k.path;
    pick.append(name, path);
    pick.addEventListener("click", () => {
      if (k.open || !k.found) return;
      this.dialog.close();
      act(() => this.browser.open(k.path));
    });
    const more = symbolButton(`What to do with ${k.name}`, SYMBOLS.more, () => {
      const box = more.getBoundingClientRect();
      showMenu(box.left, box.bottom + 2, this.actions(k));
    });
    li.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      showMenu(e.clientX, e.clientY, this.actions(k));
    });
    li.append(pick, more);
    return li;
  }

  private actions(k: KnownVault): MenuItem[] {
    const items: MenuItem[] = [];
    if (k.found) {
      items.push({ label: "Rename…", run: () => act(() => this.rename(k)) });
      if (!onAndroid) {
        items.push({ label: "Move…", run: () => act(() => this.move(k)) });
        items.push({ label: SHOW_FOLDER, run: () => act(() => invoke("show_vault_folder", { path: k.path })) });
      }
    } else if (!onAndroid) {
      items.push({ label: "Locate…", run: () => act(() => this.locate(k)) });
    }
    if (!k.open) {
      if (items.length) items.push("separator");
      items.push({ label: "Remove from list", run: () => act(() => this.forget(k)) });
    }
    return items;
  }

  private async rename(k: KnownVault) {
    const name = await askText(`Rename the Vault “${k.name}” to:`, k.name, "Rename", "It's renamed on every Device. Its folder keeps its own name.");
    if (name === null || name.trim() === k.name) return;
    const kept = await invoke<string>("rename_vault", { path: k.path, name });
    if (k.open) {
      this.browser.renamed(kept);
      void invoke("follow_vault_name").catch(() => {});
    }
    await this.fill();
  }

  /** Moves a Vault's folder (Syncthing follows, or it's refused: see `move_vault`). The open Vault is closed first and opened again from its new place. */
  private async move(k: KnownVault) {
    const parent = await this.pickFolder(`Move “${k.name}” into…`);
    if (!parent) return;
    if (!(await confirmAsk(`Move “${k.name}” into ${parent}? Its folder goes there, with everything in it.`, "Move"))) return;
    if (k.open) await this.browser.release();
    let moved: string;
    try {
      moved = await invoke<string>("move_vault", { path: k.path, toParent: parent });
    } catch (e) {
      if (k.open) await this.browser.open(k.path); // not moved: open where it was
      throw e;
    }
    if (k.open) await this.browser.open(moved);
    await this.fill();
  }

  private async locate(k: KnownVault) {
    const found = await this.pickFolder(`Where is “${k.name}” now?`);
    if (!found) return;
    await invoke("locate_vault", { path: k.path, foundAt: found });
    await this.fill();
  }

  private async forget(k: KnownVault) {
    await invoke("forget_vault", { path: k.path });
    await this.fill();
  }

  // ---- Create a new Vault ----

  private setLocation(location: string | null) {
    this.location = location;
    $("new-vault-location").textContent = location ?? "Choose where it goes";
    $("new-vault-location").classList.toggle("muted", location === null);
    this.showNewFolder();
    void this.createTick.show(location, this.openSynced);
  }

  /** The folder Create will make, under the Location. */
  private showNewFolder() {
    const name = $<HTMLInputElement>("new-vault-name").value.trim();
    const line = $("new-vault-folder");
    line.hidden = !name || !this.location;
    if (!line.hidden) line.textContent = `Makes a folder for it in ${this.location}`;
  }

  private async create() {
    const error = $("new-vault-error");
    error.hidden = true;
    const name = $<HTMLInputElement>("new-vault-name").value.trim();
    if (!name) return this.refuse("Give the new Vault a name.");
    if (!this.location) return this.refuse("Choose where the new Vault goes.");
    let path: string;
    try {
      path = await invoke<string>("create_vault", { name, location: this.location });
    } catch (e) {
      return this.refuse(String(e));
    }
    const sync = this.createTick.wanted;
    this.dialog.close();
    if (await this.browser.open(path)) await this.syncOpen(sync);
  }

  private refuse(text: string) {
    const error = $("new-vault-error");
    error.textContent = text;
    error.hidden = false;
  }

  // ---- Open a folder as a Vault ----

  private async openChosen() {
    if (!this.chosen) return;
    const sync = this.openTick.wanted;
    this.dialog.close();
    if (await this.browser.open(this.chosen)) await this.syncOpen(sync);
  }

  /** The Vault just opened becomes a Syncthing folder of its own, if ticked. */
  private async syncOpen(wanted: boolean) {
    if (!wanted) return;
    await invoke("share_vault_with_syncthing").catch((e) => showError(`The Vault opened, but Syncthing couldn't be set up for it: ${e}`));
  }

  /** A folder, from the system's picker (on Android, the app's own folder browser). */
  private async pickFolder(title: string): Promise<string | null> {
    if (onAndroid) {
      if (!(await ensureFilesAccess())) return null;
      return browseForVault();
    }
    return invoke<string | null>("pick_folder", { title });
  }
}
