// Writes the Dictionaries the app offers (the Spellchecker, ADR 0019): src-tauri/dictionaries/
// catalogue.json, every Hunspell dictionary in wooorm/dictionaries at one pinned commit, with its
// name, licence, sizes and the SHA-256 of each file, so a download is checked before it's kept;
// and the two that ship inside the app (British and American English) next to it.
//
// Run by hand when moving to a newer commit (`npm run dictionaries`): it downloads every
// dictionary once (about 300 MB) to hash it. The app never asks GitHub what's there.

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

/** The commit every App version's list is pinned to. */
const COMMIT = "8cfea406b505e4d7df52d5a19bce525df98c54ab";
const REPO = "wooorm/dictionaries";
/** Ship inside the app: our code, theirs. */
const SHIPPED = { "en-GB": "en-GB", "en-US": "en" };

const app = path.resolve(import.meta.dirname, "..");
const out = path.join(app, "src-tauri", "dictionaries");
const cache = path.join(os.tmpdir(), "wrtier-dictionaries", COMMIT);

const raw = (file) => `https://raw.githubusercontent.com/${REPO}/${COMMIT}/${file}`;

async function get(url) {
  for (let attempt = 1; ; attempt++) {
    const res = await fetch(url);
    if (res.ok) return Buffer.from(await res.arrayBuffer());
    if (attempt === 3) throw new Error(`${url}: ${res.status}`);
  }
}

/** A file of a dictionary, from the cache or downloaded into it. */
async function file(theirs, name) {
  const at = path.join(cache, theirs, name);
  if (!fs.existsSync(at)) {
    fs.mkdirSync(path.dirname(at), { recursive: true });
    fs.writeFileSync(at, await get(raw(`dictionaries/${theirs}/${name}`)));
  }
  return fs.readFileSync(at);
}

const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");

/** Their code as ours: the language, then the region in capitals. Their "en" is American English. */
const ours = (theirs) => (theirs === "en" ? "en-US" : theirs);

const tree = JSON.parse(await get(`https://api.github.com/repos/${REPO}/git/trees/${COMMIT}?recursive=1`));
const names = [...new Set(tree.tree.map((e) => /^dictionaries\/([^/]+)\/index\.dic$/.exec(e.path)?.[1]).filter(Boolean))].sort();

const dictionaries = [];
for (const theirs of names) {
  const pkg = JSON.parse(await file(theirs, "package.json"));
  const aff = await file(theirs, "index.aff");
  const dic = await file(theirs, "index.dic");
  dictionaries.push({
    code: ours(theirs),
    source: theirs,
    // Their "en" is American English, called just "English".
    name: theirs === "en" ? "English (United States)" : pkg.description.replace(/ spelling dictionary$/i, "").trim(),
    license: pkg.license,
    size: aff.length + dic.length,
    aff: sha256(aff),
    dic: sha256(dic),
  });
  process.stdout.write(`${theirs} `);
}
process.stdout.write("\n");

fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(out, "catalogue.json"), JSON.stringify({ repo: REPO, commit: COMMIT, dictionaries }, null, 1) + "\n");
for (const [code, theirs] of Object.entries(SHIPPED)) {
  const dir = path.join(out, code);
  fs.mkdirSync(dir, { recursive: true });
  for (const name of ["index.aff", "index.dic", "license"]) fs.writeFileSync(path.join(dir, name), await file(theirs, name));
}
console.log(`${dictionaries.length} dictionaries at ${COMMIT.slice(0, 7)}; shipped: ${Object.keys(SHIPPED).join(", ")}.`);
