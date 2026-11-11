// Writes public/licenses.json: the license of every Rust crate and npm package the app ships
// with, and their NOTICE files (Typst's bundled fonts are covered by one). Shown in
// Settings → Licenses…. Runs before every `npm run build`, so a release always has it.
//
// Needs cargo-about: cargo install cargo-about --locked --features cli
// The licenses allowed are listed in src-tauri/about.toml; npm packages are checked against it too.

import { execSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const app = path.resolve(import.meta.dirname, "..");
const tauri = path.join(app, "src-tauri");
const out = path.join(app, "public", "licenses.json");

const accepted = new Set(
  [...fs.readFileSync(path.join(tauri, "about.toml"), "utf8").match(/accepted\s*=\s*\[([^\]]*)\]/)[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]),
);

/** Texts that differ only in spacing count as the same. */
const key = (text) => text.replace(/\s+/g, " ").trim();

/** Groups { license, by, text } entries with the same text into one, used by all of them. */
function group(entries) {
  const groups = new Map();
  for (const { license, by, text } of entries) {
    const k = key(text);
    const g = groups.get(k) ?? { license, usedBy: new Set(), text: text.trim() };
    g.usedBy.add(by);
    groups.set(k, g);
  }
  return [...groups.values()]
    .map((g) => ({ ...g, usedBy: [...g.usedBy].sort((a, b) => a.localeCompare(b)) }))
    .sort((a, b) => a.license.localeCompare(b.license) || a.usedBy[0].localeCompare(b.usedBy[0]));
}

/** The NOTICE files in a package's folder. */
function notices(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs
    .readdirSync(dir)
    .filter((f) => /^notice(\.(md|txt))?$/i.test(f))
    .map((f) => fs.readFileSync(path.join(dir, f), "utf8"));
}

// ---- Rust ----

// Written to a file, not read from its output: under PowerShell (GitHub's Windows machines),
// cargo-about refuses to have its output redirected.
let about;
const aboutFile = path.join(os.tmpdir(), `wrtier-licenses-${process.pid}.json`);
try {
  execSync(`cargo about generate --format json --locked --fail -o "${aboutFile}"`, { cwd: tauri, stdio: ["ignore", "inherit", "inherit"] });
  about = JSON.parse(fs.readFileSync(aboutFile, "utf8"));
} catch (e) {
  console.error("\nCouldn't list the Rust crates' licenses. Is cargo-about installed?\n  cargo install cargo-about --locked --features cli\n");
  process.exit(1);
} finally {
  fs.rmSync(aboutFile, { force: true });
}

const crate = (c) => `${c.name} ${c.version}`;
const rust = group(about.licenses.flatMap((l) => l.used_by.map((u) => ({ license: l.name, by: crate(u.crate), text: l.text }))));
const rustNotices = about.crates.flatMap(({ package: p }) =>
  notices(path.dirname(p.manifest_path)).map((text) => ({ license: "NOTICE", by: crate(p), text })),
);

// ---- npm (only what's bundled into the page, not the build tools) ----

const dirs = execSync("npm ls --omit=dev --all --parseable", { cwd: app }).toString().trim().split(/\r?\n/).slice(1);
const npmEntries = [];
const npmNotices = [];
for (const dir of [...new Set(dirs)]) {
  const p = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
  const by = `${p.name} ${p.version}`;
  const ids = String(p.license ?? "").match(/[A-Za-z0-9.+-]+/g)?.filter((t) => !["AND", "OR", "WITH"].includes(t)) ?? [];
  if (ids.length === 0 || !ids.some((id) => accepted.has(id))) {
    console.error(`\n${by} has the license "${p.license}", which isn't in src-tauri/about.toml's accepted list. Check it, then add it there.\n`);
    process.exit(1);
  }
  const files = fs.readdirSync(dir).filter((f) => /^(licen[cs]e|copying)/i.test(f));
  const text = files.length
    ? files.map((f) => fs.readFileSync(path.join(dir, f), "utf8")).join("\n\n")
    : `${p.name} is licensed under ${p.license}. The package doesn't include the license's text.`;
  npmEntries.push({ license: p.license, by, text });
  npmNotices.push(...notices(dir).map((text) => ({ license: "NOTICE", by, text })));
}

// ---- Write ----

const sections = [
  {
    title: "Notices",
    about: "Notices that come with some of the libraries below, including the licenses of the fonts Typst bundles for typesetting.",
    items: group([...rustNotices, ...npmNotices]),
  },
  { title: "Rust libraries", about: "Compiled into the app. Where a library offers a choice of licenses, the app uses it under the one shown.", items: rust },
  { title: "JavaScript libraries", about: "Part of the app's window: the editor and the bridge to the Rust side.", items: group(npmEntries) },
];
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, JSON.stringify({ sections }));
const count = (s) => new Set(s.items.flatMap((i) => i.usedBy)).size;
console.log(`Wrote ${path.relative(app, out)}: ${count(sections[1])} crates, ${count(sections[2])} npm packages, ${sections[0].items.length} notices.`);
