// Sets the app's version everywhere it's written, commits that as "v<version>", tags it, and
// pushes the branch and the tag to GitHub, where the tag starts the release build
// (.github/workflows/release.yml), which ends in a draft release to check and publish.
//
//   npm run release -- 0.1.4             asks before committing and pushing
//   npm run release -- 0.1.4-rc1         a pre-release: the files say 0.1.4, the tag v0.1.4-rc1
//   npm run release -- 0.1.4 --dry-run   shows what it would change, and changes nothing
//   npm run release -- 0.1.4 --no-push   commits and tags here, pushes nothing
//   npm run release -- 0.1.4 --yes       doesn't ask
//
// It refuses to run with uncommitted changes (the commit must be only the version), with a
// version that isn't newer than the current one, or with a tag that already exists here or on
// GitHub. Going from a pre-release to its release (0.1.4-rc1 to 0.1.4) changes no files: it
// only tags the current commit.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createInterface } from "node:readline/promises";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

/**
 * Every place the version is written: a pattern that matches there exactly once and ends with
 * the version (its one group) and its closing quote.
 */
const PLACES = [
  { file: "Cargo.toml", pattern: /^\[package\]\nname = "wrtier"\nversion = "([^"]+)"/m },
  { file: "app/src-tauri/Cargo.toml", pattern: /^\[package\]\nname = "wrtier_app"\nversion = "([^"]+)"/m },
  { file: "Cargo.lock", pattern: /^name = "wrtier"\nversion = "([^"]+)"/m },
  { file: "Cargo.lock", pattern: /^name = "wrtier_app"\nversion = "([^"]+)"/m },
  { file: "app/package.json", pattern: /^ {2}"version": "([^"]+)"/m },
  // The lock file says it twice: at the top, and for the app itself under "packages" (where
  // every dependency has a "version" too, so it's found by the app's name just above it).
  { file: "app/package-lock.json", pattern: /^ {2}"version": "([^"]+)"/m },
  { file: "app/package-lock.json", pattern: /^ {4}"": \{\n {6}"name": "wrtier-app",\n {6}"version": "([^"]+)"/m },
  { file: "app/src-tauri/tauri.conf.json", pattern: /^ {2}"version": "([^"]+)"/m },
];

function fail(message) {
  console.error(`\n${message}`);
  process.exit(1);
}

function git(args, options = {}) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: options.quiet ? ["ignore", "pipe", "pipe"] : ["ignore", "pipe", "inherit"] }).trim();
}

/** "1.2.3" as numbers, to compare; null if it isn't one. */
function parts(version) {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(version);
  return m ? m.slice(1).map(Number) : null;
}

function compare(a, b) {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

// ---- What's asked for ----

const args = process.argv.slice(2);
const flags = new Set(args.filter((a) => a.startsWith("--")));
const unknown = [...flags].filter((f) => !["--dry-run", "--no-push", "--yes"].includes(f));
if (unknown.length > 0) fail(`Unknown option ${unknown.join(", ")}. The options are --dry-run, --no-push and --yes.`);
const wanted = args.find((a) => !a.startsWith("--"));
if (!wanted) fail("Which version? For example: npm run release -- 0.1.4");
const m = /^v?(\d+\.\d+\.\d+)(-[0-9A-Za-z.-]+)?$/.exec(wanted);
if (!m) fail(`"${wanted}" isn't a version. Write it as 0.1.4, or 0.1.4-rc1 for a pre-release.`);
const version = m[1];
const tag = `v${version}${m[2] ?? ""}`;
const dryRun = flags.has("--dry-run");
const push = !flags.has("--no-push");

// ---- Where things stand ----

const texts = new Map();
const read = (file) => {
  if (!texts.has(file)) texts.set(file, readFileSync(join(root, file), "utf8").replace(/\r\n/g, "\n"));
  return texts.get(file);
};

/** Finds a place's version (and where it is), checking it's there exactly once. */
function locate(place) {
  const all = [...read(place.file).matchAll(new RegExp(place.pattern.source, "gm"))];
  if (all.length !== 1) fail(`Couldn't find the version in ${place.file} (its layout changed?). Nothing was changed.`);
  const [found] = all;
  const version = found[1];
  // The version ends just before the match's closing quote.
  return { at: found.index + found[0].length - 1 - version.length, version };
}

const current = PLACES.map((place) => ({ place, ...locate(place) }));
const versions = new Set(current.map((c) => c.version));
if (versions.size !== 1) {
  fail(`The files don't agree on the current version:\n${current.map((c) => `  ${c.place.file}: ${c.version}`).join("\n")}\nMake them agree first.`);
}
const old = [...versions][0];
const oldParts = parts(old);
const newParts = parts(version);
if (!oldParts) fail(`The current version, ${old}, isn't of the form 1.2.3.`);
const order = compare(newParts, oldParts);
if (order < 0) fail(`${version} is older than the current version, ${old}. The app only ever updates to a newer version.`);

// What would stop it: a dry run lists them and goes on to show the rest; a real run stops.
const refusals = [];
const refuse = (message) => (dryRun ? refusals.push(message) : fail(message));

let branch;
try {
  branch = git(["rev-parse", "--abbrev-ref", "HEAD"], { quiet: true });
} catch {
  fail("This isn't a git repository.");
}
if (branch === "HEAD") refuse("Not on a branch (a detached HEAD). Check out the branch to release from first.");
if (git(["status", "--porcelain"], { quiet: true }) !== "") {
  refuse("There are uncommitted changes. Commit or stash them first, so the release commit is only the version.");
}
if (git(["tag", "--list", tag], { quiet: true }) !== "") refuse(`The tag ${tag} already exists here.`);
if (push) {
  let remote = "";
  try {
    remote = git(["ls-remote", "--tags", "origin", `refs/tags/${tag}`], { quiet: true });
  } catch {
    refuse("Couldn't reach GitHub (origin) to check the tag. Are you online? (--no-push skips the check.)");
  }
  if (remote !== "") refuse(`The tag ${tag} already exists on GitHub.`);
}

// ---- What it'll do ----

const changesFiles = order > 0;
console.log(`Current version: ${old}`);
console.log(`New version:     ${version}${tag !== `v${version}` ? ` (tag ${tag}, a pre-release)` : ""}`);
if (changesFiles) {
  console.log("\nFiles:");
  for (const c of current) console.log(`  ${c.place.file}: ${c.version} → ${version}`);
} else {
  console.log(`\nThe files already say ${version}: no files change, the current commit is tagged.`);
}
console.log(`\nThen: ${changesFiles ? `commit "${tag}" on ${branch}, ` : ""}tag ${tag}${push ? `, and push ${branch} and ${tag} to origin` : " (nothing pushed: --no-push)"}.`);
if (push) console.log("GitHub then builds the release as a draft, to check and publish on its Releases page.");

if (dryRun) {
  for (const r of refusals) console.log(`\nWould refuse: ${r}`);
  console.log("\n--dry-run: nothing was changed.");
  process.exit(0);
}

if (!flags.has("--yes")) {
  const prompt = createInterface({ input: process.stdin, output: process.stdout });
  const answer = (await prompt.question("\nGo ahead? [y/N] ")).trim().toLowerCase();
  prompt.close();
  if (answer !== "y" && answer !== "yes") fail("Stopped: nothing was changed.");
}

// ---- Doing it ----

if (changesFiles) {
  // Each file's places from the end backwards, so earlier positions stay right.
  const byFile = new Map();
  for (const c of current) byFile.set(c.place.file, [...(byFile.get(c.place.file) ?? []), c]);
  for (const [file, places] of byFile) {
    let text = read(file);
    for (const c of places.sort((a, b) => b.at - a.at)) text = text.slice(0, c.at) + version + text.slice(c.at + c.version.length);
    // Kept with the line endings the file had.
    const crlf = readFileSync(join(root, file), "utf8").includes("\r\n");
    writeFileSync(join(root, file), crlf ? text.replace(/\n/g, "\r\n") : text);
  }
  git(["add", ...byFile.keys()]);
  git(["commit", "--quiet", "-m", tag]);
  console.log(`\nCommitted ${tag}.`);
}
git(["tag", tag]);
console.log(`Tagged ${tag}.`);

// The branch and the tag go in one push (one login, one connection), all or nothing: the tag is
// what starts the release, so it must never be left behind by a branch push that got stuck.
const pushBoth = `git push --atomic origin ${branch} ${tag}`;
if (push) {
  console.log(`\nPushing ${branch} and ${tag}…`);
  try {
    execFileSync("git", ["push", "--atomic", "origin", branch, tag], { cwd: root, stdio: "inherit" });
  } catch {
    fail(`Pushing failed (see above). The commit and tag are here, nothing reached GitHub; once it's sorted:\n  ${pushBoth}`);
  }
  // Checked, not assumed: the release only builds if GitHub has the tag.
  let there = "";
  try {
    there = git(["ls-remote", "--tags", "origin", `refs/tags/${tag}`], { quiet: true });
  } catch {
    // can't tell: said below
  }
  if (there === "") {
    fail(`git said it pushed, but GitHub doesn't show the tag ${tag} (yet?). Check https://github.com/Fiehn/Wrtier/tags, and if it's missing:\n  git push origin ${tag}`);
  }
  console.log(`\nGitHub has ${branch} and ${tag}. The release builds at https://github.com/Fiehn/Wrtier/actions`);
} else {
  console.log(`\nNothing pushed. When ready:\n  ${pushBoth}`);
}
