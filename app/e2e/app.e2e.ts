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

test("the panels are resized by dragging their edges, and the widths are remembered", async ({ app }) => {
  const sidebar = app.locator("#sidebar");
  const panel = app.locator("#side-panel");
  const sidebarEdge = app.locator("#sidebar-resizer");
  const panelEdge = app.locator("#panel-resizer");
  const width = (l: Locator) => l.evaluate((e) => Math.round(e.getBoundingClientRect().width));
  /** Drags a handle sideways by `by` pixels, in a few steps as a hand would. */
  async function drag(handle: Locator, ...steps: number[]) {
    const box = (await handle.boundingBox())!;
    let x = box.x + box.width / 2;
    const y = box.y + box.height / 2;
    await app.mouse.move(x, y);
    await app.mouse.down();
    for (const by of steps) await app.mouse.move((x += by), y, { steps: 3 });
    await app.mouse.up();
  }
  if (!(await panel.isVisible())) await app.keyboard.press("Control+Shift+P");
  await expect(panel).toBeVisible();

  // Today's widths, at first.
  expect(await width(sidebar)).toBe(280);
  expect(await width(panel)).toBe(538); // 42% of 1280

  await drag(panelEdge, -40); // the Side panel's edge is on its left
  expect(await width(panel)).toBe(578);
  // As wide as it can go: the editor keeps 400px.
  await drag(panelEdge, -400);
  expect(await width(panel)).toBe(1280 - 280 - 400);
  await panelEdge.dblclick();
  expect(await width(panel)).toBe(538);

  // A wider Vault browser: the Side panel gives way so the editor keeps its 400px.
  await drag(sidebarEdge, 100);
  expect(await width(sidebar)).toBe(380);
  expect(await width(panel)).toBe(1280 - 380 - 400);

  // Dragged all the way: hidden, and dragging back out before letting go brings it back.
  await drag(sidebarEdge, -300, 150);
  await expect(sidebar).toBeVisible();
  expect(await width(sidebar)).toBe(230);
  // Hidden by dragging, it comes back at the width it had before that drag.
  await drag(sidebarEdge, -200);
  await expect(sidebar).toBeHidden();
  await expect(sidebarEdge).toBeHidden();
  await app.keyboard.press("Control+\\");
  await expect(sidebar).toBeVisible();
  expect(await width(sidebar)).toBe(230);

  // The keyboard: arrows move the edge, Enter goes back to the default.
  await panelEdge.focus();
  await app.keyboard.press("ArrowLeft");
  expect(await width(panel)).toBe(538 + 16);
  await app.keyboard.press("Shift+ArrowRight");
  expect(await width(panel)).toBe(538 + 16 - 64);
  await app.keyboard.press("Home");
  expect(await width(panel)).toBe(260);
  await app.keyboard.press("Enter");
  expect(await width(panel)).toBe(538);
  await drag(panelEdge, -40);

  // A narrower window shows them narrower (the Side panel first) without forgetting.
  await app.setViewportSize({ width: 850, height: 800 });
  expect(await width(panel)).toBe(260);
  expect(await width(sidebar)).toBe(850 - 400 - 260);
  await app.setViewportSize({ width: 1280, height: 800 });
  expect(await width(sidebar)).toBe(230);
  expect(await width(panel)).toBe(578);

  // Remembered on this Device.
  await app.reload();
  await expect(line(app, "She walked home.")).toBeVisible();
  expect(await width(sidebar)).toBe(230);
  expect(await width(panel)).toBe(578);
});

test("the Page preview zooms by its menu, its keys and Ctrl + wheel, keeping the spot in view", async ({ app }) => {
  const panel = app.locator("#side-panel");
  const list = app.locator("#preview-pages");
  const zoom = app.locator("#preview-zoom");
  const first = list.locator(".page").first();
  const width = (l: Locator) => l.evaluate((e) => Math.round(e.getBoundingClientRect().width));
  /** The page in the middle of the view. */
  const middlePage = () =>
    list.evaluate((el) => {
      const box = el.getBoundingClientRect();
      const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      return (hit?.closest(".page") as HTMLElement | null)?.dataset.page ?? null;
    });

  // Eight A5 pages (420 × 595 points).
  if (await panel.isVisible()) await app.keyboard.press("Control+Shift+P");
  await app.evaluate(() => {
    (window as any).__fake.pages = Array.from({ length: 8 }, () => ({ width: 420, height: 595 }));
  });
  await app.keyboard.press("Control+Shift+P");
  await app.locator("#panel-tab-pages").click();
  await expect(list.locator(".page img")).not.toHaveCount(0);

  // Fit width at first: as wide as the column.
  await expect(zoom).toHaveValue("width");
  const column = await list.evaluate((el) => el.clientWidth - 32);
  expect(await width(first)).toBe(column);

  // 100% is the real size: 420pt is 560 CSS pixels, wider than the panel, so it scrolls sideways.
  await zoom.selectOption("1");
  expect(await width(first)).toBe(560);
  expect(await list.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true);

  // Fit page: a whole page in view.
  await zoom.selectOption("page");
  const room = await list.evaluate((el) => el.clientHeight - 8 - 14);
  expect(await first.evaluate((e) => Math.round(e.getBoundingClientRect().height))).toBe(room);

  // The keys, while the pages have the focus; the spot in the middle stays there.
  await zoom.selectOption("1");
  await list.evaluate((el) => (el.scrollTop = el.scrollHeight / 2));
  const middle = await middlePage();
  expect(middle).not.toBeNull();
  await list.focus();
  await app.keyboard.press("Control+=");
  await expect(zoom).toHaveValue("1.25");
  expect(await width(first)).toBe(700);
  expect(await middlePage()).toBe(middle);
  await app.keyboard.press("Control+-");
  await app.keyboard.press("Control+-");
  await expect(zoom).toHaveValue("0.75");
  expect(await middlePage()).toBe(middle);
  await app.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/zoom.png` : undefined });
  await app.keyboard.press("Control+0");
  await expect(zoom).toHaveValue("width");

  // Ctrl + wheel over the pages: about 10% a notch, and the menu shows where it landed.
  await zoom.selectOption("1");
  await first.hover();
  await app.keyboard.down("Control");
  await app.mouse.wheel(0, -100);
  await app.keyboard.up("Control");
  await expect.poll(() => width(first)).toBeGreaterThan(560);
  const landed = Number(await zoom.inputValue());
  expect(landed).toBeGreaterThan(1);
  expect(landed).toBeLessThanOrEqual(1.25);

  // In the editor, the keys do nothing to the pages.
  await line(app, "She walked home.").click();
  const before = await width(first);
  await app.keyboard.press("Control+=");
  expect(await width(first)).toBe(before);

  // Remembered on this Device.
  await app.reload();
  await expect(line(app, "She walked home.")).toBeVisible();
  await expect(zoom).toHaveValue(String(landed));
});

test("Find in Vault finds in every Work, opens matches in a passing tab, and keeps up", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  await page.addInitScript(() => {
    const fake = (window as any).__fake;
    fake.texts["Zampanò"] += "\n= Ending\n\nA *lighthouse* -- far off.\n";
    fake.texts.Salt = "= Shore\n\nThe lighthouse stood alone.\n";
  });
  await page.goto("/");
  await expect(line(page, "She walked home.")).toBeVisible();

  const panel = page.locator("#vault-find");
  const input = page.locator("#vault-find-input");
  const summary = page.locator("#vault-find-summary");
  const group = (name: string) => page.locator(".vf-work", { has: page.locator(".vf-work-name", { hasText: name }) });
  const tab = (name: string) => page.locator(".tab", { has: page.locator(".tab-name", { hasText: name }) });

  // Ctrl+Shift+F: in place of the Works list, the cursor in the field.
  await page.keyboard.press("Control+Shift+F");
  await expect(panel).toBeVisible();
  await expect(page.locator("#works")).toBeHidden();
  await expect(input).toBeFocused();

  // As it reads: markup hidden, -- a dash; each match under its Chapter. Rain (open) has none.
  await input.fill("lighthouse");
  await expect(summary).toHaveText("2 matches in 2 Works");
  await expect(group("Zampanò").locator(".vf-chapter")).toHaveText("Ending");
  await expect(group("Zampanò").locator(".vf-match")).toHaveText("A lighthouse – far off.");
  await expect(group("Salt").locator(".vf-match mark")).toHaveText("lighthouse");
  await expect(group("Rain")).toHaveCount(0);

  // A match opens its Work in a passing tab; the next Work found takes its place.
  await group("Zampanò").locator(".vf-match").click();
  await expect(tab("Zampanò")).toHaveClass(/passing/);
  await expect(tab("Zampanò")).toHaveClass(/active/);
  await expect(page.locator(".cm-line", { hasText: "far off" }).first()).toBeVisible();
  await group("Salt").locator(".vf-match").click();
  await expect(tab("Salt")).toHaveClass(/passing/);
  await expect(tab("Zampanò")).toHaveCount(0);
  await expect(tab("Rain")).not.toHaveClass(/passing/); // open before: untouched
  // Double-clicked, it stays.
  await tab("Salt").locator(".tab-name").dblclick();
  await expect(tab("Salt")).not.toHaveClass(/passing/);

  // Up and Down go through the matches, keeping the focus in the list.
  await input.focus();
  await page.keyboard.press("ArrowDown");
  await expect(page.locator(".vf-match.current")).toHaveCount(1);
  await expect(page.locator("#vault-find-results")).toBeFocused();

  // The list keeps up with typing in an open Work.
  await tab("Rain").locator(".tab-name").click();
  await endOf(page, "The end.");
  await page.keyboard.type(" A lighthouse.");
  await expect(summary).toHaveText("3 matches in 3 Works");
  await expect(group("Rain").locator(".vf-chapter")).toHaveText("Three");
  await page.screenshot({ path: process.env.SHOTS ? `${process.env.SHOTS}/find-in-vault.png` : undefined });

  // Open with Find and replace: an ordinary tab, Find showing the same search, for Replace all there.
  await group("Zampanò").locator(".vf-open-find").click();
  await expect(tab("Zampanò")).not.toHaveClass(/passing/);
  await expect(page.locator("#find-bar")).toBeVisible();
  await expect(page.locator("#find-input")).toHaveValue("lighthouse");
  await expect(page.locator("#find-replace-row")).toBeVisible();

  // Esc: the Works come back; the search is still there next time.
  await input.focus();
  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden();
  await expect(page.locator("#works")).toBeVisible();
  await page.locator("#open-find-in-vault").click();
  await expect(input).toHaveValue("lighthouse");
  await expect(summary).toHaveText("3 matches in 3 Works");

  // The button again, while finding: it closes, as Esc does.
  await page.locator("#open-find-in-vault").click();
  await expect(panel).toBeHidden();
  await expect(page.locator("#works")).toBeVisible();

  expect(errors, "errors in the page").toEqual([]);
  expect(await page.evaluate(() => (window as any).__fake.unknown), "commands the fake doesn't know").toEqual([]);
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
  // The cursor follows the text's colour, not CodeMirror's black (issue #4).
  await expect(page.locator(".cm-content").first()).toHaveCSS("caret-color", "rgb(232, 228, 220)");
});

for (const [theme, caret] of [["dark", "rgb(232, 228, 220)"], ["vault:ember.css", "rgb(240, 120, 40)"]]) {
  test(`the writing cursor is the text's colour in dark mode, and a theme can colour it (${theme})`, async ({ page }) => {
    await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
    await page.addInitScript((theme) => {
      const fake = (window as any).__fake;
      fake.themes["ember.css"] = { name: "Ember", dark: true, css: "/* Theme: Ember, dark */\n:root { --cursor: rgb(240, 120, 40); }" };
      fake.settings.theme = theme;
    }, theme);
    await page.goto("/");
    await expect(line(page, "She walked home.")).toBeVisible();
    await expect(page.locator(".cm-content").first()).toHaveCSS("caret-color", caret);
  });
}

test("Settings shows the App version, and an update found is saved for, downloaded and installed", async ({ app }) => {
  await app.evaluate(() => {
    (window as any).__fake.update = { kind: "available", current: "0.1.2", version: "0.2.0" };
  });
  await app.locator("#open-settings").click();
  await app.locator("#settings-tab-app").click();
  await expect(app.locator("#app-version")).toHaveText("Wrtier 0.1.2");
  await app.locator("#check-updates").click();

  const dialog = app.locator("#update");
  await expect(app.locator("#update-message")).toHaveText("Version 0.2.0 is available (you have 0.1.2).");
  // The check is over: no moving bar until a download starts.
  await expect(app.locator("#update-progress")).toBeHidden();
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
  await app.locator("#settings-tab-app").click();
  await app.locator("#check-updates").click();
  await expect(app.locator("#update-message")).toHaveText("You have the newest version (0.1.2).");
  await expect(app.locator("#update-progress")).toBeHidden(); // the check is over: nothing moving
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
  await app.locator("#settings-tab-app").click();
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
  await app.locator("#settings-tab-app").click();
  await app.locator("#open-uninstall").click();
  await expect(app.locator("#uninstall-error")).toHaveText("The app can't change its own folder.");
  await expect(app.locator("#uninstall-go")).toBeHidden();
  await expect(app.locator("#uninstall-unsaved")).toBeHidden();
});

test("Settings has its tabs, and opens again on the one last shown", async ({ app }) => {
  const dialog = app.locator("#settings");
  await app.locator("#open-settings").click();
  await expect(app.locator("#settings-tab-editor")).toHaveAttribute("aria-selected", "true");
  await expect(app.locator("#settings-editor")).toBeVisible();
  await expect(app.locator("#settings-vault")).toBeHidden();
  // Switching tabs never resizes the dialog.
  const height = (await dialog.boundingBox())!.height;
  await app.locator("#settings-tab-shortcuts").click();
  await expect(app.locator("#shortcuts-list h3").first()).toBeVisible();
  await app.locator("#settings-tab-reviewers").focus();
  await app.keyboard.press("ArrowRight");
  await expect(app.locator("#settings-tab-vault")).toBeFocused();
  await expect(app.locator("#settings-vault")).toBeVisible();
  expect((await dialog.boundingBox())!.height).toBe(height);
  await app.locator("#settings button[value=done]").click();
  await expect(dialog).toBeHidden();
  await app.locator("#open-settings").click();
  await expect(app.locator("#settings-vault")).toBeVisible();
});

test("a setting only on this device shows what the others use, and unticking goes back to that", async ({ app }) => {
  await app.locator("#open-settings").click();
  const other = app.locator("#font-size-other");
  await expect(other).toBeHidden();
  await app.locator("#font-size-here").check();
  await expect(other).toHaveText("Other devices: 17");
  await app.locator("#font-size").fill("20");
  await app.locator("#font-size").press("Tab");
  await expect(other).toHaveText("Other devices: 17");
  const asked = () => app.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd.startsWith("set_setting")).map((e: any) => [e.cmd, e.args]));
  expect(await asked()).toContainEqual(["set_setting", { name: "fontSize", value: "20", here: true }]);
  // Unticking: back on the shared 17, without setting it for every device.
  await app.locator("#font-size-here").uncheck();
  await expect(other).toBeHidden();
  await expect(app.locator("#font-size")).toHaveValue("17");
  expect(await asked()).toContainEqual(["set_setting_here", { name: "fontSize", here: false }]);
  expect(await asked()).not.toContainEqual(["set_setting", { name: "fontSize", value: "20", here: false }]);
});

test("with no Vault open, Settings has only the App tab", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  await page.addInitScript(() => {
    (window as any).__fake.lastVault = null;
  });
  await page.goto("/");
  await page.locator("#open-settings").click();
  await expect(page.locator("#settings-app")).toBeVisible();
  await expect(page.locator("#settings-tabs")).toBeHidden();
  await expect(page.locator("#settings-editor")).toBeHidden();
  await expect(page.locator("#check-updates")).toBeVisible();
  const cmds = await page.evaluate(() => (window as any).__fake.log.map((e: any) => e.cmd) as string[]);
  expect(cmds).not.toContain("get_settings");
  expect(errors, "errors in the page").toEqual([]);
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

test("an Image's Past pictures are listed under it, used again or deleted for good", async ({ app }) => {
  await endOf(app, "She walked home.");
  await app.keyboard.press("Control+Alt+i");
  await app.locator("#panel-toggle").click();
  await app.locator("#panel-tab-images").click();
  await expect(app.locator(".image-row .image-title")).toHaveText("map.png");
  await expect(app.locator(".image-past-toggle")).toHaveCount(0); // none yet
  // Another picture by the same name: replaced, and the first is a Past picture.
  await app.locator("#images-list").getByRole("button", { name: "Insert Image…" }).click();
  await app.locator("#image-clash button[value=replace]").click();
  const toggle = app.locator(".image-past-toggle");
  await expect(toggle).toHaveText(/▸ 1 Past picture/);
  await toggle.click();
  await expect(app.locator(".image-past-row")).toHaveCount(1);
  await expect(app.locator(".image-past-row img")).toHaveCount(1);
  // Used again: the Image has it once more, and the one it had is the Past picture now.
  const first = "1".padStart(64, "0");
  await app.locator(".image-past-row").getByRole("button", { name: "Use this one" }).click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.images.Rain["map.png"].hash)).toBe(first);
  const second = "2".padStart(64, "0");
  await expect(app.locator(".image-past-row")).toHaveAttribute("data-hash", second);
  // Deleted for good, after asking.
  await app.locator(".image-past-row").getByRole("button", { name: "Delete" }).click();
  await app.locator("#ask-confirm-yes", { hasText: "Delete for good" }).click();
  await expect(app.locator(".image-past-toggle")).toHaveCount(0);
  const asked = await app.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "delete_past_pictures").map((e: any) => e.args.hashes));
  expect(asked).toEqual([[second]]);
});

test("a click after an Inclusion lands on the line clicked", async ({ app }) => {
  await endOf(app, "She walked home.");
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "Chapter: The Navidson Record" }).click();
  await expect(app.locator(".cm-inclusion-text")).toContainText("a quarter inch more inside than out");
  // The editor knows how tall the Inclusion is, margins and all: each line below is where it
  // shows, so a click in the middle of one puts the cursor on it.
  for (const text of ["Morning came slowly.", "The end."]) {
    const box = (await line(app, text).boundingBox())!;
    await app.mouse.click(box.x + 30, box.y + box.height / 2);
    await expect.poll(() => app.evaluate(() => {
      const node = window.getSelection()?.anchorNode;
      const el = node instanceof Element ? node : node?.parentElement;
      return el?.closest(".cm-line")?.textContent ?? "";
    })).toBe(text);
  }
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
    await expect(app.locator("#panes .cm-content")).toBeFocused();

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

test("a Footnote is written at the cursor, folds to its number, and lists in the Footnotes tab", async ({ app }) => {
  await endOf(app, "She walked home.");
  // No Footnotes yet: no tab for them.
  await expect(app.locator("#panel-tab-footnotes")).toBeHidden();
  await app.keyboard.press("Control+Alt+f");
  await app.keyboard.type("In the rain.");
  // Open while the cursor is in it: its number, then its text.
  await expect(app.locator(".cm-tp-footnote-open")).toHaveText("1");
  await expect(app.locator(".cm-tp-footnote")).toHaveText("In the rain.");
  // The cursor leaves: folded to its number, the sentence reading on.
  await app.keyboard.press("Control+Home");
  await expect(app.locator(".cm-tp-footnote")).toHaveCount(0);
  await expect(app.locator(".cm-tp-footnote-mark")).toHaveText("1");
  // The Footnotes tab is offered now, and lists it.
  await app.locator("#panel-toggle").click();
  await app.locator("#panel-tab-footnotes").click();
  await expect(app.locator(".footnote-row")).toContainText("In the rain.");
  // Clicking it puts the cursor in it: open again.
  await app.locator(".footnote-row").click();
  await expect(app.locator(".cm-tp-footnote")).toHaveText("In the rain.");
});

test("an Image is inserted on a line of its own, shown as its picture, and listed in the Images tab", async ({ app }) => {
  await endOf(app, "She walked home.");
  // No Images yet: no tab for them.
  await expect(app.locator("#panel-tab-images")).toBeHidden();
  await app.keyboard.press("Control+Alt+i");
  await expect.poll(() => app.evaluate(() => (window as any).__fake.texts.Rain)).toContain('She walked home.\n#image("map.png")\n');
  // While the cursor's on it, the markup; once it leaves, the picture with its name.
  await app.keyboard.press("Control+Home");
  await expect(app.locator(".cm-image-block img")).toHaveCount(1);
  await expect(app.locator(".cm-image-name")).toHaveText("map.png");
  // The Images tab is offered now, and lists it, used once.
  await app.locator("#panel-toggle").click();
  await app.locator("#panel-tab-images").click();
  await expect(app.locator(".image-row .image-title")).toHaveText("map.png");
  await expect(app.locator(".image-row .image-facts")).toContainText("used once");
  // Clicking low down on the picture puts the cursor on its line (not on the line the click
  // would hit if the picture weren't there): its markup shows.
  const box = (await app.locator(".cm-image-block img").boundingBox())!;
  await app.mouse.click(box.x + box.width / 2, box.y + box.height - 5);
  await expect(app.locator(".cm-image-block")).toHaveCount(0);
  await expect(app.locator(".cm-line").filter({ hasText: '#image("map.png")' })).toHaveCount(1);
  await app.keyboard.press("Control+Home");
  await expect(app.locator(".cm-image-block img")).toHaveCount(1);
  // "Go to" puts the cursor in its name: the markup shows again.
  await app.locator(".image-row").getByRole("button", { name: "Go to" }).click();
  await expect(app.locator(".cm-image-block")).toHaveCount(0);
});

test("included text is annotated: the note is kept here and numbered in the text", async ({ app }) => {
  await endOf(app, "She walked home.");
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Include in Rain…" }).click();
  await app.locator("#menu button", { hasText: "Chapter: The Navidson Record" }).click();
  const text = app.locator(".cm-inclusion-text p", { hasText: "quarter inch" });
  await expect(text).toBeVisible();
  // Select "quarter inch" in it, then Insert footnote's key: an Annotation.
  await text.evaluate((p) => {
    const node = p.firstChild!;
    const start = node.textContent!.indexOf("quarter inch");
    const range = document.createRange();
    range.setStart(node, start);
    range.setEnd(node, start + "quarter inch".length);
    getSelection()!.removeAllRanges();
    getSelection()!.addRange(range);
    (p.closest(".cm-inclusion") as HTMLElement).focus();
  });
  await app.keyboard.press("Control+Alt+f");
  const dialog = app.locator("dialog.annotation-dialog");
  await expect(dialog).toContainText("Annotation on “quarter inch”");
  await dialog.locator(".cm-content").click();
  await app.keyboard.type("Truant measured it twice.");
  await dialog.locator("button", { hasText: "Annotate" }).click();
  const sent = await app.evaluate(() => (window as any).__fake.log.find((e: any) => e.cmd === "annotate")?.args);
  expect(sent).toMatchObject({ id: "Rain", from: 23 + "It measured a ".length, to: 23 + "It measured a quarter inch".length, body: "Truant measured it twice." });
  // Its number follows what it covers, which is marked.
  await expect(app.locator(".cm-inclusion .cm-incl-annotation")).toHaveAttribute("data-n", "1");
  await expect(app.locator(".cm-inclusion .cm-annotated")).toHaveText("quarter inch");
  // Opened from its number, changed.
  await app.locator(".cm-inclusion .cm-incl-annotation").click();
  await expect(dialog).toBeVisible();
  await dialog.locator(".cm-content").click();
  await app.keyboard.press("Control+End");
  await app.keyboard.type(" Then again.");
  await dialog.locator("button", { hasText: "Save" }).click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.log.find((e: any) => e.cmd === "set_annotation")?.args.body)).toBe("Truant measured it twice. Then again.");
  // Listed in the Footnotes tab as this Work's.
  await app.locator("#panel-toggle").click();
  await app.locator("#panel-tab-footnotes").click();
  await expect(app.locator(".footnote-row.annotation.mine")).toContainText("Then again.");
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
  await app.getByRole("button", { name: "Show the panel" }).click();
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

test("with the Colours Vault Mod, a Work is coloured from its right-click menu", async ({ app }) => {
  const rain = app.locator("#works li.work-entry", { hasText: "Rain" });
  // Without the Mod, there's no Colour… to choose.
  await rain.locator(".entry-name").click({ button: "right" });
  await expect(app.locator("#menu button", { hasText: "Mods…" })).toBeVisible();
  await expect(app.locator("#menu button", { hasText: "Colour…" })).toHaveCount(0);
  await app.keyboard.press("Escape");
  // Switched on in Settings, for the whole Vault.
  await app.locator("#open-settings").click();
  await app.locator("#settings-tab-vault").click();
  await app.locator("#vault-mods label", { hasText: "Colours" }).locator("input").check();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.vaultMods)).toEqual(["colours"]);
  await app.locator("#settings button[value=done]").click();
  await rain.locator(".entry-name").click({ button: "right" });
  await app.locator("#menu button", { hasText: "Colour…" }).click();
  await app.locator("#menu .colour-pick[aria-label=Red]").click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.colours.works.Rain)).toBe("red");
  await expect(rain).toHaveClass(/coloured/);
  const red = await app.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--reviewer-red").trim());
  const shown = await rain.locator(".entry-name").evaluate((el) => getComputedStyle(el).color);
  const probe = await app.evaluate((c) => {
    const el = document.createElement("span");
    el.style.color = c;
    document.body.append(el);
    const rgb = getComputedStyle(el).color;
    el.remove();
    return rgb;
  }, red);
  expect(shown).toBe(probe);
  // No colour again.
  await rain.locator(".entry-name").click({ button: "right" });
  await app.locator("#menu button", { hasText: "Colour…" }).click();
  await app.locator("#menu .colour-pick[aria-label='No colour']").click();
  await expect(rain).not.toHaveClass(/coloured/);
});

test("Works and folders move by dragging onto a folder, or with Move to…", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  await page.addInitScript(() => {
    const fake = (window as any).__fake;
    fake.texts["Short stories/Storm"] = "= Storm\n\nThunder.\n";
    fake.vaultFolders = ["Notes"];
  });
  await page.goto("/");
  await expect(line(page, "She walked home.")).toBeVisible();
  const renames = () => page.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd.startsWith("rename_")).map((e: any) => [e.cmd, e.args]));
  const folderRow = (name: string) => page.locator("#works li.folder > .folder-toggle", { hasText: name }).first();

  // Rain (open in its tab) dragged onto a closed folder: it moves in, the folder opens and the
  // row flashes; the tab follows it.
  await page.locator("#works li.work-entry", { hasText: "Rain" }).locator(".entry").dragTo(folderRow("Short stories"));
  await expect.poll(renames).toEqual([["rename_work", { id: "Rain", name: "Short stories/Rain" }]]);
  const moved = page.locator('#works li.work-entry[data-id="Short stories/Rain"]');
  await expect(moved).toBeVisible();
  await expect(moved).toHaveClass(/just-moved/);
  await expect(line(page, "She walked home.")).toBeVisible();

  // A folder dragged onto another goes in.
  await folderRow("Notes").dragTo(folderRow("Short stories"));
  await expect.poll(async () => (await renames()).at(-1)).toEqual(["rename_folder", { path: "Notes", name: "Short stories/Notes" }]);
  await expect(page.locator('#works li.folder[data-path="Short stories/Notes"]')).toBeVisible();
  // But not into one inside itself: nothing moves, and the info line says why.
  await folderRow("Short stories").dragTo(folderRow("Notes"));
  await expect(page.locator("#info")).toHaveText("A folder can't go inside itself.");
  expect(await renames()).toHaveLength(2);

  // Move to…: from the right-click menu, to the top of the Vault.
  await page.locator("#works li.work-entry", { hasText: "Storm" }).locator(".entry-name").click({ button: "right" });
  await page.locator("#menu button", { hasText: "Move to…" }).click();
  await expect(page.locator("#menu button")).toHaveText(["Top of the Vault", "Short stories/Notes"]);
  await page.locator("#menu button", { hasText: "Top of the Vault" }).click();
  await expect.poll(async () => (await renames()).at(-1)).toEqual(["rename_work", { id: "Short stories/Storm", name: "Storm" }]);
  await expect(page.locator('#works > li.work-entry[data-id="Storm"]')).toBeVisible();

  expect(errors, "errors in the page").toEqual([]);
  expect(await page.evaluate(() => (window as any).__fake.unknown), "commands the fake doesn't know").toEqual([]);
});

test("the Style library and the trash are pinned under the Works, above the Chapters", async ({ app }) => {
  const order = await app.evaluate(() => [...document.querySelectorAll("#sidebar > *")].map((el) => el.id));
  expect(order).toEqual(["sidebar-top", "sidebar-vault", "sidebar-shelf", "chapters"]);
  await expect(app.locator("#sidebar-shelf #library")).toBeVisible();
  // However the Works scroll, the shelf stays where it is.
  const before = await app.locator("#sidebar-shelf").boundingBox();
  await app.locator("#sidebar-vault").evaluate((el) => (el.scrollTop = 1000));
  expect(await app.locator("#sidebar-shelf").boundingBox()).toEqual(before);
});

test("each Style library group has a plus box on its line for making a new one", async ({ app }, info) => {
  await app.locator("#library > summary").click();
  const styles = app.locator("#library .library-group", { has: app.locator("summary", { hasText: "Styles" }) });
  const plus = styles.locator("summary .library-add");
  await expect(app.locator("#library .library-add")).toHaveCount(3);
  // At the right end of the group's line.
  const line = (await styles.locator("summary").boundingBox())!;
  const box = (await plus.boundingBox())!;
  expect(line.x + line.width - (box.x + box.width)).toBeLessThan(8);
  await app.locator("#library").screenshot({ path: info.outputPath("library.png") });
  // It asks for the new one's name, and the group stays as it was.
  const open = await styles.evaluate((d) => (d as HTMLDetailsElement).open);
  await plus.click();
  await expect(app.locator("#ask-text")).toBeVisible();
  await expect(app.locator("#ask-text-question")).toHaveText("What's the new Style called?");
  expect(await styles.evaluate((d) => (d as HTMLDetailsElement).open)).toBe(open);
  await app.keyboard.press("Escape");
});

test("a Work's Preamble… and Mods… are in the tab row and its menu, not on the Work when hovered", async ({ app }, info) => {
  const rain = app.locator("#works li.work-entry", { hasText: "Rain" });
  await rain.hover();
  await expect(rain.locator("button", { hasText: /Preamble…|Mods…/ })).toHaveCount(0);
  // In the tab row, beside Find, Raw view and the panel's button, for the Work in front.
  const row = await app.evaluate(() => [...document.querySelectorAll("#tab-actions > button:not([hidden])")].map((b) => b.id));
  expect(row).toEqual(["find-open", "view-mode", "preamble-open", "mods-open"]);
  // And the panel's symbol at the far right, as ☰ is at the far left.
  expect(await app.evaluate(() => document.querySelector("#tabrow")!.lastElementChild!.id)).toBe("panel-toggle");
  await app.locator("#tabrow").screenshot({ path: info.outputPath("tabrow.png") });
  // The panel's button is a symbol, saying what it does and whether the panel shows.
  const toggle = app.locator("#panel-toggle");
  await expect(toggle).toHaveText("");
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  await expect(toggle).toHaveAttribute("aria-label", "Show the panel");
  await toggle.click();
  await expect(app.locator("#side-panel")).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  await expect(toggle).toHaveAttribute("aria-label", "Hide the panel");
  await expect(toggle).toHaveAttribute("title", /^Hide the panel \(/);
  await app.locator("#tabrow").screenshot({ path: info.outputPath("tabrow-shown.png") });
  await toggle.click();
  await expect(app.locator("#side-panel")).toBeHidden();
  await app.locator("#preamble-open").click();
  await expect(app.locator("#preamble")).toBeVisible();
  await app.keyboard.press("Escape");
  await expect(app.locator("#preamble")).toBeHidden();
  await app.locator("#mods-open").click();
  await expect(app.locator("#mods")).toBeVisible();
  await app.locator("#mods button[value=done]").click();
  // And in the Work's right-click menu.
  await rain.locator(".entry-name").click({ button: "right" });
  await expect(app.locator("#menu button", { hasText: "Preamble…" })).toBeVisible();
  await expect(app.locator("#menu button", { hasText: "Mods…" })).toBeVisible();
});

test("a click outside a dialog closes it as Esc does, unless it started inside or the dialog says no", async ({ app }) => {
  await app.locator("#library > summary").click();
  const dialog = app.locator("#ask-text");
  const plus = app.locator("#library .library-add").first();
  await plus.click();
  await expect(dialog).toBeVisible();
  // Selecting the name and letting go outside: still open.
  const field = (await app.locator("#ask-text-input").boundingBox())!;
  await app.mouse.move(field.x + 5, field.y + field.height / 2);
  await app.mouse.down();
  await app.mouse.move(5, 5);
  await app.mouse.up();
  await expect(dialog).toBeVisible();
  // A click on the backdrop: closed, and nothing made.
  await app.mouse.click(5, 5);
  await expect(dialog).toBeHidden();
  expect(await app.evaluate(() => (window as any).__fake.log.some((e: any) => e.cmd === "create_library"))).toBe(false);
  // A dialog that's busy (as an update installing) refuses Esc, and the click outside too.
  await app.evaluate(() => document.getElementById("ask-text")!.addEventListener("cancel", (e) => e.preventDefault()));
  await plus.click();
  await app.mouse.click(5, 5);
  await expect(dialog).toBeVisible();
});

test("the tab row's Find opens the Find bar, and closes it again", async ({ app }) => {
  await app.locator("#find-open").click();
  await expect(app.locator("#find-bar")).toBeVisible();
  await expect(app.locator("#find-input")).toBeFocused();
  await app.locator("#find-open").click();
  await expect(app.locator("#find-bar")).toBeHidden();
  await app.locator("#find-open").click();
  await expect(app.locator("#find-bar")).toBeVisible();
});

// ---- The Reading panel (reading.ts) ----

const reading = (page: Page) => page.locator("#reading-panel");
const readLine = (page: Page, text: string): Locator => reading(page).locator(".cm-line", { hasText: text }).first();
const savedReading = (page: Page) =>
  page.evaluate(() => (window as any).__fake.log.filter((e: any) => e.cmd === "save_layout").map((e: any) => e.args.layout.reading).pop());

test("another Work is read beside the one being written: read-only, found in, and closed", async ({ app }) => {
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Read beside" }).click();
  await expect(reading(app).locator("#reading-name")).toHaveText("Zampanò");
  await expect(readLine(app, "The hallway was not there before.")).toBeVisible();
  // The Work in front is still the one being written.
  await expect(app.locator("#tabbar .tab.active")).toHaveText(/Rain/);
  await expect(app.locator("#tabbar .tab")).toHaveCount(1);
  // Nothing can be typed in it.
  await readLine(app, "The hallway was not there before.").click();
  await app.keyboard.type("Nope");
  await expect(readLine(app, "The hallway was not there before.")).toHaveText("The hallway was not there before.");
  expect(await app.evaluate(() => (window as any).__fake.texts["Zampanò"])).not.toContain("Nope");
  // Its Chapters, to go to.
  await expect(app.locator("#reading-chapters option")).toHaveText(["Go to Chapter…", "Exploration A", "The Navidson Record"]);
  // Find finds in it, at its top, and never replaces there.
  await app.keyboard.press("Control+f");
  await expect(reading(app).locator("#find-bar")).toBeVisible();
  await expect(app.locator("#find-toggle-replace")).toBeHidden();
  await app.keyboard.type("quarter inch");
  await expect(app.locator("#find-count")).toHaveText("1 of 1");
  await app.keyboard.press("Control+h");
  await expect(app.locator("#find-replace-row")).toBeHidden();
  await app.keyboard.press("Escape");
  await expect(app.locator("#find-bar")).toBeHidden();
  // Remembered on this Device, then closed.
  await expect.poll(() => savedReading(app)).toMatchObject({ work: "Zampanò" });
  await app.locator("#reading-close").click();
  await expect(reading(app)).toBeHidden();
  await expect.poll(() => savedReading(app)).toBeNull();
  // Find is back above the editor, for the Work in front.
  await line(app, "She walked home.").click();
  await app.keyboard.press("Control+f");
  await expect(app.locator("main #find-bar")).toBeVisible();
  await expect(app.locator("#find-toggle-replace")).toBeVisible();
});

test("the Work in front, read beside itself, follows the typing at once", async ({ app }) => {
  await app.locator("#tabbar .tab", { hasText: "Rain" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Read beside" }).click();
  await expect(readLine(app, "Morning came slowly.")).toBeVisible();
  await endOf(app, "The end.");
  await app.keyboard.type(" Really, the end.");
  await expect(readLine(app, "The end. Really, the end.")).toBeVisible();
});

test("a Comment in the Work being read shows as a card, and opens in its Comments panel", async ({ app }) => {
  await app.evaluate(() => {
    const fake = (window as any).__fake;
    const text: string = fake.texts["Zampanò"];
    const from = text.indexOf("hallway");
    fake.readingMarks = { "Zampanò": [{
      id: "c1", ranges: [[from, from + 7]], onWork: false, sender: "reviewer/anna", name: "Anna", colour: "teal",
      body: "Which hallway?", resolved: false, quote: "hallway", timeMs: 0, otherBody: null,
      replies: [{ id: "r1", sender: "author", name: "Author", colour: "author", body: "The one on Ash Tree Lane.", timeMs: 0, otherBody: null }],
    }] };
  });
  await app.locator("#works li.work-entry", { hasText: "Zampanò" }).click({ button: "right" });
  await app.locator("#menu button", { hasText: "Read beside" }).click();
  const highlight = reading(app).locator(".cm-comment", { hasText: "hallway" });
  await expect(highlight).toBeVisible();
  await highlight.click();
  const card = app.locator("#reading-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText("Which hallway?");
  await expect(card).toContainText("The one on Ash Tree Lane.");
  await card.locator("button", { hasText: "Open in the Comments panel" }).click();
  await expect(app.locator("#tabbar .tab.active")).toHaveText(/Zampanò/);
  await expect(app.locator("#comments-pane")).toBeVisible();
  // It's open in a tab now: the panel shows the tab's text from here on.
  await expect(readLine(app, "The hallway was not there before.")).toBeVisible();
});

test("the Work being read is back after a restart, and a Work dragged onto the panel is read there", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.addInitScript({ path: path.join(import.meta.dirname, "fake-tauri.js") });
  await page.addInitScript(() => {
    (window as any).__fake.reading = { work: "Rain", identity: null, top: 0 };
  });
  await page.goto("/");
  await expect(readLine(page, "Morning came slowly.")).toBeVisible();
  await page.locator("#works li.work-entry", { hasText: "Zampanò" }).locator(".entry").dragTo(readLine(page, "Morning came slowly."));
  await expect(page.locator("#reading-name")).toHaveText("Zampanò");
  await expect(readLine(page, "The hallway was not there before.")).toBeVisible();
  expect(errors, "errors in the page").toEqual([]);
});

test("the Dialogue checker marks dialogue, and a Suggestion is accepted as typing, or the Mark dismissed", async ({ app }) => {
  // Switched on in the Work's Mods.
  const rain = app.locator("#works li.work-entry", { hasText: "Rain" });
  await rain.locator(".entry-name").click({ button: "right" });
  await app.locator("#menu button", { hasText: "Mods…" }).click();
  await app.locator("#mods-list label", { hasText: "Dialogue checker" }).locator("input").check();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.mods.Rain)).toContain("dialogue");
  await app.locator("#mods button[value=done]").click();

  await endOf(app, "She walked home.");
  await app.keyboard.type(' "Hi." she said. "Wait---" She shouted.');
  const mark = app.locator(".cm-mark-rule");
  await expect(mark).toHaveCount(2, { timeout: 5000 });
  await expect(app.locator("#mark-counts .mark-count-rule")).toHaveText("2");

  // Ctrl+. on the first: its card, and 1 accepts the Suggestion, as an edit typed by the Author.
  await mark.first().click();
  await app.keyboard.press("Escape");
  await app.keyboard.press("Control+.");
  const card = app.locator(".cm-mark-card");
  await expect(card).toContainText("Rule break");
  await expect(card.locator(".cm-mark-suggestion").first()).toContainText('"Hi," she said.');
  await app.keyboard.press("1");
  await expect.poll(() => textOf(app)).toContain('She walked home. "Hi," she said.');
  expect((await ops(app)).some((op) => op.op === "type" && op.text === ",")).toBe(true);

  // F8 goes to the other, and D dismisses it: kept by the Vault, not typed.
  await app.keyboard.press("F8");
  await expect(card).toContainText("lowercase");
  await app.keyboard.press("d");
  await expect(app.locator(".cm-mark")).toHaveCount(0);
  expect((await ops(app)).some((op) => op.op === "dismiss" && (op as any).key === "dialogue/capital-tag")).toBe(true);
  await expect(app.locator("#mark-counts")).toBeHidden();
  // Still dismissed after the Mods look again.
  await app.keyboard.press("End");
  await app.keyboard.type(" Fine.");
  await app.waitForTimeout(1200);
  await expect(app.locator(".cm-mark")).toHaveCount(0);
});

test("the Spellchecker marks unknown words, suggests when asked, and allows a name in the Work", async ({ app }) => {
  const rain = app.locator("#works li.work-entry", { hasText: "Rain" });
  await rain.locator(".entry-name").click({ button: "right" });
  await app.locator("#menu button", { hasText: "Mods…" }).click();
  await app.locator("#mods-list label", { hasText: "Spellchecker" }).locator("input").check();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.mods.Rain)).toContain("spellcheck");
  await expect(app.locator("#mods-list")).toContainText("English (United Kingdom)");
  await app.locator("#mods button[value=done]").click();
  // The platform's own spellchecking stays off.
  await expect(app.locator(".cm-content").first()).toHaveAttribute("spellcheck", "false");

  await endOf(app, "She walked home.");
  await app.keyboard.type(" Teh rain fell on Kvothe.");
  await expect(app.locator(".cm-mark-mistake")).toHaveCount(1, { timeout: 5000 });
  await expect(app.locator(".cm-mark-query")).toHaveCount(1);

  // Ctrl+. on "Teh": its Suggestions come when the card opens, capitalised as typed; 1 accepts.
  await app.locator(".cm-mark-mistake").click();
  await app.keyboard.press("Escape");
  await app.keyboard.press("Control+.");
  const card = app.locator(".cm-mark-card");
  await expect(card).toContainText("Mistake");
  await expect(card.locator(".cm-mark-suggestion").first()).toHaveText(/The/);
  await expect(card).toContainText("Dismiss here only");
  await app.keyboard.press("1");
  await expect.poll(() => textOf(app)).toContain("She walked home. The rain fell on Kvothe.");

  // "Kvothe" looks like a name: Allow in Work keeps it for this Work, and its Mark goes.
  await app.locator(".cm-mark-query").click();
  await card.locator("button", { hasText: "Allow in Work" }).click();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.modSettings.work.Rain)).toEqual({ "spellcheck.allow.Kvothe": "on" });
  await expect(app.locator(".cm-mark")).toHaveCount(0, { timeout: 5000 });
});

test("the Spellchecker says what it can get wrong, and shows five Suggestions with the rest under See more", async ({ app }) => {
  const rain = app.locator("#works li.work-entry", { hasText: "Rain" });
  await rain.locator(".entry-name").click({ button: "right" });
  await app.locator("#menu button", { hasText: "Mods…" }).click();
  const row = app.locator("#mods-list li.mod-row", { hasText: "Spellchecker" });
  // In its About, which pops up, not in the list itself.
  await expect(row.locator(".mod-caveat")).toHaveCount(0);
  await row.locator(".mod-about").click();
  await expect(app.locator("#menu.menu-note .mod-caveat")).toContainText("only been tested on English");
  await app.locator("#mods-list li.mod-row", { hasText: "Dialogue checker" }).locator(".mod-about").click();
  await expect(app.locator("#menu.menu-note .mod-caveat")).toContainText("isn't perfect");
  await app.keyboard.press("Escape");
  await expect(app.locator("#menu")).toHaveCount(0);
  await expect(app.locator("#mods")).toBeVisible(); // Escape closed only the About
  await expect(app.locator("#mods-list li.mod-row", { hasText: "Grammar checker" }).locator(".mod-beta")).toHaveText("(beta)");
  await row.locator("label", { hasText: "Spellchecker" }).locator("input").check();
  await expect.poll(() => app.evaluate(() => (window as any).__fake.mods.Rain)).toContain("spellcheck");
  await app.locator("#mods button[value=done]").click();

  // From the keyboard: five, then M shows the other two, and 7 accepts the last.
  await endOf(app, "She walked home.");
  await app.keyboard.type(" I recieve it.");
  const mark = app.locator(".cm-mark-mistake");
  await expect(mark).toHaveCount(1, { timeout: 5000 });
  await mark.click();
  await app.keyboard.press("Escape");
  await app.keyboard.press("Control+.");
  const card = app.locator(".cm-mark-card");
  await expect(card.locator(".cm-mark-suggestion")).toHaveCount(5);
  await expect(card.locator(".cm-mark-more")).toHaveText(/See 2 more/);
  await app.keyboard.press("m");
  await expect(card.locator(".cm-mark-suggestion")).toHaveCount(7);
  await expect(card.locator(".cm-mark-more")).toHaveCount(0);
  await app.keyboard.press("7");
  await expect.poll(() => textOf(app)).toContain("She walked home. I recite it.");

  // By clicking: See more, then the last.
  await endOf(app, "I recite it.");
  await app.keyboard.type(" I recieve more.");
  await expect(mark).toHaveCount(1, { timeout: 5000 });
  await mark.click();
  await card.locator(".cm-mark-more").click();
  await card.locator(".cm-mark-suggestion", { hasText: "recite" }).click();
  await expect.poll(() => textOf(app)).toContain("I recite it. I recite more.");
});

test("a Mark's Suggestion is accepted by clicking it on the card", async ({ app }) => {
  await app.evaluate(() => (window as any).__fake.mods.Rain.push("dialogue"));
  await app.locator("#works li.work-entry", { hasText: "Rain" }).locator(".entry-name").click({ button: "right" });
  await app.locator("#menu button", { hasText: "Mods…" }).click();
  await app.locator("#mods button[value=done]").click();
  await endOf(app, "She walked home.");
  await app.keyboard.type(' "hey dudes" He said.');
  const mark = app.locator(".cm-mark-rule");
  await expect(mark).toHaveCount(1, { timeout: 5000 });
  await mark.click();
  await app.locator(".cm-mark-card .cm-mark-suggestion").first().click();
  await expect.poll(() => textOf(app)).toContain('She walked home. "hey dudes," he said.');
  await expect(app.locator(".cm-mark-rule")).toHaveCount(0);
});
