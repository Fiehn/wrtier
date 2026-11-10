// The Licenses dialog (Settings → Licenses…): the open-source libraries and fonts the app ships
// with, and the license of each. public/licenses.json is written at build time by
// scripts/licenses.mjs (`npm run licenses`).

type Item = { license: string; usedBy: string[]; text: string };
type Section = { title: string; about: string; items: Item[] };

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** "a, b, c and 4 more" */
function names(usedBy: string[]): string {
  const shown = usedBy.slice(0, 3).join(", ");
  return usedBy.length > 3 ? `${shown} and ${usedBy.length - 3} more` : shown;
}

function item({ license, usedBy, text }: Item): HTMLElement {
  const details = el("details", "license");
  const summary = el("summary", "");
  summary.append(el("span", "license-name", license), el("span", "muted", ` · ${names(usedBy)}`));
  // The text is only built when it's opened: the whole list is several hundred kB.
  details.addEventListener("toggle", () => {
    if (!details.open || details.childElementCount > 1) return;
    details.append(el("p", "license-used muted", `Used by ${usedBy.join(", ")}.`), el("pre", "", text));
  });
  details.append(summary);
  return details;
}

let loaded = false;

/** Opens the dialog. Resolves when it's closed. */
export async function openLicenses(): Promise<void> {
  const dialog = $<HTMLDialogElement>("licenses");
  const list = $("licenses-list");
  const error = $("licenses-error");
  if (!loaded) {
    try {
      const res = await fetch("/licenses.json");
      if (!res.ok) throw new Error(String(res.status));
      const { sections } = (await res.json()) as { sections: Section[] };
      list.replaceChildren(
        ...sections.flatMap((s) => [el("h3", "", s.title), el("p", "muted", s.about), ...s.items.map(item)]),
      );
      error.hidden = true;
      loaded = true;
    } catch {
      error.textContent = "This build has no list of licenses. Run “npm run licenses” in the app folder, then reopen this.";
      error.hidden = false;
    }
  }
  await new Promise<void>((resolve) => {
    dialog.addEventListener("close", () => resolve(), { once: true });
    dialog.showModal();
  });
}
