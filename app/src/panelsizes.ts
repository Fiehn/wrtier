// How wide the Vault browser, the Reading panel and the Side panel are (v2 plan, "Resizable
// panels"). Each is dragged by its edge nearer the editor, and its width is Device state, kept in
// this Device's local storage; a panel never dragged has its default width. The editor is what's
// protected: it keeps at least EDITOR_MIN while the window allows it, the Side panel giving way
// first, then the Reading panel, then the Vault browser. The phone layout has no handles (each
// panel is a whole screen). The Reading panel is never hidden by dragging: hiding it is closing it.

export type PanelName = "vault" | "reading" | "side";

const PANELS: readonly PanelName[] = ["vault", "reading", "side"];

/** Which give way first when the window is too narrow for them all. */
const GIVING_WAY: readonly PanelName[] = ["side", "reading", "vault"];

/** The narrowest the editor gets while the panels can still give way. */
export const EDITOR_MIN = 400;

/**
 * The Side panel's narrowest: room for its whole tab row (six symbols and their counts), and with
 * the app's own frame (windowframe.ts) the window's buttons beside them, since the panel's row is
 * then the one at the window's top-right corner.
 */
export const SIDE_MIN = 290;
export const SIDE_MIN_FRAMED = 440;
let sideMin = SIDE_MIN;

/** The window has the app's own frame (or not): the Side panel's narrowest follows. */
export function setFramed(framed: boolean) {
  sideMin = framed ? SIDE_MIN_FRAMED : SIDE_MIN;
}

export const LIMITS: Record<PanelName, { readonly min: number; readonly max: number }> = {
  vault: { min: 180, max: 480 },
  reading: { min: 260, max: Infinity },
  side: { get min() { return sideMin; }, max: Infinity },
};

/** Arrow keys on a handle move it this far; with Shift, BIG_STEP. */
export const STEP = 16;
export const BIG_STEP = 64;

const STORAGE_KEYS: Record<PanelName, string> = { vault: "sidebar-width", reading: "reading-width", side: "panel-width" };

/** A panel's width when it was never dragged on this Device. */
export function defaultWidth(panel: PanelName, windowWidth: number): number {
  if (panel === "vault") return 280;
  if (panel === "reading") return Math.max(LIMITS.reading.min, Math.min(windowWidth * 0.34, 600));
  return Math.max(LIMITS.side.min, Math.min(windowWidth * 0.42, 640));
}

/** What this Device remembered: a width within the panel's own limits, or null for the default. */
export function readWidth(panel: PanelName, get: (key: string) => string | null): number | null {
  const saved = Number(get(STORAGE_KEYS[panel]) ?? NaN);
  if (!Number.isFinite(saved) || saved <= 0) return null;
  return clamp(Math.round(saved), LIMITS[panel].min, LIMITS[panel].max);
}

/** The widths wanted, null for a hidden panel. */
export type Widths = { vault: number | null; reading: number | null; side: number | null };

/**
 * The widths the panels are shown at in a window this wide. While the remembered widths leave
 * the editor less than EDITOR_MIN, the Side panel is shown narrower, down to its minimum, then
 * the Reading panel and then the Vault browser down to theirs; past that the editor gets
 * narrower. Nothing remembered changes, and no panel is hidden.
 */
export function fit(windowWidth: number, wanted: Widths): Widths {
  const out = { ...wanted };
  let over = PANELS.reduce((sum, name) => sum + (out[name] ?? 0), 0) - (windowWidth - EDITOR_MIN);
  for (const name of GIVING_WAY) {
    const width = out[name];
    if (over <= 0 || width === null) continue;
    const cut = Math.min(over, Math.max(0, width - LIMITS[name].min));
    out[name] = width - cut;
    over -= cut;
  }
  return out;
}

/**
 * The widest a panel can be dragged in a window this wide, given the others (their widths as
 * shown, null while hidden). It can grow until the editor is down to EDITOR_MIN, leaving each
 * panel that gives way before it (see `fit`) its minimum, since that one then gets narrower.
 * The Vault browser has a cap of its own.
 */
export function maxWidth(panel: PanelName, windowWidth: number, shown: Widths): number {
  const { min, max } = LIMITS[panel];
  const before = (other: PanelName) => GIVING_WAY.indexOf(other) < GIVING_WAY.indexOf(panel);
  const others = PANELS.filter((other) => other !== panel && shown[other] !== null)
    .reduce((sum, other) => sum + (before(other) ? LIMITS[other].min : shown[other]!), 0);
  return Math.max(min, Math.min(max, windowWidth - EDITOR_MIN - others));
}

/**
 * Where a drag puts a panel: the width it started at plus how far the edge moved (outwards,
 * so wider), kept within its limits. Dragged on past its minimum by more than half the minimum,
 * the panel is hidden instead; the Reading panel stays at its minimum (it's only ever closed).
 */
export function dragTo(panel: PanelName, start: number, moved: number, max: number): number | "hidden" {
  const { min } = LIMITS[panel];
  const wanted = start + moved;
  if (wanted < min / 2 && panel !== "reading") return "hidden";
  return clamp(Math.round(wanted), min, max);
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** One panel, as the handles need it. */
export type PanelHooks = {
  element: HTMLElement;
  handle: HTMLElement;
  /** The panel is shown (on the wide layout). */
  shown: () => boolean;
  /** Shows or hides it, exactly as its button and key do. */
  setShown: (shown: boolean) => void;
};

/** The handles, and the panels' widths. */
export class PanelSizes {
  /** The widths remembered on this Device, null for the default. */
  private readonly remembered: Record<PanelName, number | null> = { vault: null, reading: null, side: null };
  /** The panel being dragged, and how the drag began. */
  private drag: {
    panel: PanelName;
    pointer: number;
    x: number;
    start: number;
    before: number | null;
    max: number;
    moved: boolean;
  } | null = null;

  constructor(private readonly panels: Record<PanelName, PanelHooks>) {
    for (const name of PANELS) {
      try {
        this.remembered[name] = readWidth(name, (key) => localStorage.getItem(key));
      } catch {
        // nothing remembered: the default
      }
      this.listen(name);
    }
    // Shown or hidden by anything (a button, a key, the phone layout): lay out again.
    const again = new MutationObserver(() => this.layout());
    again.observe(document.body, { attributes: true, attributeFilter: ["class"] });
    for (const { element } of Object.values(panels)) {
      again.observe(element, { attributes: true, attributeFilter: ["hidden"] });
      again.observe(element.parentElement!, { attributes: true, attributeFilter: ["class"] });
    }
    addEventListener("resize", () => this.layout());
    this.layout();
  }

  private get phone(): boolean {
    return document.body.classList.contains("phone");
  }

  private get windowWidth(): number {
    return document.documentElement.clientWidth;
  }

  /** The widths as shown now (null for a hidden panel). */
  private shownWidths(): Widths {
    const want = (name: PanelName) =>
      this.panels[name].shown() ? (this.remembered[name] ?? defaultWidth(name, this.windowWidth)) : null;
    return fit(this.windowWidth, { vault: want("vault"), reading: want("reading"), side: want("side") });
  }

  /** Puts each panel at its width, and each handle on its edge (or away, with its panel). */
  layout() {
    const widths: Widths = this.phone ? { vault: null, reading: null, side: null } : this.shownWidths();
    for (const name of PANELS) {
      const { element, handle } = this.panels[name];
      const width = widths[name];
      // The phone layout's own CSS sizes the panels there.
      element.style.width = this.phone || width === null ? "" : `${width}px`;
      // While dragging, the handle stays even if its panel is hidden, so it can be dragged back.
      handle.hidden = this.phone || (width === null && this.drag?.panel !== name);
      handle.setAttribute("aria-valuemin", String(LIMITS[name].min));
      handle.setAttribute("aria-valuemax", String(maxWidth(name, this.windowWidth, widths)));
      if (width !== null) handle.setAttribute("aria-valuenow", String(width));
    }
  }

  /** Remembers a width (null: back to the default) and lays out again. */
  private setWidth(name: PanelName, width: number | null) {
    this.remembered[name] = width;
    try {
      if (width === null) localStorage.removeItem(STORAGE_KEYS[name]);
      else localStorage.setItem(STORAGE_KEYS[name], String(width));
    } catch {
      // not remembered, then: no harm done
    }
    this.layout();
  }

  /** The widest this panel can go now, with the others as shown. */
  private maxNow(name: PanelName): number {
    return maxWidth(name, this.windowWidth, this.shownWidths());
  }

  private listen(name: PanelName) {
    const { handle } = this.panels[name];
    // The Vault browser's edge is on its right, the Reading panel's and the Side panel's on
    // their left: moving the edge away from the panel's own side of the window widens it.
    const outwards = name === "vault" ? 1 : -1;

    handle.addEventListener("pointerdown", (e) => {
      if (e.button !== 0 || this.drag) return;
      e.preventDefault();
      const shown = this.shownWidths()[name];
      if (shown === null) return;
      handle.setPointerCapture(e.pointerId);
      this.drag = {
        panel: name,
        pointer: e.pointerId,
        x: e.clientX,
        start: shown,
        before: this.remembered[name],
        max: this.maxNow(name),
        moved: false,
      };
      handle.classList.add("dragging");
      document.body.classList.add("resizing");
    });

    handle.addEventListener("pointermove", (e) => {
      const drag = this.drag;
      if (!drag || drag.panel !== name || e.pointerId !== drag.pointer) return;
      const moved = (e.clientX - drag.x) * outwards;
      if (!drag.moved && moved === 0) return;
      drag.moved = true;
      const to = dragTo(name, drag.start, moved, drag.max);
      const { setShown, shown } = this.panels[name];
      if (to === "hidden") {
        if (shown()) setShown(false);
        return;
      }
      // Only in memory while dragging: kept when the drag ends with the panel shown.
      this.remembered[name] = to;
      if (!shown()) setShown(true);
      else this.layout();
    });

    const end = (e: PointerEvent) => {
      const drag = this.drag;
      if (!drag || drag.panel !== name || e.pointerId !== drag.pointer) return;
      this.drag = null;
      handle.classList.remove("dragging");
      document.body.classList.remove("resizing");
      if (handle.hasPointerCapture(e.pointerId)) handle.releasePointerCapture(e.pointerId);
      if (!drag.moved) return;
      // Hidden by the drag: it comes back at the width it had before.
      if (this.panels[name].shown()) this.setWidth(name, this.remembered[name]);
      else this.setWidth(name, drag.before);
    };
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
    handle.addEventListener("lostpointercapture", end);

    handle.addEventListener("dblclick", () => this.setWidth(name, null));

    // The handle is a separator: arrows move it, Home and End go to the limits, Enter resets.
    handle.addEventListener("keydown", (e) => {
      const shown = this.shownWidths()[name];
      if (shown === null || e.ctrlKey || e.altKey || e.metaKey) return;
      const step = e.shiftKey ? BIG_STEP : STEP;
      const max = this.maxNow(name);
      let to: number | null;
      if (e.key === "ArrowRight") to = shown + step * outwards;
      else if (e.key === "ArrowLeft") to = shown - step * outwards;
      else if (e.key === "Home") to = LIMITS[name].min;
      else if (e.key === "End") to = max;
      else if (e.key === "Enter") to = null;
      else return;
      e.preventDefault();
      this.setWidth(name, to === null ? null : clamp(to, LIMITS[name].min, max));
    });
  }
}
