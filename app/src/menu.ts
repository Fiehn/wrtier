// A small right-click menu. It closes on a click elsewhere, Escape, scrolling, or the window
// losing focus; arrow keys move between its items.

export type MenuItem = { label: string; run: () => void } | "separator";

let open: HTMLElement | null = null;

function close() {
  open?.remove();
  open = null;
  window.removeEventListener("pointerdown", outside, true);
  window.removeEventListener("keydown", keys, true);
  window.removeEventListener("blur", close);
  window.removeEventListener("resize", close);
  window.removeEventListener("scroll", close, true);
}

function outside(e: PointerEvent) {
  if (open && !open.contains(e.target as Node)) close();
}

function keys(e: KeyboardEvent) {
  if (!open) return;
  if (e.key === "Escape") {
    e.preventDefault();
    close();
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    const buttons = [...open.querySelectorAll("button")];
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = (at + (e.key === "ArrowDown" ? 1 : buttons.length - 1)) % buttons.length;
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
  window.addEventListener("scroll", close, true);
}
