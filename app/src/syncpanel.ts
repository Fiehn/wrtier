// How the Vault is syncing: a line at the top of the sidebar, and the Sync dialog it opens.
//
// Rust asks Syncthing's REST API (syncthing.rs); this only shows the answer and sends the
// Author's choices back. The line is asked for every few seconds while the window shows, and
// the dialog's lists more often while it's open. The forms in the dialog (adding a Device,
// Syncthing's address and key) are never redrawn, so nothing half-typed is lost.

import { invoke } from "@tauri-apps/api/core";
import { dayOf } from "./historypanel";

export type Peer = { id: string; name: string; connected: boolean; completion: number; needItems: number; lastSeen: string | null };
export type Waiting = { id: string; name: string; offersFolder: boolean };
export type SyncStatus =
  | { kind: "notFound"; line: string }
  | { kind: "notRunning"; line: string; address: string }
  | { kind: "refused"; line: string; address: string }
  | { kind: "failed"; line: string; reason: string }
  | { kind: "notInFolder"; line: string; myId: string }
  | { kind: "folder"; line: string; state: string; myId: string; folderId: string; folderLabel: string; peers: Peer[]; waiting: Waiting[] };

/** How often the line is asked for, and the dialog's lists while it's open. */
const LINE_MS = 5000;
const DIALOG_MS = 2000;

/** "Connected", "Last seen today 14:02", "Never seen". */
export function peerState(peer: Peer, now = Date.now()): string {
  if (peer.connected) return "Connected";
  if (!peer.lastSeen) return "Never seen";
  const ms = Date.parse(peer.lastSeen);
  if (Number.isNaN(ms)) return "Not connected";
  const day = dayOf(ms, now);
  const shown = day === "Today" || day === "Yesterday" ? day.toLowerCase() : day;
  const time = new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `Last seen ${shown} ${time}`;
}

/** What a Sync peer still lacks, if anything: "has 87%, lacks 16 files". */
export function peerCompletion(peer: Peer): string {
  if (peer.completion >= 100 && peer.needItems === 0) return "";
  const lacks = peer.needItems > 0 ? `, lacks ${peer.needItems} ${peer.needItems === 1 ? "file" : "files"}` : "";
  return `has ${Math.floor(Math.max(0, peer.completion))}%${lacks}`;
}

/** The explanation in the dialog for a status without a folder to show. */
export function explanation(status: SyncStatus): string {
  switch (status.kind) {
    case "notFound":
      return "wrtier works without Syncthing, but this Device's Syncthing wasn't found. If Syncthing runs here, give its address and API key below (Syncthing shows the key under Actions → Settings → GUI). On Android, paste the key from the Syncthing app's settings.";
    case "notRunning":
      return `Nothing answered at ${status.address}. Start Syncthing on this Device, and this shows how the Vault syncs. Writing works as usual meanwhile.`;
    case "refused":
      return `Syncthing at ${status.address} didn't accept the API key. Give the current key below (Syncthing shows it under Actions → Settings → GUI).`;
    case "failed":
      return `Syncthing couldn't be asked: ${status.reason}`;
    case "notInFolder":
      return "This Vault isn't in a Syncthing folder, so nothing syncs it to your other Devices yet. Sharing it makes the Vault's folder a Syncthing folder; then add the Devices to share it with.";
    case "folder":
      return "";
  }
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

export class SyncPanel {
  private status: SyncStatus | null = null;
  private lineTimer: number | undefined;
  private dialogTimer: number | undefined;
  private asking = false;
  private open = false;

  constructor(private readonly error: (message: string) => void, private readonly confirm: (text: string, yes: string) => Promise<boolean>) {
    $("sync-status").addEventListener("click", () => void this.show());
    $("sync-share").addEventListener("click", () => void this.run(() => invoke("share_vault_with_syncthing")));
    $("sync-copy").addEventListener("click", () => {
      if (this.status && "myId" in this.status) void navigator.clipboard.writeText(this.status.myId).then(() => this.flash("sync-copy", "Copied"));
    });
    $("sync-add-form").addEventListener("submit", (event) => {
      event.preventDefault();
      const id = $<HTMLInputElement>("sync-add-id").value.trim();
      const name = $<HTMLInputElement>("sync-add-name").value.trim();
      if (!id) return;
      void this.run(async () => {
        await invoke("share_with_peer", { id, name });
        $<HTMLInputElement>("sync-add-id").value = "";
        $<HTMLInputElement>("sync-add-name").value = "";
      });
    });
    $("sync-save").addEventListener("click", () => void this.run(() =>
      invoke("set_syncthing", { address: $<HTMLInputElement>("sync-address").value, apiKey: $<HTMLInputElement>("sync-key").value })));
    $("sync-forget").addEventListener("click", () => void this.run(async () => {
      await invoke("set_syncthing", { address: "", apiKey: "" });
      $<HTMLInputElement>("sync-address").value = "";
      $<HTMLInputElement>("sync-key").value = "";
    }));
    $("sync").addEventListener("close", () => {
      this.open = false;
      $("sync-error").hidden = true;
      clearTimeout(this.dialogTimer);
    });
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden && this.lineTimer !== undefined) void this.refresh();
    });
  }

  /** A Vault opened: the line shows, and is kept up to date. */
  start() {
    $("sync-status").hidden = false;
    clearInterval(this.lineTimer);
    this.lineTimer = window.setInterval(() => {
      if (!document.hidden) void this.refresh();
    }, LINE_MS);
    void this.refresh();
  }

  /** Asks Rust again and redraws. Overlapping asks are skipped: the next tick catches up. */
  async refresh() {
    if (this.asking) return;
    this.asking = true;
    try {
      this.status = await invoke<SyncStatus>("sync_status");
    } catch (e) {
      this.status = { kind: "failed", line: "Sync status unavailable", reason: String(e) };
    } finally {
      this.asking = false;
    }
    this.draw();
  }

  private async show() {
    const manual = await invoke<{ address: string; apiKey: string } | null>("get_syncthing").catch(() => null);
    $<HTMLInputElement>("sync-address").value = manual?.address ?? "";
    $<HTMLInputElement>("sync-key").value = manual?.apiKey ?? "";
    this.open = true;
    await this.refresh();
    const dialog = $<HTMLDialogElement>("sync");
    if (!dialog.open) dialog.showModal();
    const tick = () => {
      this.dialogTimer = window.setTimeout(async () => {
        if (!this.open) return;
        await this.refresh();
        tick();
      }, DIALOG_MS);
    };
    tick();
  }

  /** Does something the Author asked for, then shows the result. */
  private async run(action: () => Promise<unknown>) {
    const shown = $("sync-error");
    shown.hidden = true;
    try {
      await action();
    } catch (e) {
      // In the dialog, where the Author is looking; the window's own error line is behind it.
      if (this.open) {
        shown.textContent = String(e);
        shown.hidden = false;
      } else {
        this.error(String(e));
      }
    }
    await this.refresh();
  }

  private flash(id: string, text: string) {
    const button = $(id);
    const was = button.textContent;
    button.textContent = text;
    setTimeout(() => (button.textContent = was), 1200);
  }

  private draw() {
    const status = this.status;
    if (!status) return;
    const line = $("sync-status");
    line.textContent = status.line;
    line.dataset.state = status.kind === "folder" ? status.state : status.kind === "notInFolder" ? "alone" : "off";
    line.title = "How this Vault syncs through Syncthing. Click for the Sync dialog.";
    $("sync-line").textContent = status.line;
    $("sync-line").dataset.state = line.dataset.state;

    const text = explanation(status);
    $("sync-explain").textContent = text;
    $("sync-explain").hidden = !text;
    $("sync-share").hidden = status.kind !== "notInFolder";
    const hasId = status.kind === "folder" || status.kind === "notInFolder";
    $("sync-me").hidden = !hasId;
    if (hasId) $("sync-my-id").textContent = status.myId;
    $("sync-folder").hidden = status.kind !== "folder";
    $("sync-add").hidden = status.kind !== "folder";
    // Needed only when Syncthing can't be reached as found; otherwise tucked away.
    const details = $<HTMLDetailsElement>("sync-connection");
    if (status.kind === "notFound" || status.kind === "refused") details.open = true;
    if (status.kind !== "folder") return;

    $("sync-folder-label").textContent = `Syncthing folder “${status.folderLabel}”`;
    const peers = $("sync-peers");
    peers.replaceChildren();
    if (status.peers.length === 0) {
      const li = document.createElement("li");
      li.className = "muted";
      li.textContent = "Not shared with another Device yet. Add one below.";
      peers.append(li);
    }
    for (const peer of status.peers) {
      const li = document.createElement("li");
      const name = document.createElement("strong");
      name.textContent = peer.name;
      const about = document.createElement("span");
      about.className = "muted";
      about.textContent = [peerState(peer), peerCompletion(peer)].filter(Boolean).join(" · ");
      // A file the other Device ignores counts as missing forever, as in Syncthing's own pages.
      if (peer.needItems > 0) about.title = "Files it ignores count as missing too: Syncthing doesn't know another Device's ignore patterns.";
      li.dataset.connected = String(peer.connected);
      li.append(name, about);
      peers.append(li);
    }

    const waiting = $("sync-waiting-list");
    waiting.replaceChildren();
    $("sync-waiting").hidden = status.waiting.length === 0;
    for (const w of status.waiting) {
      const li = document.createElement("li");
      const name = document.createElement("strong");
      name.textContent = w.name;
      const about = document.createElement("span");
      about.className = "muted";
      about.textContent = `${w.offersFolder ? "shares this Vault and is waiting for this Device" : "asks to connect"} · ${w.id.split("-")[0]}…`;
      about.title = w.id;
      const accept = document.createElement("button");
      accept.type = "button";
      accept.textContent = "Accept";
      accept.title = "Share this Vault with it";
      accept.addEventListener("click", () => void (async () => {
        if (!w.offersFolder && !(await this.confirm(`Share this Vault with “${w.name}” (${w.id})? Only accept a Device you know is yours.`, "Share"))) return;
        await this.run(() => invoke("share_with_peer", { id: w.id, name: w.name }));
      })());
      li.append(name, about, accept);
      waiting.append(li);
    }
  }
}
