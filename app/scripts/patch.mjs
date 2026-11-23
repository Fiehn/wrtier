// Writes harper-core with the repository's patch applied (patches/, ADR 0023) to target/patch/,
// which Cargo needs before it can resolve anything in the workspace, `cargo about` included. Quick
// once the folder is there. Runs first in `npm run dev` and `npm run build`; after a fresh clone or
// `cargo clean`, run it (or `cargo patch-crate` in the repository's root) before `cargo build` or
// `cargo test` too. `tauri dev` builds the Rust side while `npm run dev` still runs, so on a fresh
// clone run this once before it.
//
// Needs patch-crate: cargo install patch-crate --locked

import { execSync } from "node:child_process";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..", "..");
try {
  execSync("cargo patch-crate", { cwd: root, stdio: ["ignore", "inherit", "inherit"] });
} catch {
  console.error("\nCouldn't patch Harper (ADR 0023). Is patch-crate installed?\n  cargo install patch-crate --locked\n");
  process.exit(1);
}
