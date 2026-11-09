// The WebKit tests (v1 plan, Phase 6): the real interface in Playwright's WebKit, with the Rust
// side replaced by e2e/fake-tauri.js. WebKitGTK on Debian is the engine most likely to differ;
// Windows' WebView2 is close to Chromium, and the Vitest tests cover the interface's logic.
// Run with `npm run e2e`.

import { defineConfig, devices } from "@playwright/test";

const PORT = 1499;

export default defineConfig({
  testDir: "e2e",
  testMatch: "*.e2e.ts", // not *.test.ts, which Vitest runs
  fullyParallel: true,
  reporter: "list",
  use: {
    baseURL: `http://localhost:${PORT}`,
    ...devices["Desktop Safari"],
    viewport: { width: 1280, height: 800 },
  },
  projects: [{ name: "webkit", use: { browserName: "webkit" } }],
  webServer: {
    command: `npx vite --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: true,
  },
});
