import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

test.describe("homepage smoke (Q325)", () => {
  test("loads with no console errors", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
    page.on("console", (msg) => {
      if (msg.type() === "error") errors.push(`console.error: ${msg.text()}`);
    });

    const resp = await page.goto("/");
    expect(resp?.status(), "homepage HTTP status").toBeLessThan(400);

    await expect(page).toHaveTitle(/worldbuilders/i);
    expect(errors, "no console / page errors").toEqual([]);
  });

  test("has a waitlist email-capture form", async ({ page }) => {
    await page.goto("/");
    const form = page.locator("form#waitlist");
    await expect(form).toBeVisible();
    await expect(form.locator('input[type="email"]')).toBeVisible();
    await expect(form.locator('button[type="submit"]')).toBeVisible();
  });

  test("axe-core accessibility pass (no serious/critical violations)", async ({
    page,
  }) => {
    await page.goto("/");
    const results = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
      .analyze();
    const blocking = results.violations.filter((v) =>
      ["serious", "critical"].includes(v.impact ?? ""),
    );
    expect(
      blocking,
      `axe violations: ${JSON.stringify(blocking, null, 2)}`,
    ).toEqual([]);
  });

  test("nav links to /press /developers /blog /status all 200", async ({
    page,
    request,
  }) => {
    for (const path of ["/press", "/developers", "/blog", "/status"]) {
      const r = await request.get(path);
      expect(r.status(), `${path} status`).toBeLessThan(400);
    }
  });
});
