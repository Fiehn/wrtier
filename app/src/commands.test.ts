// The commands the interface sends, and those the WebKit tests' fake Rust side answers, must all
// be ones the app registers (lib.rs): a command renamed or removed on one side fails here, not
// only in the running app.

import { describe, expect, it } from "vitest";
import lib from "../src-tauri/src/lib.rs?raw";
import fake from "../e2e/fake-tauri.js?raw";

const sources = import.meta.glob(["./*.ts", "!./*.test.ts"], { query: "?raw", import: "default", eager: true });

/** The commands lib.rs registers with `generate_handler!`. */
function registered(): Set<string> {
  const list = lib.match(/generate_handler!\[([\s\S]*?)\]/);
  if (!list) throw new Error("lib.rs has no generate_handler!");
  return new Set(list[1].split(",").map((name) => name.replace(/\/\/.*$/gm, "").trim()).filter(Boolean));
}

/** The commands the interface's own code invokes by name. */
function invoked(): Map<string, string> {
  const found = new Map<string, string>();
  for (const [file, text] of Object.entries(sources)) {
    for (const match of text.matchAll(/invoke(?:<[^(]*?>)?\(\s*"([a-z_]+)"/g)) found.set(match[1], file);
  }
  return found;
}

/** The commands the fake answers (the keys of its `handlers`), besides Tauri's own plugins. */
function faked(): string[] {
  const table = fake.match(/const handlers = \{([\s\S]*?)\n {2}\};/);
  if (!table) throw new Error("fake-tauri.js has no handlers table");
  return [...table[1].matchAll(/^ {4}([a-z_]+):/gm)].map((match) => match[1]);
}

describe("commands", () => {
  it("are found", () => {
    expect(registered().size).toBeGreaterThan(100);
    expect(invoked().size).toBeGreaterThan(100);
    expect(faked().length).toBeGreaterThan(50);
  });

  it("the interface sends are all registered", () => {
    const known = registered();
    const unknown = [...invoked()].filter(([name]) => !known.has(name)).map(([name, file]) => `${name} (${file})`);
    expect(unknown).toEqual([]);
  });

  it("the fake Rust side answers are all registered", () => {
    const known = registered();
    expect(faked().filter((name) => !known.has(name))).toEqual([]);
  });
});
