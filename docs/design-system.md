# Worldbuilders design system — "Aurora Atlas"

Status: binding (2026-05-26). Companion to the binding DECISIONS.md entry
"Worldbuilders design system — Aurora Atlas". Subsequent frontend agents
(web companion, Roblox UI, marketing, brand) MUST read this file before
writing any frontend code.

## 1. Concept (one sentence)

Editorial atlas meets aurora-tinged game-layer: cartographic legitimacy
(this is real geography, real OSM ways) plus transformative magic (a
polar-light game layer atop). References: Stamen Maps × Monocle magazine ×
Studio Anti Norway × Nordic noir × the Northern Lights.

## 2. Direction picked (intentional, not gold-plated)

**Refined-minimalism-with-cartographic-precision.** Not bold maximalism.
Generous negative space, hairline rules, mono coordinate readouts, one
saturated colour (aurora-green) used sparingly for the moments that
matter. The bold move is the *restraint* — vs. the rejected Inter + neon
attempt and against the generic AI-slop default.

## 3. Default mode

**Dark (ink)** for the launch site, plot browser, hub — leans game-y.
Light (paper) mode opts in via `<html data-theme="paper">` for editorial
surfaces (long-form blog posts, press kit detail pages once they ship).

## 4. Typography

| Role     | Family          | Notes |
| -------- | --------------- | ----- |
| Display  | Fraunces        | Variable serif. Use opsz 144 + SOFT 30–100 axes. Italic for "becomes". |
| Body     | Manrope         | Variable, weight 200–800. Default 400. Distinctive geometric humanist. |
| Mono     | JetBrains Mono  | Coordinates, code, eyebrows, tags, status pills, lat-band labels. |

Hard rejections: Inter, Roboto, system-ui, Open Sans, Arial, Space Grotesk,
default Tailwind sans.

### Type scale (CSS, fluid)
- h1 — `clamp(2.6rem, 6.4vw, 5.2rem)`, opsz 144, SOFT 50, wght 420, ls -0.022em
- h2 — `clamp(1.6rem, 3vw, 2.4rem)`, opsz 144, SOFT 30
- lede — 1.04–1.08rem, color `--fg-soft`, max-width 56–64ch
- body — 1rem, line-height 1.55, max-width 62ch
- eyebrow — JetBrains Mono uppercase 0.72rem ls 0.22em, prefixed by a 1.6rem rule line

## 5. Colour tokens — "Polar Atlas"

All tokens declared in `src/styles/global.css` under `@theme`. NEVER use
arbitrary hex in components.

### Ink (dark — default)
| Token              | Value     | Usage |
| ------------------ | --------- | ----- |
| `--color-ink-bg`         | `#0D0F14` | page background |
| `--color-ink-surface`    | `#161B26` | cards, waitlist input bg |
| `--color-ink-elevated`   | `#1F2638` | popovers, hover surfaces |
| `--color-paper-on-ink`   | `#EAE3D2` | body text on dark |
| `--color-rule-on-ink`    | `#2A3142` | hairlines |

### Paper (light — opt-in)
| Token         | Value     | Usage |
| ------------- | --------- | ----- |
| `--color-paper`     | `#F3EEE5` | parchment background |
| `--color-ink`       | `#0D0F14` | text on light |
| `--color-ink-soft`  | `#3A4053` | secondary text on light |
| `--color-rule`      | `#C9C0AE` | hairlines on light |
| `--color-map-line`  | `#7B8190` | contour stroke, map dots |

### Aurora accents (sparingly — never gradient soup)
| Token                   | Value     | Usage |
| ----------------------- | --------- | ----- |
| `--color-aurora-green`  | `#5BE9B9` | primary CTA, claim pulse, success, "live" |
| `--color-aurora-violet` | `#9D6FFF` | rare/mythic landmarks, premium currency |
| `--color-aurora-amber`  | `#FFB347` | warnings, "ON FIRE" leaderboard markers |

### Semantic surface tokens (theme-aware — use these in components)
`--bg`, `--bg-elevated`, `--bg-popped`, `--fg`, `--fg-soft`, `--fg-faint`,
`--rule`, `--map-line`. They flip automatically between ink/paper modes.

### Aurora-green usage rule
Use aurora-green for **the moment of claim** and small "live" indicators
only. NEVER fill large areas with it. Never gradient it into violet/amber.
A page may have at most ~3 instances of full-saturation aurora-green
above the fold.

## 6. Motifs (binding)

1. **Hand-drawn cartographic contour lines** — SVG, `feTurbulence`
   displacement for roughness, 0.16–0.22 opacity. Renders behind the hero.
2. **Paper grain noise overlay** — Base64 SVG noise, `mix-blend-mode:
   overlay`, 3.5% (paper) / 6% (ink) opacity. Applied via `body::before`.
3. **Asymmetric compass rose** — north arm aurora-green, south arm gradient
   to violet. Spins in on page load. Component: `CompassRose.astro`.
4. **Latitude/longitude tick marks** — repeating-linear-gradient on the
   four page edges. Coordinate readouts at the four corners.
5. **Halftone dot patterns** — `.halftone` utility for emphasis blocks.
6. **Latitude band rule** — `.lat-band` wraps each major section with top
   + bottom hairlines and a mono "LAT BAND 62°N · …" label that sits
   inset into the top rule.

## 7. Anti-patterns (reject on sight)

- Inter / Roboto / system-ui display
- Neon green (#39FF8A) + deep navy (#0B1530) — the rejected v1 palette
- Purple-to-pink gradients
- Glassmorphism cards floating on solid colour
- Bootstrap-style cards with rounded corners + drop shadows
- Stock vector mascots, illustrated people
- Pre-baked Tailwind UI templates without heavy customisation
- "Coming soon" lozenge in default Tailwind amber
- Slack/Discord-style screenshot mosaics in hero

## 8. Motion primitives

| Primitive          | Trigger        | CSS keyframe         | Duration |
| ------------------ | -------------- | -------------------- | -------- |
| Fade-up reveal     | data-anim="fade-up" | `wb-fade-up`    | 900 ms easeOutCubic |
| Compass spin-in    | data-anim="spin-in" | `wb-spin-in`    | 1.4 s easeOutCubic |
| Contour draw-in    | `.contour path` stroke-dashoffset | `wb-draw` | 2.2 s ease-out |
| Aurora claim pulse | `.aurora-pulse` | `wb-pulse-aurora`   | 1.8 s, infinite |
| Compass spinner    | `.compass-spinner` | `wb-needle-spin` | 1.6 s linear, infinite |
| Hover halo on CTA  | `.btn-aurora:hover` | (box-shadow tx)| 280 ms easeOutCubic |

`prefers-reduced-motion: reduce` collapses all of the above to fade-only
(1 ms duration). See global.css.

## 9. Layout primitives

- `.wrap` — 1200px max, fluid horizontal padding `clamp(1.2rem, 4vw, 3rem)`
- `.wrap-narrow` — 760px for prose-heavy sections
- `.atlas-grid` — 7fr / 5fr asymmetric (collapses to 1fr at 880px)
- `.lat-band` — top + bottom hairlines, vertical rhythm
  `clamp(3rem, 8vw, 6rem)`, with `data-lat="…"` text inset into top rule
- `.atlas-frame` — body class; paints fixed tick marks on the four edges
- `.atlas-coord.tl/.tr/.bl/.br` — coordinate readouts at the corners

## 10. Component inventory (v1)

| Path                                  | Purpose | Used by |
| ------------------------------------- | ------- | ------- |
| `layouts/Atlas.astro`                 | Page shell — frame, ticks, coords, nav, footer | every page |
| `components/NavAtlas.astro`           | Top nav — wordmark + asymmetric compass-dot lockup | Atlas layout |
| `components/CompassRose.astro`        | Asymmetric SVG compass; spins in | hero, press |
| `components/ContourBackground.astro`  | Stylised cartographic contour SVG, draws in | hero |
| `components/AuroraPulse.astro`        | The one memorable moment — green pulse + label | hero, status |
| `components/WaitlistForm.astro`       | Pill-shaped email capture + aurora CTA | hero, final CTA |

### Page-screen descriptions (textual, for reviewers)

**Homepage (`/`)** — dark page. Top edge: repeating tick marks with
mono "N 62°28′ 22.6″" at the top-left corner. Below, hairline-bordered
nav: asymmetric compass-dot wordmark on the left, mono uppercase nav
links on the right. Hero opens with an aurora-green pulse dot beside the
text "PRE-LAUNCH · ÅLESUND TILE BAKE LIVE". Then a Fraunces-italic /
roman composition: "Your hometown / *becomes* / a game." (last line
aurora-green). To the right of the headline, a 260 px asymmetric
compass rose with an aurora-green/violet gradient needle spins in on
load. Behind everything, three concentric topographic contour rings
draw themselves in over 2 s. Below the headline, a pill-shaped waitlist
input ending in a green "Claim your hometown →" button that haloes on
hover. Below: three mono numerals listing live bake status. Scroll past
the first lat-band rule and you see the tile inspector: a real OSM
mini-tile rendered as SVG (grid, roads, violet building footprints, and
a single aurora-green pulsing claim ring on one landmark). Three feature
cards follow, each labelled with a mono "01 · REAL" tag inset into the
hairline border. Then the pipeline section (four steps with aurora-green
left borders), and a final CTA latitude band.

**Press (`/press`)** — same shell. Hero is split: factsheet eyebrow +
"Press kit" Fraunces headline on the left, a 180 px CompassRose on the
right. Two-column factsheet dl below with mono labels. Asset cards in a
2-col grid with violet "PNG · 2400×1260" mono tags.

**Developers (`/developers`)** — JSON manifest snippet in a mono code
block with an aurora-green left border. Endpoint table with status pills
(aurora-green "live" with subtle halo, amber for "Q443"-pending).

**Status (`/status`)** — aurora-pulse eyebrow + "all systems nominal".
Service list with dot indicators (green / amber). 2×2 SLO grid using
Fraunces for the target numbers.

**Devblog (`/blog`)** — empty state foregrounded: a slowly-rotating
compass-rose SVG with mono "// awaiting tile bake" caption.

## 11. Implementation contract

Subsequent frontend agents MUST:
1. Read this file before writing any frontend code.
2. Pull Fraunces + Manrope + JetBrains Mono via `@font-face` from Google
   Fonts (already done in `global.css`).
3. Use the listed CSS variables. No arbitrary hex.
4. Include `.atlas-frame`, contour SVG, and grain texture on hero
   surfaces.
5. Treat the aurora-green pulse as a reusable component, not a one-off.
6. Honour `prefers-reduced-motion: reduce` in every new animation.
7. Never reach for any item in §7 Anti-patterns.

Violations are findings the verifier-agent must reject.
