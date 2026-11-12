// The WebKit tests: what can differ between web engines, in the real interface with the Rust
// side replaced by fake-tauri.js (one Work, "Rain", open at the start). See playwright.config.ts.

import { test as base, expect, type Locator, type Page } from "@playwright/test";
import path from "node:path";

type Op = { op: string; from?: number; to?: number; at?: number; text?: string; body?: string };

/** The page with the fake loaded and Rain open; any error in the page fails the test. */
const test = base.extend<{ app: Page }>({
  app: async ({ page }, use) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
    await page.goto("/");
    await expect(line(page, "She walked home.")).toBeVisible();
    await use(page);
    expect(errors, "errors in the page").toEqual([]);
    expect(await page.evaluate(() => (window as any).__fake.unknown), "commands the fake doesn't know").toEqual([]);
  },
});

const line = (page: Page, text: string): Locator => page.locator(".cm-line", { hasText: text }).first();
const ops = (page: Page): Promise<Op[]> =>
  page.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "edit").flatMap((e: any) => e.args.ops));
const textOf = (page: Page): Promise<string> => page.evaluate(() => (window as any).__fake.texts.Rain);

/** Puts the cursor at the end of a line. */
async function endOf(page: Page, text: string) {
  await line(page, text).click();
  await page.keyboard.press("End");
}

/** Selects a whole line's text. */
async function selectLine(page: Page, text: string) {
  await line(page, text).click();
  await page.keyboard.press("Home");
  await page.keyboard.press("Shift+End");
}

test("typing reaches Rust as typed", async ({ app }) => {
  await endOf(app, "The end.");
  await app.keyboard.type(" Really, the end.");
  await expect(line(app, "The end. Really, the end.")).toBeVisible();
  await expect.poll(() => textOf(app)).toContain("The end. Really, the end.\n");
  expect((await ops(app)).some((op) => op.op === "type")).toBe(true);
});

test("the Formatted view hides markup, and shows it while the cursor is inside", async ({ app }) => {
  await endOf(app, "Morning came slowly.");
  await app.keyboard.type(" It was _quiet_ there.");
  // The cursor is past the emphasis, so its marks are hidden already.
  const morning = line(app, "Morning came slowly.");
  await expect(morning).toHaveText("Morning came slowly. It was quiet there.");
  await expect(morning.locator(".cm-tp-emph")).toHaveText("quiet");
  await morning.locator(".cm-tp-emph").click(); // the cursor inside "quiet"
  await expect(morning).toHaveText("Morning came slowly. It was _quiet_ there.");
  await line(app, "She walked home.").click();
  await expect(morning).toHaveText("Morning came slowly. It was quiet there.");
  await expect.poll(() => textOf(app)).toContain("Morning came slowly. It was _quiet_ there.");
});

test("Ctrl+F finds in the whole Work, past the lines drawn, and Replace all is one op", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  // Far more text than the editor draws at once, with the rain at the very end.
  await page.addInitScript(() => {
    const fake = (window as any).__fake;
    fake.texts.Rain += "Filler line.\n\n".repeat(400) + "The *rain* had stopped at last.\n";
  });
  await page.goto("/");
  await expect(line(page, "She walked home.")).toBeVisible();
  await line(page, "She walked home.").click();
  await page.keyboard.press("Control+f");
  const bar = page.locator("#find-bar");
  await expect(bar).toBeVisible();
  await expect(page.locator("#find-input")).toBeFocused();
  // As it reads: the stars around "rain" are hidden, so "rain had" finds it.
  await page.keyboard.type("rain had");
  await expect(page.locator("#find-count")).toHaveText("2 of 2"); // the first after the cursor is the far one
  await expect(line(page, "had stopped at last.").locator(".cm-find-current")).toBeVisible();
  await page.keyboard.press("Control+h");
  await expect(page.locator("#replace-input")).toBeFocused();
  await page.keyboard.type("snow has");
  await page.locator("#replace-all").click();
  await expect(page.locator("#find-note")).toHaveText("Replaced 2.");
  await expect.poll(() => page.evaluate(() => (window as any).__fake.texts.Rain)).toContain("The *snow* has stopped at last.");
  const sent = await page.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "edit").flatMap((e: any) => e.args.ops));
  expect(sent.filter((op: Op) => op.op === "replace")).toHaveLength(1);
  await page.keyboard.press("Escape");
  await expect(bar).toBeHidden();
  expect(errors, "errors in the page").toEqual([]);
});

test("the Raw view shows every mark, coloured by Typst", async ({ app }) => {
  await endOf(app, "She walked home.");
  await app.keyboard.press("Control+e");
  await expect(app.locator(".cm-line .typ-heading", { hasText: "= Two" })).toBeVisible();
  await app.keyboard.press("Control+e"); // back to the Formatted view: no colouring there
  await expect(app.locator(".typ-heading")).toHaveCount(0);
});

test("dragging a selection is a Move", async ({ app }) => {
  await selectLine(app, "She walked home.");
  const from = await line(app, "She walked home.").boundingBox();
  const to = await line(app, "The end.").boundingBox();
  await app.mouse.move(from!.x + 20, from!.y + from!.height / 2);
  await app.mouse.down();
  await app.mouse.move(to!.x + 5, to!.y + to!.height / 2, { steps: 10 });
  await app.mouse.move(to!.x + 3, to!.y + to!.height / 2, { steps: 2 });
  await app.mouse.up();
  await expect.poll(async () => (await ops(app)).find((op) => op.op === "move")).toBeTruthy();
  const move = (await ops(app)).find((op) => op.op === "move")!;
  const before = "= One\n\nThe rain had stopped. The street was wet.\n\n";
  expect([move.from, move.to]).toEqual([before.length, before.length + "She walked home.".length]);
  expect((await ops(app)).some((op) => op.op === "type" || op.op === "paste"), "never typed or pasted").toBe(false);
  await expect.poll(() => textOf(app)).toContain("She walked home.The end.");
});

test("dragging a Chapter in the Chapter list moves it", async ({ app }) => {
  const chapters = app.locator("#chapters-list li[draggable=true]");
  await expect(chapters).toHaveCount(3);
  await chapters.filter({ hasText: "Three" }).dragTo(chapters.filter({ hasText: "One" }));
  await expect.poll(() => textOf(app)).toMatch(/^= Three\n/);
  const sent = await ops(app);
  expect(sent.some((op) => op.op === "move")).toBe(true);
  expect(sent.some((op) => op.op === "type" || op.op === "paste" || op.op === "delete"), "only Moves").toBe(false);
  await expect(chapters.first()).toContainText("Three");
});

test("right-click on a selection adds a Comment", async ({ app }) => {
  await selectLine(app, "Morning came slowly.");
  await line(app, "Morning came slowly.").click({ button: "right", position: { x: 30, y: 8 } });
  await app.getByRole("menuitem", { name: /Add Comment/ }).click();
  const box = app.locator("#comments-new textarea");
  await expect(box).toBeVisible();
  await box.fill("Too slow?");
  await app.locator("#comments-new").getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => (await ops(app)).find((op) => op.op === "comment")).toBeTruthy();
  const comment = (await ops(app)).find((op) => op.op === "comment")!;
  const start = (await textOf(app)).indexOf("Morning came slowly.");
  expect([comment.from, comment.to, comment.body]).toEqual([start, start + "Morning came slowly.".length, "Too slow?"]);
  await expect(app.locator(".cm-comment", { hasText: "Morning came slowly." })).toBeVisible();
});

test("the Comments panel adds a Work comment, listed first with no quote", async ({ app }) => {
  // One on the text first, so the Work comment has something to come before.
  await selectLine(app, "Morning came slowly.");
  await line(app, "Morning came slowly.").click({ button: "right", position: { x: 30, y: 8 } });
  await app.getByRole("menuitem", { name: /Add Comment/ }).click();
  await app.locator("#comments-new textarea").fill("Too slow?");
  await app.locator("#comments-new").getByRole("button", { name: "Save" }).click();
  await expect(app.locator(".comment-card")).toHaveCount(1);

  await app.getByRole("button", { name: "Comment on this Work" }).click();
  await expect(app.locator("#comments-new")).toContainText("New Comment on the whole Work");
  await expect(app.locator(".cm-comment-pending")).toHaveCount(0);
  await app.locator("#comments-new textarea").fill("The ending drags.");
  await app.locator("#comments-new").getByRole("button", { name: "Save" }).click();

  await expect.poll(async () => (await ops(app)).find((op) => op.op === "workComment")).toBeTruthy();
  const sent = (await ops(app)).find((op) => op.op === "workComment")!;
  expect([sent.sender, sent.body]).toEqual(["author", "The ending drags."]);
  await expect(app.locator("#comments-new")).toBeHidden();
  await expect(app.locator("#comments-list .comments-section-title")).toHaveText(["On the whole Work", "On the text"]);
  const cards = app.locator(".comment-card");
  await expect(cards).toHaveCount(2);
  await expect(cards.first()).toContainText("The ending drags.");
  await expect(cards.first().locator(".comment-quote")).toHaveCount(0);
  await expect(cards.nth(1).locator(".comment-quote")).toHaveText("Morning came slowly.");

  // Each kind folds under its heading, remembered on this Device.
  const onWork = app.locator(".comments-on-work-heading button");
  await onWork.click();
  await expect(onWork).toHaveAttribute("aria-expanded", "false");
  await expect(onWork).toContainText("1");
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText("Too slow?");
  expect(await app.evaluate(() => localStorage.getItem("comments-folded"))).toBe(JSON.stringify(["work"]));
  await app.locator(".comments-on-text-heading button").click();
  await expect(cards).toHaveCount(0);
  await expect(app.locator("#comments-list .comments-section-title")).toHaveText(["On the whole Work", "On the text"]);
  await onWork.click();
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText("The ending drags.");

  // Clicking a highlight unfolds the text's Comments to show it.
  await app.locator(".cm-comment", { hasText: "Morning came slowly." }).click();
  await expect(cards).toHaveCount(2);
  await expect(app.locator(".comment-card.focused")).toContainText("Too slow?");
  await app.locator(".comments-on-work-heading button").click();
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/work-comment.png` : undefined });
});

test("the sidebar and the right panel hide and show", async ({ app }) => {
  const sidebar = app.locator("#sidebar");
  const panel = app.locator("#side-panel");
  await expect(sidebar).toBeVisible();
  await app.keyboard.press("Control+\\");
  await expect(sidebar).toBeHidden();
  await app.keyboard.press("Control+\\");
  await expect(sidebar).toBeVisible();

  const panelShown = await panel.isVisible();
  await app.keyboard.press("Control+Shift+P");
  await expect(panel).toBeVisible({ visible: !panelShown });
  await app.keyboard.press("Control+Shift+P");
  await expect(panel).toBeVisible({ visible: panelShown });
});

test("a Vault theme wins over the built-in colours it starts from", async ({ page }) => {
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  await page.addInitScript(() => {
    const fake = (window as any).__fake;
    fake.themes["night.css"] = { name: "Night", dark: true, css: "/* Theme: Night, dark */\n:root { --bg: rgb(1, 2, 3); --text-width: 30em; }" };
    fake.settings.theme = "vault:night.css";
  });
  await page.goto("/");
  await expect(line(page, "She walked home.")).toBeVisible();
  const root = page.locator("html");
  await expect(root).toHaveAttribute("data-theme", "dark");
  // The built-in dark colours use a more specific selector; the layer lets the theme win anyway.
  await expect(page.locator("body")).toHaveCSS("background-color", "rgb(1, 2, 3)");
  // Colours the theme leaves alone come from the built-in dark theme.
  const text = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--text").trim());
  expect(text).toBe("#e8e4dc");
  const width = await page.locator(".cm-content").first().evaluate((el) => getComputedStyle(el).maxWidth);
  expect(width).toBe(`${30 * 17}px`);
});

test("Settings shows the App version, and an update found is saved for, downloaded and installed", async ({ app }) => {
  await app.evaluate(() => {
    (window as any).__fake.update = { kind: "available", current: "0.1.2", version: "0.2.0" };
  });
  await app.locator("#open-settings").click();
  await expect(app.locator("#app-version")).toHaveText("Wrtier 0.1.2");
  await app.locator("#check-updates").click();

  const dialog = app.locator("#update");
  await expect(app.locator("#update-message")).toHaveText("Version 0.2.0 is available (you have 0.1.2).");
  await expect(app.locator("#update-close")).toHaveText("Not now");
  await app.locator("#update-whats-new").click();
  await app.locator("#update-install").click();
  await expect(app.locator("#update-message")).toHaveText("Starting the new version…");
  await expect(dialog).toBeVisible();

  // Everything typed is saved before the download, and the version asked for is the one shown.
  const calls = await app.evaluate(() => (window as any).__fake.log.map((e: any) => e.cmd));
  const asked = await app.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd !== "edit" && e.cmd.includes("_")).map((e: any) => [e.cmd, e.args]));
  expect(asked).toContainEqual(["open_release_page", { version: "0.2.0" }]);
  expect(asked).toContainEqual(["install_update", { version: "0.2.0" }]);
  expect(calls.lastIndexOf("app_hidden")).toBeLessThan(calls.lastIndexOf("install_update"));
});

test("with no newer version, the dialog says so and offers nothing to install", async ({ app }) => {
  await app.locator("#open-settings").click();
  await app.locator("#check-updates").click();
  await expect(app.locator("#update-message")).toHaveText("You have the newest version (0.1.2).");
  await expect(app.locator("#update-install")).toBeHidden();
  await expect(app.locator("#update-whats-new-row")).toBeHidden();
  await app.locator("#update-close").click();
  await expect(app.locator("#update")).toBeHidden();
});

test("Uninstall names writing that would be lost, saves first, and only goes on when confirmed", async ({ app }) => {
  await app.evaluate(() => {
    (window as any).__fake.uninstallCheck = { problem: null, unsaved: [{ vault: "C:/Fake Vault", work: "Rain" }] };
  });
  await app.locator("#open-settings").click();
  await app.locator("#open-uninstall").click();
  const dialog = app.locator("#uninstall");
  await expect(app.locator("#uninstall-unsaved-list")).toHaveText("Rain (in C:/Fake Vault)");
  await expect(app.locator("#uninstall-go")).toHaveText("Uninstall anyway");

  // Cancel changes nothing.
  await app.locator("#uninstall-cancel").click();
  await expect(dialog).toBeHidden();
  const cmds = () => app.evaluate(() => (window as any).__fake.log.map((e: any) => e.cmd) as string[]);
  expect(await cmds()).not.toContain("uninstall");

  await app.locator("#open-uninstall").click();
  await app.locator("#uninstall-go").click();
  await expect.poll(cmds).toContain("uninstall");
  const log = await cmds();
  expect(log.lastIndexOf("app_hidden")).toBeLessThan(log.lastIndexOf("uninstall"));
});

test("Uninstall says why when the app can't delete itself, and offers no button", async ({ app }) => {
  await app.evaluate(() => {
    (window as any).__fake.uninstallCheck = { problem: "The app can't change its own folder.", unsaved: [] };
  });
  await app.locator("#open-settings").click();
  await app.locator("#open-uninstall").click();
  await expect(app.locator("#uninstall-error")).toHaveText("The app can't change its own folder.");
  await expect(app.locator("#uninstall-go")).toBeHidden();
  await expect(app.locator("#uninstall-unsaved")).toBeHidden();
});

test("a Sync problem from a newer version of the app offers Check for updates", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  await page.addInitScript(() => {
    (window as any).__fake.problems = [
      { path: "Future.db", reason: "made by a newer version of the app", newer: true },
    ];
  });
  await page.goto("/");
  const problems = page.locator("#problems");
  await expect(problems).toContainText("Future.db: made by a newer version of the app");
  await problems.getByRole("button", { name: "Check for updates…" }).click();
  await expect(page.locator("#update-message")).toHaveText("You have the newest version (0.1.2).");
  expect(errors, "errors in the page").toEqual([]);
});

test("on Android, the Vault needs All files access and is chosen in the app's folder browser", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // MainActivity.kt's bridge, without "All files access" at first.
  await page.addInitScript(() => {
    const w = window as any;
    w.__access = false;
    w.__settingsOpened = 0;
    w.WrtierAndroid = {
      hasAllFilesAccess: () => w.__access,
      openAllFilesAccessSettings: () => w.__settingsOpened++,
      openAppInfo: () => {},
      sharedStorage: () => "/storage/emulated/0",
    };
  });
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  await page.goto("/");

  // The last Vault waits for the access; the dialog opens the switch in Settings.
  const access = page.locator("#files-access");
  await expect(access).toBeVisible();
  await page.locator("#files-access-open").click();
  expect(await page.evaluate(() => (window as any).__settingsOpened)).toBe(1);
  // Coming back with the switch on carries on by itself.
  await page.evaluate(() => {
    (window as any).__access = true;
    window.dispatchEvent(new Event("focus"));
  });
  await expect(access).toBeHidden();
  await expect(line(page, "She walked home.")).toBeVisible();

  // The folder browser: shared storage, Syncthing folders and Vaults marked, into a Vault.
  await page.locator("#choose-vault").click();
  const folders = page.locator("#folders");
  await expect(page.locator("#folders-path")).toHaveText("Phone storage");
  await expect(folders.locator(".folder-row")).toHaveText(["Download", "SyncSyncthing folder"]);
  await folders.locator(".folder-row", { hasText: "Sync" }).click();
  await expect(page.locator("#folders-path")).toHaveText("Sync");
  await expect(folders.locator(".folder-row")).toHaveText(["↑ Up", "NovelsVault"]);
  await folders.locator(".folder-row", { hasText: "Novels" }).click();
  await expect(page.locator("#folders-use")).toHaveText("Open this Vault");
  await page.locator("#folders-use").click();
  await expect(folders).toBeHidden();
  const opened = await page.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "open_vault").map((e: any) => e.args.path));
  expect(opened).toEqual(["C:/Fake Vault", "/storage/emulated/0/Sync/Novels"]);
  expect(errors, "errors in the page").toEqual([]);
  expect(await page.evaluate(() => (window as any).__fake.unknown), "commands the fake doesn't know").toEqual([]);
});

test("a Chapter of another Work is included, read-only, and counted", async ({ app }) => {
  await endOf(app, "She walked home.");
  // Right-click the other Work in the Vault: include it in the Work in front, one Chapter.
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "Chapter: The Navidson Record" }).click();
  // The editor sent the character as a paste, then what it includes.
  await expect.poll(async () => (await ops(app)).find((op) => op.op === "include")).toMatchObject({ work: "Zampanò", chapter: 52 });
  expect(await textOf(app)).toContain("She walked home.\n\uFFFC\n");
  // It shows as a block with the included text, under a bar naming it.
  const block = app.locator(".cm-inclusion");
  await expect(block.locator(".cm-inclusion-label")).toHaveText("Zampanò: The Navidson Record");
  await expect(block.locator(".cm-inclusion-text")).toContainText("a quarter inch more inside than out");
  await expect(app.locator("#counts")).toContainText("(12 included)");
  // In the Chapter list it's a Chapter of its own, between One and Two, marked as included.
  await expect(app.locator("#chapters-list li.included")).toContainText("The Navidson Record");
  await expect(app.locator("#chapters-list li[data-index] .chapter-title")).toHaveText(["One", "The Navidson Record", "Two", "Three"]);
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/inclusion.png` : undefined });
  // Typing before it moves it along; it's still there and still one character.
  await endOf(app, "The rain had stopped.");
  await app.keyboard.type(" Still.");
  await expect(block.locator(".cm-inclusion-label")).toBeVisible();
  // Folded, only the bar shows.
  await block.locator(".cm-inclusion-fold").click();
  await expect(block.locator(".cm-inclusion-text")).toHaveCount(0);
  // "Go to" opens the Work it includes.
  await block.locator(".cm-inclusion-go").click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.log.some((e: any) => e.cmd === "open_work" && e.args.id === "Zampanò"))).toBe(true);
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/goto.png` : undefined });
});

test.describe("at phone width", () => {
  test.use({ viewport: { width: 412, height: 860 } });

  test("one screen at a time, with the bar at the bottom and the toolbar under the text", async ({ app }) => {
    const bar = app.locator("#phone-bar");
    const screen = (name: string) => bar.locator(`[data-screen="${name}"]`);
    // A Work was open last time: writing, with the toolbar; the other parts aren't on screen.
    await expect(screen("write")).toHaveAttribute("aria-current", "page");
    await expect(app.locator("#write-toolbar")).toBeVisible();
    await expect(app.locator("#sidebar")).toBeHidden();
    await expect(app.locator("#side-panel")).toBeHidden();

    // The toolbar does what the keys do, and the keyboard stays where it was.
    await selectLine(app, "She walked home.");
    await app.locator('#write-toolbar [data-command="strong"]').click();
    await expect.poll(() => textOf(app)).toContain("*She walked home.*");
    await expect(app.locator(".cm-content")).toBeFocused();

    // Comment: a new Comment on the selection, on the Comments screen.
    await selectLine(app, "Morning came slowly.");
    await app.locator('#write-toolbar [data-command="addComment"]').click();
    await expect(screen("comments")).toHaveAttribute("aria-current", "page");
    await expect(app.locator("#comments-new")).toBeVisible();

    // Works: the Vault; opening a Work goes back to writing.
    await screen("works").click();
    await expect(app.locator("#sidebar")).toBeVisible();
    await expect(app.locator("#app > main")).toBeHidden();
    await app.locator("#works").getByText("Rain").click();
    await expect(screen("write")).toHaveAttribute("aria-current", "page");

    // The Chapter list is a sheet; a Chapter goes there and closes it.
    await app.locator("#chapters-open").click();
    await expect(app.locator("#chapters-sheet")).toBeVisible();
    await app.locator("#chapters-list li", { hasText: "Three" }).click();
    await expect(app.locator("#chapters-sheet")).toBeHidden();

    // With the keyboard open (Android says so), the bar makes room for the writing.
    await app.evaluate(() => document.documentElement.classList.add("keyboard-open"));
    await expect(bar).toBeHidden();
    await expect(app.locator("#write-toolbar")).toBeVisible();
  });
});

test("an Export with an Inclusion that can't be shown asks first", async ({ app }) => {
  await app.evaluate(() => ((window as any).__fake.missingInclusions = ["Zampanò: Zampanò is in the trash."]));
  const exported = () => app.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "export_pdf").length);
  const exportRain = async () => {
    await app.locator("#works li.work-entry", { hasText: "Rain" }).click({ button: "right" });
    await app.locator("#menu button", { hasText: "Export PDF…" }).click();
  };
  await exportRain();
  await expect(app.locator("#ask-confirm-text")).toContainText("One Inclusion can't be shown: Zampanò: Zampanò is in the trash.");
  await app.locator("#ask-confirm button", { hasText: "Cancel" }).click();
  expect(await exported()).toBe(0);
  await exportRain();
  await app.locator("#ask-confirm-yes", { hasText: "Export anyway" }).click();
  await expect.poll(exported).toBe(1);
});

test("a Comment on included text is kept in the Work it comes from, and shown here", async ({ app }) => {
  await endOf(app, "She walked home.");
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "Chapter: The Navidson Record" }).click();
  const text = app.locator(".cm-inclusion-text p", { hasText: "quarter inch" });
  await expect(text).toBeVisible();
  // A double-click selects a word in it (the editor doesn't take the selection back).
  const where = (words: string) => text.evaluate((p, words) => {
    const node = p.firstChild!;
    const start = node.textContent!.indexOf(words);
    const range = document.createRange();
    range.setStart(node, start);
    range.setEnd(node, start + words.length);
    return JSON.parse(JSON.stringify(range.getBoundingClientRect()));
  }, words);
  const measured = await where("measured");
  await app.mouse.dblclick(measured.x + measured.width / 2, measured.y + measured.height / 2);
  await expect.poll(() => app.evaluate(() => getSelection()!.toString().trim())).toBe("measured");
  // Drag across "quarter inch" to select it, the way the Author would, then right-click it.
  const box = await where("quarter inch");
  const y = box.y + box.height / 2;
  await app.mouse.move(box.x + 1, y);
  await app.mouse.down();
  await app.mouse.move(box.x + box.width / 2, y, { steps: 5 });
  await app.mouse.move(box.x + box.width + 1, y, { steps: 5 });
  await app.mouse.up();
  await expect.poll(() => app.evaluate(() => getSelection()!.toString())).toBe("quarter inch");
  await app.mouse.click(box.x + box.width / 2, y, { button: "right" });
  await app.locator("#menu button", { hasText: "Add Comment" }).click({ timeout: 5000 });
  await expect(app.locator("#comments-new")).toContainText("kept in Zampanò");
  await app.locator("#comments-new textarea").fill("How much, exactly?");
  await app.locator("#comments-new button", { hasText: "Save" }).click();
  // It went to the included Work, at the selection in the shown text ("= The Navidson Record\n\n" is 23).
  const sent = await app.evaluate(() => (window as any).__fake.log.find((e: any) => e.cmd === "comment_on_inclusion")?.args);
  expect(sent).toMatchObject({ id: "Rain", from: 23 + "It measured a ".length, to: 23 + "It measured a quarter inch".length, body: "How much, exactly?" });
  // Shown on the Inclusion, and in the Comments panel under the Inclusion.
  await expect(app.locator(".cm-inclusion .cm-comment")).toHaveText("quarter inch");
  await expect(app.locator(".comments-included-heading .comments-section-title")).toHaveText("In Zampanò: The Navidson Record");
  const card = app.locator(".comment-card", { hasText: "How much, exactly?" });
  await expect(card).toBeVisible();
  await expect(app.locator("#counts")).toContainText("(12 included)");
  await expect(app.locator("#chapters-list li.included")).toBeVisible();
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/comment.png` : undefined });
  // A Reply from here goes to Zampanò too.
  await card.locator("button", { hasText: "Reply" }).click();
  await card.locator("textarea").fill("A quarter inch.");
  await card.locator("button", { hasText: "Reply" }).last().click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.log.find((e: any) => e.cmd === "comment_in")?.args.work)).toBe("Zampanò");
  await expect(card).toContainText("A quarter inch.");
});

test("a whole included Work shows its own Chapters a level down in the Chapter list", async ({ app }) => {
  await endOf(app, "The end.");
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "The whole Work" }).click();
  const entry = app.locator("#chapters-list li.included");
  await expect(entry.locator(".chapter-sub li .chapter-title")).toHaveText(["Exploration A", "The Navidson Record"]);
  await expect(entry.locator("> .chapter-title")).toHaveText("Zampanò");
  await expect(entry.locator("> .chapter-words")).toHaveText("15"); // its Chapters' 6 and 9: headings aren't counted
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/whole.png` : undefined });
});

test("a Chapter split in its own Work shows a note, and the new one can be included too", async ({ app }) => {
  // In Zampanò, "Exploration A" now has a new Chapter after it ("Interlude", at 52).
  await app.evaluate(() => ((window as any).__fake.splits = { "Zampanò": { after: 0, title: "Interlude", at: 52 } }));
  await endOf(app, "She walked home.");
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "Chapter: Exploration A" }).click();
  const note = app.locator(".cm-inclusion-split");
  await expect(note).toContainText("Zampanò has a new Chapter after this one, not included: “Interlude”.");
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/split.png` : undefined });
  await note.locator("button", { hasText: "Include it here too" }).click();
  // A second Inclusion, of the new Chapter, right after the first; and the note goes.
  await expect.poll(async () => (await ops(app)).filter((op) => op.op === "include").map((op: any) => op.chapter)).toEqual([0, 52]);
  expect(await textOf(app)).toContain("She walked home.\n\uFFFC\n\uFFFC\n");
  await expect(note).toHaveCount(0);
  expect(await app.evaluate(() => (window as any).__fake.log.some((e: any) => e.cmd === "settle_split"))).toBe(true);
});

test("an Inclusion gets Styles of its own from the bar", async ({ app }) => {
  await endOf(app, "She walked home.");
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "The whole Work" }).click();
  const styles = app.locator(".cm-inclusion .cm-inclusion-styles");
  await expect(styles).toHaveText("Styles");
  await styles.click();
  await app.locator("#menu button", { hasText: "Courier voice" }).click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.log.find((e: any) => e.cmd === "set_inclusion_styles")?.args.styles)).toEqual(["Courier voice"]);
  await expect(styles).toHaveText("Styles (1)");
  // Picked again: taken off.
  await styles.click();
  await expect(app.locator("#menu button", { hasText: "✓ Courier voice" })).toBeVisible();
  await app.locator("#menu button", { hasText: "No Styles of its own" }).click();
  await expect(styles).toHaveText("Styles");
});

test("the PDF of a Milestone shown in the Page preview is of that Milestone", async ({ app }) => {
  await app.getByRole("button", { name: "Show panel" }).click();
  await app.locator("#panel-tab-pages").click();
  // Name a Milestone, as the Author would.
  await app.locator("#works li.work-entry", { hasText: "Rain" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Name a Milestone…" }).click();
  await app.locator("#ask-text-input").fill("Sent");
  await app.locator("#ask-text-ok").click();
  const version = app.locator("#preview-version");
  await expect(version).toBeVisible();
  await version.selectOption("00000001:90");
  await app.locator("#preview-pdf").click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.log.find((e: any) => e.cmd === "export_pdf")?.args.milestone)).toBe("00000001:90");
  // Back to now: an Export of now.
  await version.selectOption({ index: 0 });
  await app.locator("#preview-pdf").click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "export_pdf").map((e: any) => e.args.milestone))).toEqual(["00000001:90", null]);
});

test("a Work dragged from the Vault into the text is included where it's dropped", async ({ app }) => {
  const zampano = app.locator("#works li.work-entry", { hasText: "Zampanò" }).locator(".entry");
  await zampano.dragTo(line(app, "She walked home."));
  // Dropped on a line with text: on a line of its own after it.
  await expect.poll(async () => (await ops(app)).find((op) => op.op === "include")).toMatchObject({ work: "Zampanò", chapter: null });
  expect(await textOf(app)).toContain("She walked home.\n\uFFFC\n");
  await expect(app.locator(".cm-inclusion .cm-inclusion-label")).toHaveText("Zampanò");
  // A Work dragged into itself is refused, and nothing changes.
  await app.locator("#works li.work-entry", { hasText: "Rain" }).locator(".entry").dragTo(line(app, "The end."));
  await expect(app.locator("#error")).toContainText("can't include itself");
  expect((await ops(app)).filter((op) => op.op === "include")).toHaveLength(1);
  expect((await textOf(app)).match(/\uFFFC/g)).toHaveLength(1);
});

test("Works picked in the Vault become a new Collection, in the order they were picked", async ({ app }) => {
  const entry = (name: string) => app.locator("#works li.work-entry", { hasText: name });
  await entry("Zampanò").locator(".entry-name").click({ modifiers: ["ControlOrMeta"] });
  await entry("Rain").locator(".entry-name").click({ modifiers: ["ControlOrMeta"] });
  // Each is marked with its place in the Collection (Rain is listed first, picked second).
  await expect(app.locator("#works li.picked .entry-pick")).toHaveText(["2", "1"]);
  expect(await app.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "open_work").length)).toBe(1); // only Rain, at the start
  await entry("Rain").click({ button: "right" });
  await app.locator("#menu button", { hasText: "New Collection from selected Works (2)…" }).click();
  await expect(app.locator("#ask-text")).toContainText("It includes Zampanò, Rain, in that order.");
  await app.locator("#ask-text-input").fill("Both");
  await app.locator("#ask-text-ok").click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.log.find((e: any) => e.cmd === "create_collection")?.args))
    .toEqual({ id: "Both", works: ["Zampanò", "Rain"] });
  // It opens, showing both, and the selection is cleared.
  await expect(app.locator(".pane:not([hidden]) .cm-inclusion-label")).toHaveText(["Zampanò", "Rain"]);
  await expect(app.locator("#works li.picked")).toHaveCount(0);
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/collection.png` : undefined });
});

test("in the Raw view an Inclusion is one line saying what it includes", async ({ app }) => {
  await endOf(app, "She walked home.");
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "Chapter: The Navidson Record" }).click();
  await expect(app.locator(".cm-inclusion")).toBeVisible();
  await app.locator("#view-mode").click();
  const raw = app.locator(".cm-inclusion-raw");
  await expect(raw).toHaveText('#inclusion("Zampanò", chapter: "The Navidson Record")');
  await expect(app.locator(".cm-inclusion")).toHaveCount(0);
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/raw.png` : undefined });
  // Typing after it goes after it; the line itself can't be typed in.
  await line(app, "She walked home.").click();
  await app.keyboard.press("ArrowDown");
  await app.keyboard.press("End");
  await app.keyboard.type(" After.");
  await expect.poll(() => textOf(app)).toContain("\uFFFC After.");
  // Back in the Formatted view: the block again.
  await app.locator("#view-mode").click();
  await expect(app.locator(".cm-inclusion .cm-inclusion-label")).toHaveText("Zampanò: The Navidson Record");
});
