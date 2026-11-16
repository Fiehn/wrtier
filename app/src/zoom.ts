// Zooming the Page preview (v2 plan, "Zooming the Page preview"). Three modes: Fit width (each
// page as wide as the pages' column, the default), Fit page (each page whole in view) and a
// percentage of the page's real printed size, which stays put when the panel is resized. The
// zoom is Device state, one for every Work, and never changes the pages themselves.
//
// It zooms by the menu in the preview's header, Ctrl + mouse wheel or a pinch over the pages,
// and keys (main.ts) while the Page preview has the focus. Whatever changes the zoom or the
// panel's size keeps a spot in place: the one under the pointer or fingers, else the middle.

/** "width" and "page" fit the panel; a number is a fraction of the real size (1 = 100%). */
export type Zoom = "width" | "page" | number;

export const ZOOM_MIN = 0.25;
export const ZOOM_MAX = 4;
/** The menu's percentages, which the keys step through. */
export const LEVELS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4];
/** Page sizes are in points; CSS pixels are 1/96 inch. */
export const PX_PER_PT = 96 / 72;
/** One notch of a mouse wheel (deltaY 100) zooms by this much. */
const NOTCH = 1.1;

const STORAGE_KEY = "pages-zoom";

/** A zoom as stored ("width", "page", "1.25"); anything else is Fit width. */
export function readZoom(stored: string | null): Zoom {
  if (stored === "page") return "page";
  const n = Number(stored);
  return stored && Number.isFinite(n) && n > 0 ? clampZoom(n) : "width";
}

/** How a zoom is named in the menu. */
export function zoomLabel(zoom: Zoom): string {
  if (zoom === "width") return "Fit width";
  if (zoom === "page") return "Fit page";
  return `${Math.round(zoom * 100)}%`;
}

/** Within the limits. Not rounded: a pinch on a touchpad moves in steps far under 1%. */
export function clampZoom(zoom: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom));
}

/** The menu's own percentage this zoom is (to the nearest half percent), if any. */
export function levelOf(zoom: number): number | null {
  return LEVELS.find((l) => Math.abs(l - zoom) < 0.005) ?? null;
}

type Size = { width: number; height: number };

/**
 * CSS pixels per point for a page (in points) in the space the pages have (in pixels):
 * Fit width fills the width, Fit page the width or the height, whichever comes first.
 */
export function scaleOf(zoom: Zoom, page: Size, space: Size): number {
  const across = space.width / page.width;
  if (zoom === "width") return across;
  if (zoom === "page") return Math.min(across, space.height / page.height);
  return zoom * PX_PER_PT;
}

/** The next of the menu's percentages above (1) or below (-1) a zoom, kept within the ends. */
export function stepped(zoom: number, direction: 1 | -1): number {
  const near = 0.005; // 99.6% steps up to 125%, not to 100%
  if (direction === 1) return LEVELS.find((l) => l > zoom + near) ?? ZOOM_MAX;
  return [...LEVELS].reverse().find((l) => l < zoom - near) ?? ZOOM_MIN;
}

/** The zoom after a wheel turn (deltaY in pixels; a pinch on a touchpad gives small ones). */
export function wheeled(zoom: number, deltaY: number): number {
  return clampZoom(zoom * NOTCH ** (-deltaY / 100));
}

/** A page as the zoom needs it: its element, and its size in points. */
export type SizedPage = { el: HTMLElement; width: number; height: number };

/** A spot on a page, as fractions of it, and where it is in the pages' view (in pixels). */
type Anchor = { page: number; fx: number; fy: number; x: number; y: number };

/** The space between pages (the CSS's margin under each). */
const GAP = 14;

export class PageZoom {
  zoom: Zoom = "width";
  /** The spot in the middle of the view, kept up to date so a resize can put it back. */
  private middle: Anchor | null = null;
  private pinch: { distance: number; zoom: number } | null = null;

  constructor(
    private readonly list: HTMLElement,
    private readonly menu: HTMLSelectElement,
    private readonly pages: () => SizedPage[],
  ) {
    try {
      this.zoom = readZoom(localStorage.getItem(STORAGE_KEY));
    } catch {
      // nothing remembered: Fit width
    }
    this.showMenu();
    this.menu.addEventListener("change", () => this.set(readZoom(this.menu.value)));

    list.addEventListener("scroll", () => this.remember(), { passive: true });
    // The panel resized (dragged, the window, the phone turned): the fit modes follow, and the
    // spot that was in the middle stays there.
    new ResizeObserver(() => {
      if (this.list.clientHeight === 0) return; // hidden
      const middle = this.middle;
      this.layout();
      if (middle) this.restore(middle);
    }).observe(list);

    list.addEventListener("wheel", (e) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      const delta = e.deltaY * (e.deltaMode === 1 ? 33 : e.deltaMode === 2 ? 400 : 1);
      this.set(wheeled(this.current(), delta), this.pointIn(e.clientX, e.clientY));
    }, { passive: false });

    // Two fingers: zoom around the point between them.
    list.addEventListener("touchstart", (e) => {
      if (e.touches.length === 2) this.pinch = { distance: distance(e.touches), zoom: this.current() };
    }, { passive: true });
    list.addEventListener("touchmove", (e) => {
      if (!this.pinch || e.touches.length !== 2) return;
      e.preventDefault();
      const [a, b] = [e.touches[0], e.touches[1]];
      const zoom = clampZoom(this.pinch.zoom * (distance(e.touches) / this.pinch.distance));
      if (zoom !== this.zoom) this.set(zoom, this.pointIn((a.clientX + b.clientX) / 2, (a.clientY + b.clientY) / 2));
    }, { passive: false });
    const lift = (e: TouchEvent) => {
      if (e.touches.length < 2) this.pinch = null;
    };
    list.addEventListener("touchend", lift);
    list.addEventListener("touchcancel", lift);
  }

  /** Zooms, keeping the spot at `at` (in the view, in pixels; the middle if not given) in place. */
  set(zoom: Zoom, at?: { x: number; y: number }) {
    const point = at ?? { x: this.list.clientWidth / 2, y: this.list.clientHeight / 2 };
    const anchor = this.anchorAt(point.x, point.y);
    this.zoom = typeof zoom === "number" ? clampZoom(zoom) : zoom;
    try {
      localStorage.setItem(STORAGE_KEY, String(this.zoom));
    } catch {
      // not remembered, then: no harm done
    }
    this.showMenu();
    this.layout();
    if (anchor) this.restore(anchor);
    else this.remember();
  }

  /** One of the menu's percentages up. */
  zoomIn() {
    this.set(stepped(this.current(), 1));
  }

  /** One of the menu's percentages down. */
  zoomOut() {
    this.set(stepped(this.current(), -1));
  }

  /** Back to Fit width. */
  reset() {
    this.set("width");
  }

  /** Sizes every page for the zoom. Fit width is the CSS's own (100%), so it needs nothing. */
  layout() {
    const pages = this.pages();
    this.list.dataset.zoom = typeof this.zoom === "number" ? "percent" : this.zoom;
    if (this.zoom === "width") {
      for (const page of pages) page.el.style.width = "";
      return;
    }
    const space = this.space();
    for (const page of pages) page.el.style.width = `${page.width * scaleOf(this.zoom, page, space)}px`;
  }

  /** The pages were all taken away (another Work, or a Milestone): the next ones start at the top. */
  cleared() {
    this.middle = null;
  }

  /** The pages changed (a compile): size them, keeping the middle in place. */
  pagesChanged() {
    const middle = this.middle;
    this.layout();
    if (middle) this.restore(middle);
    else this.remember();
  }

  /** The zoom as a percentage of the real size: in a fit mode, what it works out to here. */
  current(): number {
    if (typeof this.zoom === "number") return this.zoom;
    const pages = this.pages();
    const page = pages[this.middle?.page ?? 0] ?? pages[0];
    if (!page || page.el.offsetWidth === 0) return 1;
    return clampZoom(page.el.offsetWidth / (page.width * PX_PER_PT));
  }

  /** The room the pages have: the view less its padding, and a gap under each page. */
  private space(): Size {
    const style = getComputedStyle(this.list);
    const px = (v: string) => parseFloat(v) || 0;
    return {
      width: Math.max(1, this.list.clientWidth - px(style.paddingLeft) - px(style.paddingRight)),
      height: Math.max(1, this.list.clientHeight - px(style.paddingTop) - GAP),
    };
  }

  /** The menu shows the zoom, with its percentage added when it isn't one of the menu's own. */
  private showMenu() {
    const option = (zoom: Zoom) => {
      const o = document.createElement("option");
      o.value = String(zoom);
      o.textContent = zoomLabel(zoom);
      return o;
    };
    const levels: number[] = [...LEVELS];
    let shown: Zoom = this.zoom;
    if (typeof this.zoom === "number") {
      shown = levelOf(this.zoom) ?? this.zoom;
      if (!LEVELS.includes(shown)) levels.push(shown);
      levels.sort((a, b) => a - b);
    }
    this.menu.replaceChildren(option("width"), option("page"), ...levels.map(option));
    this.menu.value = String(shown);
  }

  /** A point on screen, in the view's own pixels. */
  private pointIn(clientX: number, clientY: number): { x: number; y: number } {
    const box = this.list.getBoundingClientRect();
    return { x: clientX - box.left - this.list.clientLeft, y: clientY - box.top - this.list.clientTop };
  }

  /** Which page is at a point in the view, and where on it. */
  private anchorAt(x: number, y: number): Anchor | null {
    const pages = this.pages();
    if (pages.length === 0 || this.list.clientHeight === 0) return null;
    const top = this.list.scrollTop + y;
    const left = this.list.scrollLeft + x;
    // The last page starting above the point (the pages are in order down the view).
    let lo = 0;
    let hi = pages.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (pages[mid].el.offsetTop <= top) lo = mid;
      else hi = mid - 1;
    }
    const el = pages[lo].el;
    if (el.offsetHeight === 0) return null;
    return { page: lo, fx: (left - el.offsetLeft) / el.offsetWidth, fy: (top - el.offsetTop) / el.offsetHeight, x, y };
  }

  /** Scrolls so the anchor's spot is back where it was in the view. */
  private restore(anchor: Anchor) {
    const el = this.pages()[anchor.page]?.el;
    if (el) {
      this.list.scrollTop = el.offsetTop + anchor.fy * el.offsetHeight - anchor.y;
      this.list.scrollLeft = el.offsetLeft + anchor.fx * el.offsetWidth - anchor.x;
    }
    this.remember();
  }

  private remember() {
    if (this.list.clientHeight === 0) return;
    this.middle = this.anchorAt(this.list.clientWidth / 2, this.list.clientHeight / 2) ?? this.middle;
  }
}

function distance(touches: TouchList): number {
  const [a, b] = [touches[0], touches[1]];
  return Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY) || 1;
}
