import { describe, expect, it } from "vitest";
import { readPanelState } from "./sidepanel";

const stored = (values: Record<string, string>) => (key: string) => values[key] ?? null;

describe("what the panel remembers", () => {
  it("starts hidden, on the pages", () => {
    expect(readPanelState(stored({}))).toEqual({ shown: false, tab: "pages" });
  });

  it("keeps whether it was shown and which tab was open", () => {
    expect(readPanelState(stored({ panel: "open", "panel-tab": "comments" }))).toEqual({ shown: true, tab: "comments" });
    expect(readPanelState(stored({ panel: "closed", "panel-tab": "comments" }))).toEqual({ shown: false, tab: "comments" });
  });

  it("still reads whether the Page preview was open, from before the panel had tabs", () => {
    expect(readPanelState(stored({ preview: "open" }))).toEqual({ shown: true, tab: "pages" });
    expect(readPanelState(stored({ preview: "open", panel: "closed" }))).toEqual({ shown: false, tab: "pages" });
  });

  it("ignores a tab it doesn't know", () => {
    expect(readPanelState(stored({ panel: "open", "panel-tab": "history" }))).toEqual({ shown: true, tab: "pages" });
  });
});
