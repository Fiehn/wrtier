// A small right-click menu. It closes on a click elsewhere, Escape, scrolling, or the window
// losing focus; arrow keys move between its items.

/** One colour to pick in a row of them: `css` null is "no colour". */
export type Swatch = { label: string; css: string | null; chosen: boolean; run: () => void };

export type MenuItem = { label: string; run: () => void } | { swatches: Swatch[] } | "separator";

let open: HTMLElement | null = null;

function close() {
  open?.remove();
  open = null;
  window.removeEventListener("pointerdown", outside, true);
  window.removeEventListener("keydown", keys, true);
  window.removeEventListener("blur", close);
  window.removeEventListener("resize", close);
  window.removeEventListener("scroll", scrolled, true);
}

/** Scrolling anything but the menu itself (a long one scrolls) closes it. */
function scrolled(e: Event) {
  if (open && !open.contains(e.target as Node)) close();
}

function outside(e: PointerEvent) {
  if (open && !open.contains(e.target as Node)) close();
}

function keys(e: KeyboardEvent) {
  if (!open) return;
  if (e.key === "Escape") {
    e.preventDefault();
    close();
  } else if (["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(e.key)) {
    e.preventDefault();
    const buttons = [...open.querySelectorAll("button")];
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const forward = e.key === "ArrowDown" || e.key === "ArrowRight";
    const next = (at + (forward ? 1 : buttons.length - 1)) % buttons.length;
    buttons[next]?.focus();
  }
}

/** Shows a menu at (x, y) on the screen. */
export function showMenu(x: number, y: number, items: MenuItem[]) {
  close();
  if (items.length === 0) return;
  const menu = document.createElement("div");
  menu.id = "menu";
  menu.setAttribute("role", "menu");
  for (const item of items) {
    if (item === "separator") {
      menu.append(document.createElement("hr"));
      continue;
    }
    if ("swatches" in item) {
      const row = document.createElement("div");
      row.className = "menu-swatches";
      row.setAttribute("role", "radiogroup");
      for (const swatch of item.swatches) {
        const pick = document.createElement("button");
        pick.type = "button";
        pick.className = swatch.css ? "colour-pick" : "colour-pick colour-none";
        pick.setAttribute("role", "radio");
        pick.setAttribute("aria-checked", String(swatch.chosen));
        pick.setAttribute("aria-label", swatch.label);
        pick.title = swatch.label;
        if (swatch.css) pick.style.background = swatch.css;
        pick.addEventListener("click", () => {
          close();
          swatch.run();
        });
        row.append(pick);
      }
      menu.append(row);
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute("role", "menuitem");
    button.textContent = item.label;
    button.addEventListener("click", () => {
      close();
      item.run();
    });
    menu.append(button);
  }
  document.body.append(menu);
  // Keep it on the screen.
  const box = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - box.width - 4))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - box.height - 4))}px`;
  open = menu;
  menu.querySelector("button")?.focus();
  window.addEventListener("pointerdown", outside, true);
  window.addEventListener("keydown", keys, true);
  window.addEventListener("blur", close);
  window.addEventListener("resize", close);
  window.addEventListener("scroll", scrolled, true);
}
