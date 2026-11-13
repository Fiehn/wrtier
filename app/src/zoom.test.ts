import { describe, expect, it } from "vitest";
import { LEVELS, PX_PER_PT, ZOOM_MAX, ZOOM_MIN, levelOf, readZoom, scaleOf, stepped, wheeled, zoomLabel } from "./zoom";

// An A5 page, in points.
const a5 = { width: 420, height: 595 };

describe("what a Device remembers", () => {
  it("is Fit width at first, and for anything it can't read", () => {
    expect(readZoom(null)).toBe("width");
    expect(readZoom("width")).toBe("width");
    expect(readZoom("huge")).toBe("width");
    expect(readZoom("-1")).toBe("width");
  });

  it("is Fit page or a percentage, kept within the limits", () => {
    expect(readZoom("page")).toBe("page");
    expect(readZoom("1.25")).toBe(1.25);
    expect(readZoom("1.3712")).toBe(1.3712);
    expect(readZoom("40")).toBe(ZOOM_MAX);
    expect(readZoom("0.01")).toBe(ZOOM_MIN);
  });
});

describe("the menu", () => {
  it("names each zoom", () => {
    expect(zoomLabel("width")).toBe("Fit width");
    expect(zoomLabel("page")).toBe("Fit page");
    expect(zoomLabel(1)).toBe("100%");
    expect(zoomLabel(1.3712)).toBe("137%");
  });

  it("knows a percentage of its own when a pinch lands next to it", () => {
    expect(levelOf(1.25)).toBe(1.25);
    expect(levelOf(0.9996)).toBe(1);
    expect(levelOf(1.37)).toBeNull();
  });
});

describe("how big a page is", () => {
  const space = { width: 500, height: 600 };

  it("fills the width at Fit width", () => {
    expect(a5.width * scaleOf("width", a5, space)).toBeCloseTo(500);
  });

  it("fits whole at Fit page, by its height or its width", () => {
    expect(a5.height * scaleOf("page", a5, space)).toBeCloseTo(600);
    expect(a5.width * scaleOf("page", a5, { width: 300, height: 2000 })).toBeCloseTo(300);
  });

  it("is its real size at 100%, whatever room there is", () => {
    expect(a5.width * scaleOf(1, a5, space)).toBeCloseTo(560); // 420pt is 560 CSS pixels
    expect(scaleOf(1, a5, { width: 200, height: 200 })).toBe(PX_PER_PT);
    expect(scaleOf(2, a5, space)).toBe(2 * PX_PER_PT);
  });
});

describe("stepping through the menu's percentages", () => {
  it("goes to the next one up or down", () => {
    expect(stepped(1, 1)).toBe(1.25);
    expect(stepped(1, -1)).toBe(0.75);
  });

  it("starts from where a fit mode or a pinch left it", () => {
    expect(stepped(0.92, 1)).toBe(1);
    expect(stepped(0.92, -1)).toBe(0.75);
    expect(stepped(1.37, 1)).toBe(1.5);
  });

  it("doesn't count a percentage it's already at, give or take", () => {
    expect(stepped(0.998, 1)).toBe(1.25);
    expect(stepped(1.002, -1)).toBe(0.75);
  });

  it("stops at the ends", () => {
    expect(stepped(LEVELS[LEVELS.length - 1], 1)).toBe(ZOOM_MAX);
    expect(stepped(0.5, -1)).toBe(ZOOM_MIN);
    expect(stepped(ZOOM_MIN, -1)).toBe(ZOOM_MIN);
  });
});

describe("the wheel", () => {
  it("zooms about 10% a notch", () => {
    expect(wheeled(1, -100)).toBeCloseTo(1.1);
    expect(wheeled(1, 100)).toBeCloseTo(1 / 1.1);
  });

  it("moves a little for a touchpad's small steps", () => {
    expect(wheeled(1, -2)).toBeGreaterThan(1);
    expect(wheeled(1, -2)).toBeLessThan(1.01);
  });

  it("stays within the limits", () => {
    expect(wheeled(3.9, -1000)).toBe(ZOOM_MAX);
    expect(wheeled(0.3, 1000)).toBe(ZOOM_MIN);
  });
});
