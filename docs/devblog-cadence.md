# Worldbuilders devblog cadence

Status: binding (2026-05-26). Companion to Q511. Read this before writing a
new devblog post.

## Schedule

- **Frequency**: weekly, 52 posts/year, no skips pre-launch.
- **Day**: Tuesday.
- **Time**: 14:00 UTC (9 EST · 15 CET · 23 JST).
- **Word count**: 800–1,500 words for a standard post; up to 3,500 for the
  monthly "deep dive" (every 4th post).
- **Reading time**: ~6–9 minutes for standard posts.
- **Make-up rule**: a missed week is published within 7 days.

## Authoring

- MDX in `web/src/content/blog/*.mdx`.
- Frontmatter is Zod-validated by Astro Content Collections — see
  `web/src/content/config.ts` for the schema (title, date, tags, summary,
  author, heroImageAlt, draft, theme, latMarker).
- Filename convention: `YYYY-MM-DD-slug.mdx` so the directory sorts
  chronologically.
- Slugs are keyword-rich and stable forever. Never rename — 301-redirect
  instead.

## Editorial principles

1. **Show numbers and screenshots, not adjectives.** "Bake time dropped 47%"
   beats "bake is faster." Tables are encouraged.
2. **Show what didn't work.** Negative space teaches more than highlight
   reels. Use an aurora-amber callout for "ON FIRE" honest disclosures.
3. **Link primary sources.** OSM PRs, GitHub issues, JOSM-loadable way IDs.
4. **One action item per post.** Vote, try, file a bug, claim a tile.
5. **Be honest about scope.** If the renderer is a stub, say so. If the
   pipeline is single-tile, say so. Marketing copy is not allowed.
6. **Norwegian flavour allowed in headlines and intros.** Body is English.

## Aurora Atlas styling

- The blog defaults to `theme: paper` per post (long-form editorial
  surface). The index page inherits the dark `ink` default of the site.
- Use the Aurora Atlas callout pattern for callouts:

  ```html
  <aside class="callout callout-amber">
    <p class="callout-eyebrow mono">ON FIRE · honest disclosure</p>
    <p>Body text…</p>
  </aside>
  ```

- Tones: `callout-amber` for warnings / ON FIRE; `callout-green` for
  "this works"; `callout-violet` for rare/notable callouts.
- Code snippets get an aurora-green left border automatically.
- Never use Inter, Roboto, or default Tailwind sans — body is Manrope,
  headings Fraunces, code JetBrains Mono.

## Distribution

| Channel | T+ |
|---|---|
| worldbuilders.app/blog (canonical) | 0 |
| RSS 2.0 / Atom 1.0 / JSON Feed 1.1 | 0 |
| Discord #devblog webhook | +10 min |
| Buttondown newsletter | +30 min |
| Bluesky / Mastodon / X thread | +1 h |
| Reddit r/Worldbuilders | +2 h |

## Post template (copy-paste)

```mdx
---
title: "Devblog #N: <headline>"
date: 2026-MM-DDT14:00:00Z
tags: [tech, osm, tiles]
summary: "<140-char hook>"
author: rolf
heroImageAlt: "<alt text>"
latMarker: "LAT 62°28′ N"
theme: paper
---

## TL;DR

- Bullet 1
- Bullet 2
- Bullet 3

## The problem

...

## The approach

...

## What we tried that did not work

...

<aside class="callout callout-amber">
  <p class="callout-eyebrow mono">ON FIRE · honest disclosure</p>
  <p>What is still broken or stubbed.</p>
</aside>

## Numbers

| Metric | Value |
|---|---|

## What is next

...

## Discuss

Link to discussion channels.
```
