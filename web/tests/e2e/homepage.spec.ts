import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Q325-redesign — Aurora Atlas design system assertions.
// See /docs/design-system.md for the binding spec.

test.describe("Aurora Atlas homepage (Q325-redesign)", () => {
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

  test("has the Aurora Atlas waitlist form (pill-shaped, aurora CTA)", async ({ page }) => {
    await page.goto("/");
    const form = page.locator("form#waitlist");
    await expect(form).toBeVisible();
    await expect(form.locator('input[type="email"]')).toBeVisible();
    await expect(form.locator('button[type="submit"]')).toBeVisible();
    // CTA copy is Aurora Atlas-specific, not generic "Join waitlist"
    await expect(form.locator('button[type="submit"]')).toContainText(/claim/i);
  });

  test("hero headline uses the Aurora Atlas 'becomes a game' composition", async ({ page }) => {
    await page.goto("/");
    const h1 = page.locator("h1").first();
    await expect(h1).toContainText(/your hometown/i);
    await expect(h1).toContainText(/becomes/i);
    await expect(h1).toContainText(/game/i);
  });

  test("Fraunces display font is loaded for the h1", async ({ page }) => {
    await page.goto("/");
    // Wait for fonts to be ready so the computed style picks up the webfont.
    await page.evaluate(async () => {
      await (document as any).fonts.ready;
    });
    const fontFamily = await page.locator("h1").first().evaluate((el) =>
      window.getComputedStyle(el).fontFamily,
    );
    expect(fontFamily.toLowerCase()).toContain("fraunces");
    // Rejected fonts MUST NOT appear in the resolved stack root.
    expect(fontFamily.toLowerCase()).not.toMatch(/^inter\b/);
    expect(fontFamily.toLowerCase()).not.toMatch(/^roboto\b/);
  });

  test("compass rose component renders", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("svg.compass-rose").first()).toBeVisible();
  });

  test("paper-grain noise overlay is present", async ({ page }) => {
    await page.goto("/");
    const grain = await page.evaluate(() => {
      const before = window.getComputedStyle(document.documentElement, "::before");
      return {
        bg: before.backgroundImage,
        opacity: before.opacity,
      };
    });
    // Inline base64 SVG with feTurbulence is the binding signature.
    expect(grain.bg.toLowerCase()).toContain("turbulence");
  });

  test("cartographic contour background SVG is present in hero", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("svg.contour").first()).toBeAttached();
  });

  test("atlas frame ticks + coordinate readouts render", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".atlas-side-ticks.left")).toBeAttached();
    await expect(page.locator(".atlas-coord.tl")).toContainText(/N 62/);
  });

  test("rejected v1 palette tokens are absent from the page", async ({ page }) => {
    await page.goto("/");
    // The rejected hex codes from the Q325 v1 attempt must not appear in
    // any computed background — sample the body + hero.
    const samples = await page.evaluate(() => {
      const bodyBg = window.getComputedStyle(document.body).backgroundColor;
      const heroBg = window.getComputedStyle(document.querySelector(".hero") as Element).backgroundColor;
      return { bodyBg, heroBg };
    });
    // #0B1530 = rgb(11, 21, 48), #39FF8A = rgb(57, 255, 138)
    const rejected = ["rgb(11, 21, 48)", "rgb(57, 255, 138)"];
    for (const r of rejected) {
      expect(samples.bodyBg).not.toBe(r);
      expect(samples.heroBg).not.toBe(r);
    }
  });

  test("axe-core accessibility pass (no serious/critical violations)", async ({ page }) => {
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

  test("nav links to /press /developers /blog /status all 200", async ({ page, request }) => {
    for (const path of ["/press", "/developers", "/blog", "/status"]) {
      const r = await request.get(path);
      expect(r.status(), `${path} status`).toBeLessThan(400);
    }
  });
});
