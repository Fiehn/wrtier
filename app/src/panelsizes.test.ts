import { describe, expect, it } from "vitest";
import { EDITOR_MIN, defaultWidth, dragTo, fit, maxWidth, readWidth } from "./panelsizes";

const stored = (values: Record<string, string>) => (key: string) => values[key] ?? null;

describe("the default widths", () => {
  it("are what the layout always had", () => {
    expect(defaultWidth("vault", 1280)).toBe(280);
    expect(defaultWidth("side", 1280)).toBeCloseTo(537.6);
    expect(defaultWidth("side", 2560)).toBe(640);
  });

  it("never put the Side panel under its minimum", () => {
    expect(defaultWidth("side", 500)).toBe(260);
  });
});

describe("what a Device remembers", () => {
  it("is nothing at first: the defaults", () => {
    expect(readWidth("vault", stored({}))).toBeNull();
    expect(readWidth("side", stored({}))).toBeNull();
  });

  it("is a width per panel", () => {
    expect(readWidth("vault", stored({ "sidebar-width": "320" }))).toBe(320);
    expect(readWidth("side", stored({ "panel-width": "900" }))).toBe(900);
  });

  it("keeps a width within the panel's limits, and ignores nonsense", () => {
    expect(readWidth("vault", stored({ "sidebar-width": "2000" }))).toBe(480);
    expect(readWidth("vault", stored({ "sidebar-width": "20" }))).toBe(180);
    expect(readWidth("side", stored({ "panel-width": "wide" }))).toBeNull();
    expect(readWidth("side", stored({ "panel-width": "-5" }))).toBeNull();
  });
});

describe("fitting the panels into the window", () => {
  it("shows the widths wanted when the editor keeps its room", () => {
    expect(fit(1920, { vault: 300, side: 900 })).toEqual({ vault: 300, side: 900 });
    expect(fit(1280, { vault: null, side: null })).toEqual({ vault: null, side: null });
  });

  it("narrows the Side panel first, down to its minimum", () => {
    // A 1100px Side panel, and the window snapped to half a big screen.
    expect(fit(960, { vault: 280, side: 1100 })).toEqual({ vault: 280, side: 960 - EDITOR_MIN - 280 });
    expect(fit(960, { vault: 280, side: 1100 }).side).toBe(280);
  });

  it("then the Vault browser, down to its minimum", () => {
    expect(fit(900, { vault: 300, side: 600 })).toEqual({ vault: 240, side: 260 });
  });

  it("then lets the editor get narrower, hiding nothing", () => {
    expect(fit(760, { vault: 300, side: 600 })).toEqual({ vault: 180, side: 260 });
  });

  it("gives a hidden panel's room to the other one", () => {
    expect(fit(960, { vault: null, side: 1100 })).toEqual({ vault: null, side: 560 });
    expect(fit(700, { vault: 480, side: null })).toEqual({ vault: 300, side: null });
  });
});

describe("the widest a panel can be dragged", () => {
  it("lets the Side panel grow until the editor is at its minimum", () => {
    expect(maxWidth("side", 1920, 280)).toBe(1920 - EDITOR_MIN - 280);
    expect(maxWidth("side", 1920, null)).toBe(1920 - EDITOR_MIN);
  });

  it("caps the Vault browser, leaving the Side panel its minimum", () => {
    expect(maxWidth("vault", 1920, 900)).toBe(480);
    expect(maxWidth("vault", 1000, 600)).toBe(1000 - EDITOR_MIN - 260);
  });

  it("is never under the panel's minimum", () => {
    expect(maxWidth("side", 760, 300)).toBe(260);
    expect(maxWidth("vault", 760, 300)).toBe(180);
  });
});

describe("dragging", () => {
  it("follows the edge within the limits", () => {
    expect(dragTo("vault", 280, 60, 480)).toBe(340);
    expect(dragTo("vault", 280, 400, 480)).toBe(480);
    expect(dragTo("side", 500, -100, 1000)).toBe(400);
  });

  it("stops at the minimum", () => {
    expect(dragTo("vault", 280, -150, 480)).toBe(180);
    expect(dragTo("side", 500, -300, 1000)).toBe(260);
  });

  it("hides the panel when dragged on past the minimum by half of it", () => {
    expect(dragTo("vault", 280, -189, 480)).toBe(180); // at 91px: still there, at its minimum
    expect(dragTo("vault", 280, -191, 480)).toBe("hidden"); // under 90px
    expect(dragTo("side", 500, -371, 1000)).toBe("hidden"); // under 130px
  });
});
