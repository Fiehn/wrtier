import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: vi.fn() }));
vi.mock("./android", () => ({ onAndroid: false }));

import { rightmost, windowTitle } from "./windowframe";

describe("the window's title", () => {
  it("names the tab in front, then the Vault, then the app", () => {
    expect(windowTitle("Rain", "Writing")).toBe("Rain – Writing – wrtier");
  });

  it("leaves out what isn't there", () => {
    expect(windowTitle(null, "Writing")).toBe("Writing – wrtier");
    expect(windowTitle(null, null)).toBe("wrtier");
    expect(windowTitle("", "Writing")).toBe("Writing – wrtier");
  });
});

describe("the row at the window's top-right corner", () => {
  it("is the shown row that reaches furthest right", () => {
    const tabRow = { right: 900, width: 600 };
    const panelTabs = { right: 1280, width: 380 };
    expect(rightmost([tabRow, panelTabs])).toBe(1);
  });

  it("skips hidden rows, which take no width", () => {
    const tabRow = { right: 1280, width: 980 };
    const hiddenPanel = { right: 0, width: 0 };
    expect(rightmost([tabRow, hiddenPanel])).toBe(0);
  });

  it("is none when no row is shown", () => {
    expect(rightmost([{ right: 0, width: 0 }])).toBe(-1);
    expect(rightmost([])).toBe(-1);
  });
});
