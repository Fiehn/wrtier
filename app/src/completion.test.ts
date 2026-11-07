import { describe, expect, it } from "vitest";
import { toCompletion } from "./completion";

describe("completions", () => {
  it("puts Story elements first, in their own group", () => {
    const dream = toCompletion({ label: "dream", apply: "dream[${}]", detail: null, kind: "function", storyElement: true });
    const text = toCompletion({ label: "text", apply: "text(${})", detail: "Styles text.", kind: "function", storyElement: false });
    expect(dream.section).toMatchObject({ name: "Story elements", rank: 0 });
    expect(text.section).toMatchObject({ name: "Typst", rank: 1 });
    expect(dream.boost).toBeGreaterThan(text.boost ?? 0);
    expect(text.info).toBe("Styles text.");
  });

  it("types a template with the cursor in it, or plain text without fields", () => {
    expect(typeof toCompletion({ label: "dream", apply: "dream[${}]", detail: null, kind: "function", storyElement: true }).apply).toBe("function");
    expect(toCompletion({ label: "none", apply: "none", detail: null, kind: "constant", storyElement: false }).apply).toBe("none");
    // `#{` would be a field in CodeMirror's snippets, so such templates go in as plain text.
    expect(toCompletion({ label: "code", apply: "#{${}}", detail: null, kind: "syntax", storyElement: false }).apply).toBe("#{}");
  });
});
