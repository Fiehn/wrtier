// Uninstalling (Windows only; uninstall.rs): Settings → "Uninstall…". Removes the app and this
// Device's data on this PC, never anything in the Vault. Writing that never reached its Work
// (recovery files) is named first, since it would be lost.

import { invoke } from "@tauri-apps/api/core";

type UninstallCheck = {
  /** Why the app can't remove itself, if it can't. */
  problem: string | null;
  unsaved: { vault: string; work: string }[];
};

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

export class UninstallDialog {
  private readonly dialog = $<HTMLDialogElement>("uninstall");
  private readonly unsaved = $("uninstall-unsaved");
  private readonly unsavedList = $("uninstall-unsaved-list");
  private readonly error = $("uninstall-error");
  private readonly go = $<HTMLButtonElement>("uninstall-go");
  private readonly cancel = $<HTMLButtonElement>("uninstall-cancel");
  private busy = false;

  /** `saveAll` puts everything typed on disk first, so only writing that can't be saved is lost. */
  constructor(private readonly saveAll: () => Promise<void>) {
    this.dialog.addEventListener("cancel", (e) => {
      if (this.busy) e.preventDefault();
    });
    this.go.addEventListener("click", () => void this.uninstall());
  }

  async open() {
    await this.saveAll();
    const check = await invoke<UninstallCheck>("uninstall_check");
    this.busy = false;
    this.error.hidden = check.problem === null;
    this.error.textContent = check.problem ?? "";
    this.unsaved.hidden = check.unsaved.length === 0;
    this.unsavedList.replaceChildren(
      ...check.unsaved.map(({ vault, work }) => {
        const li = document.createElement("li");
        li.textContent = `${work} (in ${vault})`;
        return li;
      }),
    );
    this.go.hidden = check.problem !== null;
    this.go.disabled = false;
    this.go.textContent = check.unsaved.length > 0 ? "Uninstall anyway" : "Uninstall";
    this.cancel.disabled = false;
    this.dialog.showModal();
    this.cancel.focus(); // Enter never uninstalls by accident
  }

  private async uninstall() {
    if (this.busy) return;
    this.busy = true;
    this.go.disabled = true;
    this.cancel.disabled = true;
    try {
      await this.saveAll();
      await invoke("uninstall"); // the app quits; a helper deletes it once it has
    } catch (e) {
      this.busy = false;
      this.go.disabled = false;
      this.cancel.disabled = false;
      this.error.textContent = String(e);
      this.error.hidden = false;
    }
  }
}
