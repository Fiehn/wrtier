// App updates (ADR 0014): the "Check for updates" dialog, opened from Settings and from a Sync
// problem or error that a newer version of the app caused. Rust finds, downloads, checks and
// installs the update (update.rs); on Android the APK goes to Android's installer.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ensureInstallAccess, installApk, onAndroid } from "./android";

type Checked =
  | { kind: "newest"; current: string }
  | { kind: "available"; current: string; version: string }
  | { kind: "noBuild"; current: string; version: string };

type Installed = { kind: "restarting" } | { kind: "apk"; path: string };

type Progress = { done: number; total: number | null };

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

const megabytes = (bytes: number) => (bytes / (1024 * 1024)).toFixed(1);

/** Debian asks for the password to install (pkexec); Windows and Android don't. */
const asksForPassword = !onAndroid && navigator.userAgent.includes("Linux");

export class UpdateDialog {
  private readonly dialog = $<HTMLDialogElement>("update");
  private readonly message = $("update-message");
  private readonly whatsNew = $("update-whats-new-row");
  private readonly progress = $<HTMLProgressElement>("update-progress");
  private readonly error = $("update-error");
  private readonly install = $<HTMLButtonElement>("update-install");
  private readonly close = $<HTMLButtonElement>("update-close");
  /** The App version on offer. */
  private version: string | null = null;
  /** Downloading or installing: the dialog stays open. */
  private busy = false;

  /** `saveAll` puts everything typed on disk before the app is replaced. */
  constructor(private readonly saveAll: () => Promise<void>) {
    this.dialog.addEventListener("cancel", (e) => {
      if (this.busy) e.preventDefault();
    });
    $("update-whats-new").addEventListener("click", () => {
      if (this.version) invoke("open_release_page", { version: this.version }).catch((e) => this.fail(e));
    });
    this.install.addEventListener("click", () => void this.download());
  }

  /** Opens the dialog and asks GitHub for the newest stable App version. */
  async open() {
    this.version = null;
    this.busy = false;
    this.show("Checking for updates…");
    this.whatsNew.hidden = true;
    this.progress.hidden = true;
    this.error.hidden = true;
    this.install.hidden = true;
    this.install.disabled = false;
    this.close.disabled = false;
    this.close.textContent = "Close";
    if (!this.dialog.open) this.dialog.showModal();
    try {
      const found = await invoke<Checked>("check_update");
      switch (found.kind) {
        case "newest":
          this.show(`You have the newest version (${found.current}).`);
          break;
        case "available":
          this.version = found.version;
          this.show(`Version ${found.version} is available (you have ${found.current}).`);
          this.whatsNew.hidden = false;
          this.install.hidden = false;
          this.close.textContent = "Not now";
          this.install.focus();
          break;
        case "noBuild":
          this.version = found.version;
          this.show(`Version ${found.version} is out (you have ${found.current}), but it has no build for this device yet.`);
          this.whatsNew.hidden = false;
          break;
      }
    } catch (e) {
      this.show("");
      this.fail(e);
    }
  }

  private show(text: string) {
    this.message.textContent = text;
    this.message.hidden = text === "";
  }

  private fail(e: unknown) {
    this.error.textContent = String(e);
    this.error.hidden = false;
  }

  /** "Download and install". */
  private async download() {
    const version = this.version;
    if (!version || this.busy) return;
    if (onAndroid && !(await ensureInstallAccess())) return;
    this.busy = true;
    this.error.hidden = true;
    this.install.disabled = true;
    this.close.disabled = true;
    this.show("Saving…");
    const unlisten = await listen<Progress>("update-progress", ({ payload: { done, total } }) => {
      this.progress.hidden = false;
      if (total) {
        this.progress.max = total;
        this.progress.value = done;
      } else {
        this.progress.removeAttribute("value"); // size unknown: an indeterminate bar
      }
      if (total !== null && done >= total) {
        this.show(onAndroid ? "Downloaded." : asksForPassword ? "Installing… Your system may ask for your password." : "Installing…");
      } else {
        this.show(`Downloading… ${megabytes(done)}${total ? ` of ${megabytes(total)}` : ""} MB`);
      }
    });
    try {
      await this.saveAll();
      this.show("Downloading…");
      const installed = await invoke<Installed>("install_update", { version });
      if (installed.kind === "restarting") {
        this.show("Starting the new version…"); // the app quits in a moment
        return;
      }
      installApk(installed.path);
      this.busy = false;
      this.progress.hidden = true;
      this.install.hidden = true;
      this.close.disabled = false;
      this.close.textContent = "Close";
      this.show("Android's installer is open: tap Install. Wrtier closes while it updates; open it again afterwards.");
    } catch (e) {
      this.busy = false;
      this.progress.hidden = true;
      this.install.disabled = false;
      this.close.disabled = false;
      this.show(`Version ${version} is available.`);
      this.fail(e);
    } finally {
      unlisten();
    }
  }
}
