# Worldbuilders web companion

Interim canonical: <https://worldbuilders.quicktoolry.com> (per [Q503](../docs/grill/q503-website-domain-strategy-app-vs-subdomain.md)).
Launch canonical: <https://worldbuilders.app>.

Stack: Astro 4 (static), Tailwind v4, MDX, Playwright + axe-core + Lighthouse CI.

## Layout

- `src/pages/` — `/`, `/press`, `/developers`, `/blog/`, `/status`
- `src/layouts/Base.astro` — shared header/footer + a11y skip-link
- `src/styles/global.css` — Tailwind v4 + brand tokens (neon-green / deep-blue / Inter)
- `public/logo.svg` — placeholder brand mark (Q258 will replace)
- `tests/e2e/homepage.spec.ts` — smoke + waitlist + axe-core a11y
- `scripts/deploy.sh` — build + rsync into `../web-deploy/releases/<ts>/`, swap `../web-deploy/current` symlink
- `scripts/worldbuilders.caddy` / `worldbuilders.nginx.conf` — operator vhost snippets
- `lighthouserc.json` — Lighthouse budgets (Q520 will raise to 100/100/100/100)

## Local

```
npm install
npm run build           # -> dist/
npm run preview         # serves dist/ on http://127.0.0.1:4321
npm run test:e2e        # spins preview + runs Playwright + axe
```

## Deploy (hetzner-prod)

```
bash scripts/deploy.sh
```

Writes to `/home/deploy/projects/worldbuilders/web-deploy/releases/<ts>/` and swaps the
`web-deploy/current` symlink. Operator installs the Caddy/nginx vhost once (see
`BLOCKED/needs-human.md` — "Worldbuilders web companion vhost").
