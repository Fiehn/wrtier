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
