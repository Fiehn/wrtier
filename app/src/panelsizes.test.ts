import { describe, expect, it } from "vitest";
import { EDITOR_MIN, SIDE_MIN, SIDE_MIN_FRAMED, defaultWidth, dragTo, fit, maxWidth, readWidth, setFramed, type Widths } from "./panelsizes";

const stored = (values: Record<string, string>) => (key: string) => values[key] ?? null;

/** The Vault browser and the Side panel, the Reading panel closed. */
const two = (vault: number | null, side: number | null): Widths => ({ vault, reading: null, side });

describe("the default widths", () => {
  it("are what the layout always had", () => {
    expect(defaultWidth("vault", 1280)).toBe(280);
    expect(defaultWidth("side", 1280)).toBeCloseTo(537.6);
    expect(defaultWidth("side", 2560)).toBe(640);
  });

  it("never put the Side panel under its minimum", () => {
    expect(defaultWidth("side", 500)).toBe(SIDE_MIN);
  });

  it("give the Reading panel about a third of the window", () => {
    expect(defaultWidth("reading", 1920)).toBe(600);
    expect(defaultWidth("reading", 1280)).toBeCloseTo(435.2);
    expect(defaultWidth("reading", 600)).toBe(260);
  });
});

describe("what a Device remembers", () => {
  it("is nothing at first: the defaults", () => {
    expect(readWidth("vault", stored({}))).toBeNull();
    expect(readWidth("side", stored({}))).toBeNull();
    expect(readWidth("reading", stored({}))).toBeNull();
  });

  it("is a width per panel", () => {
    expect(readWidth("vault", stored({ "sidebar-width": "320" }))).toBe(320);
    expect(readWidth("side", stored({ "panel-width": "900" }))).toBe(900);
    expect(readWidth("reading", stored({ "reading-width": "500" }))).toBe(500);
  });

  it("keeps a width within the panel's limits, and ignores nonsense", () => {
    expect(readWidth("vault", stored({ "sidebar-width": "2000" }))).toBe(480);
    expect(readWidth("vault", stored({ "sidebar-width": "20" }))).toBe(180);
    expect(readWidth("side", stored({ "panel-width": "wide" }))).toBeNull();
    expect(readWidth("side", stored({ "panel-width": "-5" }))).toBeNull();
    expect(readWidth("reading", stored({ "reading-width": "100" }))).toBe(260);
  });
});

describe("fitting the panels into the window", () => {
  it("shows the widths wanted when the editor keeps its room", () => {
    expect(fit(1920, two(300, 900))).toEqual(two(300, 900));
    expect(fit(1280, two(null, null))).toEqual(two(null, null));
    expect(fit(2560, { vault: 280, reading: 600, side: 640 })).toEqual({ vault: 280, reading: 600, side: 640 });
  });

  it("narrows the Side panel first, down to its minimum", () => {
    // A 1100px Side panel, and the window snapped to about half a big screen.
    expect(fit(1000, two(280, 1100))).toEqual(two(280, 1000 - EDITOR_MIN - 280));
    expect(fit(960, two(280, 1100)).side).toBe(SIDE_MIN);
  });

  it("then the Vault browser, down to its minimum", () => {
    expect(fit(900, two(300, 600))).toEqual(two(900 - EDITOR_MIN - SIDE_MIN, SIDE_MIN));
  });

  it("then lets the editor get narrower, hiding nothing", () => {
    expect(fit(760, two(300, 600))).toEqual(two(180, SIDE_MIN));
  });

  it("gives a hidden panel's room to the others", () => {
    expect(fit(960, two(null, 1100))).toEqual(two(null, 560));
    expect(fit(700, two(480, null))).toEqual(two(300, null));
  });

  it("narrows the Reading panel after the Side panel and before the Vault browser", () => {
    // 1600px: 280 + 600 + 640 wanted, 1200 there for them.
    expect(fit(1600, { vault: 280, reading: 600, side: 640 })).toEqual({ vault: 280, reading: 600, side: 320 });
    // 1400px: 1000 there: the Side panel at its minimum, then the Reading panel gives way.
    expect(fit(1400, { vault: 280, reading: 600, side: 640 })).toEqual({ vault: 280, reading: 1000 - 280 - SIDE_MIN, side: SIDE_MIN });
    // 1100px: 700 there: both at their minimum, then the Vault browser.
    expect(fit(1100, { vault: 280, reading: 600, side: 640 })).toEqual({ vault: 180, reading: 260, side: SIDE_MIN });
  });
});

describe("the widest a panel can be dragged", () => {
  it("lets the Side panel grow until the editor is at its minimum", () => {
    expect(maxWidth("side", 1920, two(280, 500))).toBe(1920 - EDITOR_MIN - 280);
    expect(maxWidth("side", 1920, two(null, 500))).toBe(1920 - EDITOR_MIN);
  });

  it("caps the Vault browser, leaving the Side panel its minimum", () => {
    expect(maxWidth("vault", 1920, two(300, 900))).toBe(480);
    expect(maxWidth("vault", 1000, two(300, 600))).toBe(1000 - EDITOR_MIN - SIDE_MIN);
  });

  it("is never under the panel's minimum", () => {
    expect(maxWidth("side", 760, two(300, 300))).toBe(SIDE_MIN);
    expect(maxWidth("vault", 760, two(300, 300))).toBe(180);
  });

  it("lets the Reading panel take the Side panel's room down to its minimum, but not the Vault browser's", () => {
    const shown = { vault: 280, reading: 500, side: 600 };
    expect(maxWidth("reading", 1920, shown)).toBe(1920 - EDITOR_MIN - 280 - SIDE_MIN);
    // The Side panel gives way first, so it can't take the Reading panel's room.
    expect(maxWidth("side", 1920, shown)).toBe(1920 - EDITOR_MIN - 280 - 500);
  });
});

describe("dragging", () => {
  it("follows the edge within the limits", () => {
    expect(dragTo("vault", 280, 60, 480)).toBe(340);
    expect(dragTo("vault", 280, 400, 480)).toBe(480);
    expect(dragTo("side", 500, -100, 1000)).toBe(400);
    expect(dragTo("reading", 400, 80, 1000)).toBe(480);
  });

  it("stops at the minimum", () => {
    expect(dragTo("vault", 280, -150, 480)).toBe(180);
    expect(dragTo("side", 500, -300, 1000)).toBe(SIDE_MIN);
  });

  it("hides the panel when dragged on past the minimum by half of it", () => {
    expect(dragTo("vault", 280, -189, 480)).toBe(180); // at 91px: still there, at its minimum
    expect(dragTo("vault", 280, -191, 480)).toBe("hidden"); // under 90px
    expect(dragTo("side", 500, -371, 1000)).toBe("hidden"); // under 145px
  });

  it("never hides the Reading panel: it's only closed", () => {
    expect(dragTo("reading", 400, -390, 1000)).toBe(260);
  });
});

describe("the Side panel's narrowest", () => {
  it("leaves room for the window's buttons beside its tabs with the app's own frame", () => {
    setFramed(true);
    try {
      expect(dragTo("side", 600, -300, 1000)).toBe(SIDE_MIN_FRAMED); // 300px wanted: held at 440
      expect(defaultWidth("side", 900)).toBe(SIDE_MIN_FRAMED);
    } finally {
      setFramed(false);
    }
    expect(dragTo("side", 600, -300, 1000)).toBe(300); // without it, 300px is wide enough
  });
});
