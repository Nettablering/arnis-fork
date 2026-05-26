import { test, expect } from "@playwright/test";

// Q511 — devblog assertions. The blog must list ≥1 post, render the
// individual post page, expose feeds in <head>, and keep Aurora Atlas
// tokens (no Inter / Tailwind default fallback).

test.describe("Q511 devblog", () => {
  test("blog index returns 200 and lists the first post", async ({ page }) => {
    const resp = await page.goto("/blog/");
    expect(resp?.status(), "/blog/ HTTP status").toBeLessThan(400);
    await expect(page).toHaveTitle(/devblog/i);
    // At least one post on the index — the Aksla bake post.
    await expect(page.locator("article, li.post").first()).toBeVisible();
    await expect(page.getByRole("heading", { level: 2 }).first()).toContainText(/aksla|tile|bake/i);
  });

  test("blog post page renders the Aksla post with Fraunces title", async ({ page }) => {
    const resp = await page.goto("/blog/2026-05-26-pre-launch-aksla-bake/");
    expect(resp?.status(), "post HTTP status").toBeLessThan(400);

    await page.evaluate(async () => { await (document as any).fonts.ready; });
    const h1 = page.locator("h1.post-title").first();
    await expect(h1).toContainText(/aksla|baked|tile/i);
    const fontFamily = await h1.evaluate((el) => window.getComputedStyle(el).fontFamily);
    expect(fontFamily.toLowerCase()).toContain("fraunces");
    expect(fontFamily.toLowerCase()).not.toMatch(/^inter\b/);

    // Body text must be Manrope (Aurora Atlas), not generic sans.
    const bodyFont = await page.locator(".post-prose p").first().evaluate((el) =>
      window.getComputedStyle(el).fontFamily,
    );
    expect(bodyFont.toLowerCase()).toContain("manrope");
  });

  test("RSS feed is valid XML at /blog/rss.xml", async ({ request }) => {
    const r = await request.get("/blog/rss.xml");
    expect(r.status()).toBe(200);
    expect(r.headers()["content-type"]).toMatch(/rss\+xml|xml/);
    const body = await r.text();
    expect(body).toContain('<?xml');
    expect(body).toContain('<rss version="2.0"');
    expect(body).toMatch(/<title>Worldbuilders devblog<\/title>/);
    expect(body).toMatch(/<item>[\s\S]*<\/item>/);
  });

  test("Atom + JSON Feed both ship", async ({ request }) => {
    const atom = await request.get("/blog/atom.xml");
    expect(atom.status()).toBe(200);
    expect(await atom.text()).toContain('<feed xmlns="http://www.w3.org/2005/Atom"');

    const json = await request.get("/blog/feed.json");
    expect(json.status()).toBe(200);
    const feed = await json.json();
    expect(feed.version).toContain("jsonfeed.org/version/1.1");
    expect(feed.items.length).toBeGreaterThan(0);
  });

  test("feed autodiscovery links are present in <head>", async ({ page }) => {
    await page.goto("/blog/");
    await expect(page.locator('link[rel="alternate"][type="application/rss+xml"]')).toHaveAttribute("href", /rss\.xml/);
    await expect(page.locator('link[rel="alternate"][type="application/atom+xml"]')).toHaveAttribute("href", /atom\.xml/);
    await expect(page.locator('link[rel="alternate"][type="application/feed+json"]')).toHaveAttribute("href", /feed\.json/);
  });
});
