# BLOCKED — needs human action

## 2026-05-26 — Q469 finish: real iPhone SE 2 device test (Studio MCP unavailable)

**Q-id:** Q469 (build-step-7-mobile-test-iphone-se)
**Status:** scaffolding green; real-device run deferred until Studio MCP lands
or Rolf has an iPhone SE 2 in hand.

**What agent shipped (all under `/home/deploy/projects/worldbuilders/`):**

- `game/mobile/README.md` — manual test procedure (publish flow, scenes,
  frame targets, screenshot + video capture, audit MD publish).
- `game/mobile/iphone-se-emulation.lua` — Lune harness that asserts the same
  invariants as a real iPhone SE 2 run (FPS p50 >= 28 anchor scenario,
  instance count <= 5000, zero errors). Drives the same
  `game/perf/iphone-se.budget.toml` as Q478.
- `game/mobile/expected-screenshots.md` — textual baseline of what each scene
  should look like, ready for Q477 visual-regression to diff once real PNGs
  exist.
- `game/mobile/selene.toml` + `lune.yml` + `stylua.toml` — per-dir tooling
  config (selene clean, stylua clean).
- `.github/workflows/mobile-emulation-ci.yml` — runs the harness on every PR.

**What needs a human:**

1. Publish `wb-dev-01` from Studio (File -> Publish to Roblox under Klokk
   Studios group). Record place ID in `docs/place-ids.md`. Requires an
   interactive Studio session; no MCP path available today.
2. Sit with a real iPhone SE 2nd gen (battery health >= 85%, iOS 17+),
   plug into MacBook via Lightning, mirror via QuickTime, and walk every
   scene in `game/mobile/README.md` section 4.
3. Write the audit MD at `docs/mobile-audits/iphone-se-2nd-<YYYY-MM-DD>.md`
   following the template in
   `docs/grill/q469-build-step-7-mobile-test-iphone-se.md`. Commit
   screenshots into `docs/mobile-audits/img/`; attach the master video as a
   release asset (not committed — size).
4. Flip the real-device gate in
   `.github/workflows/mobile-emulation-ci.yml` (`real-device-gate` job,
   change `if: false` to match the runner's availability condition).

**No secrets involved.** The harness reads only the public budget file.

**Trigger to unblock:** either Roblox Studio MCP becomes available on
hetzner-prod, or Rolf acquires an iPhone SE 2 (used, ~150 EUR) and a MacBook
mirror. Whichever lands first.

---

## 2026-05-26 — Q470 finish: install bake-server systemd unit + reverse-proxy vhost

**Q-id:** Q470 (build-step-8-hetzner-deploy-staging)
**Status:** binary built + smoke-tested locally; DNS live; needs sudo install for end-to-end HTTPS

**What agent shipped:**
- `/home/deploy/projects/worldbuilders/backend/bake-server/` — stand-alone Rust Axum crate
  (`cargo build --release` green; HMAC verify + 200/401 paths smoke-tested on 127.0.0.1:9090)
- `/home/deploy/projects/worldbuilders/backend/bake-server.service` — systemd unit (User=deploy)
- `/home/deploy/projects/worldbuilders/backend/Caddyfile.staging` — Caddy snippet (spec'd shape)
- `/home/deploy/projects/worldbuilders/backend/nginx.staging.conf` — nginx equivalent
  (hetzner-prod actually runs nginx, not Caddy, per /srv/shared/nginx/)
- `/home/deploy/projects/worldbuilders/backend/scripts/deploy-staging.sh` — build+restart
- `.env` with fresh 32-byte HMAC key at `bake-server/.env` (0600, deploy:deploy)
- Cloudflare DNS A-record `staging.worldbuilders.quicktoolry.com → 178.104.48.160` (proxied)
  via CLOUDFLARE_API_TOKEN; DNS resolves through CF edge (verified with `dig`).

**What needs sudo (one-time):**

1. Install systemd unit:
   ```
   sudo cp /home/deploy/projects/worldbuilders/backend/bake-server.service \
           /etc/systemd/system/bake-server.service
   sudo systemctl daemon-reload
   sudo systemctl enable --now bake-server
   sudo systemctl status bake-server
   ```

2. Install reverse-proxy vhost — pick ONE:
   - **nginx (matches current host)**: copy `backend/nginx.staging.conf` to
     `/srv/shared/nginx/vhosts/worldbuilders-bake-staging.conf`, issue Let's Encrypt
     cert (`sudo certbot certonly --webroot -w /srv/shared/nginx/wellknown -d staging.worldbuilders.quicktoolry.com`),
     then `sudo nginx -t && sudo systemctl reload nginx`.
   - **Caddy (per spec)**: only if Caddy is installed; `backend/Caddyfile.staging` is ready
     to `import` from /srv/shared/caddy/Caddyfile.

3. Verify end-to-end:
   ```
   curl -sf https://staging.worldbuilders.quicktoolry.com/v1/health
   # expect 200 {"status":"ok",...}
   ```

**HMAC key location:** `/home/deploy/projects/worldbuilders/backend/bake-server/.env` (0600).
Same key needs mirroring into Roblox HttpSecretService once the Studio integration lands
(Q088). Do NOT commit `.env`.

---


<!--
Q463 (arnis-fork-bootstrap) RESOLVED 2026-05-26T08:05Z.
PAT now provisioned at ~/.claude/shared/api-keys/github-nettablering.key.
Fork https://github.com/Nettablering/arnis-fork pushed (main @ f4cf212).
Workspace reshape (arnis-core / arnis-emitters / arnis-cli) green via
`cargo build --workspace --release --no-default-features`. CI workflow
shipped at .github/workflows/ci.yml. See COMPLETION-LEDGER.md.
-->

## 2026-05-26 — Q477: visual regression baselines need real capture path

**Q-id:** Q477 (verification-visual-regression-screenshots)
**Status:** scaffolding shipped + CI wired + self-test green; baselines/ deliberately empty

**What agent shipped:**
- `game/visual/compare.py` — pure-Python pixelmatch-style diff (YIQ delta, red diff PNG)
- `game/visual/scenes.yaml` — 11 scenes x 4 devices x 3 themes manifest with per-scene tolerances
- `game/visual/baselines/` — empty (only `.gitkeep` with naming convention)
- `game/visual/README.md` — capture + review workflow
- `.github/workflows/visual-regression-ci.yml` — runs self-test + sweep, blocks merge on diff > 0.5%
- Self-tests verified: `--self-test` exits 0 (identical), `--self-test --inject-diff` exits 1 (diff detected)

**What is blocked:**
Real golden screenshots cannot land until ONE of:
1. **Roblox Studio MCP** server providing the headless `Capture` service over MCP, OR
2. **Web companion** at `worldbuilders.quicktoolry.com` shipping (per Q325) so Playwright can shoot browser surfaces.

Until then the CI sweep gracefully exits 0 for every (scene x device x theme) — the
`compare_paths()` path treats a missing baseline as "no truth yet, skip" rather than a
regression. The diff math itself is exercised on every CI run via `--self-test`.

**Resolves when:** Either upstream lands. At that point the runner writes PNGs to
`game/visual/captures/<scene>__<device>__<theme>.png`, a human reviews + promotes to
`baselines/`, and the sweep starts enforcing tolerances.

---

## 2026-05-26 — Q295/Q472 finish: install lune + selene + stylua on hetzner-prod

**Q-ids:** Q295 (luau-code-quality), Q472 (verification-unit-tests-luau)
**Status:** configs + mocks + test harness shipped; runtime binaries missing on dev host

**What agent prepared:**
- `game/selene.toml`, `game/stylua.toml`, `game/testez.yml`
- `game/tests/{init.lua, example.spec.lua, mocks/HttpService.lua, mocks/DataStoreService.lua, mocks/MemoryStoreService.lua}` — all Studio-faithful
- `game/scripts/test.sh` (chmod +x)

**What needs to happen:**
- Install user-mode (no sudo): aftman, then `aftman add lune selene stylua`
- Or use rokit (already installed by Q466) — verify it knows about lune/selene/stylua and run `rokit install` in `game/`
- Run `bash game/scripts/test.sh` and verify all green

**Note:** GitHub Actions CI bypasses this — `setup-aftman` action handles it. Local dev on hetzner-prod just needs one-time install. Low-friction.

---

## 2026-05-26 — Q081 finish: install wb-bake-worker.service + Redis credentials

**Q-id:** Q081 (bake-queue-technology)
**Status:** code shipped (`backend/bake-queue/` crate + bake-server 202-on-miss + arnis-cli `--consume-stream`); systemd unit drafted; needs human to install + provide Redis URL.

**What agent prepared:**
- `backend/bake-queue/` — producer + consumer + mock Redis + 15 unit tests + 2 integration tests (all green via `cargo test`).
- `backend/bake-queue/wb-bake-worker.service` — systemd unit, NOT installed.
- bake-server now reads `WB_REDIS_URL` / `WB_STYLE_VERSION` / `WB_OSM_SNAPSHOT`; on cache-miss enqueues + returns 202 with Retry-After.

**What needs to happen (one-time, on hetzner-prod, as root):**
1. Decide which Redis instance Worldbuilders is allowed to share with. Options:
   - The existing `shared-redis` container on the `edge` Docker network (`/srv/shared/redis/compose.yml`, password in `/srv/shared/secrets/redis.env`). The Worldbuilders agent **cannot** read `/srv/*` so the operator must hand the URL over — e.g. publish `127.0.0.1:6379` from the container or expose a dedicated `worldbuilders` ACL user. All Worldbuilders keys are prefixed `wb:` so namespace collision is structurally avoided (see DECISIONS.md).
   - Or stand up a dedicated Worldbuilders Redis (recommended once volume > preheat trial).
2. Write the URL to `/etc/worldbuilders/wb-bake-worker.env` as `WB_REDIS_URL=redis://[:pass@]host:port/db`, `WB_CACHE_DIR=/home/deploy/projects/worldbuilders/backend/cache/manifests`, `WB_WORKER_NAME=worker-prod-1`.
3. Install + enable:
   ```
   cp /home/deploy/projects/worldbuilders/backend/bake-queue/wb-bake-worker.service /etc/systemd/system/
   systemctl daemon-reload
   systemctl enable --now wb-bake-worker.service
   ```
4. Restart bake-server with `WB_REDIS_URL` exported so it switches from 503 to 202 on cache miss.

**Resolves when:** systemd unit is running and `journalctl -u wb-bake-worker` shows the consumer loop starting; bake-server logs `bake-queue producer configured`.

---

## 2026-05-26 — Q325 finish: install web companion vhost on Caddy/nginx

**Q-id:** Q325 (web-companion-site-architecture)
**Status:** Astro static site built + Playwright/axe/Lighthouse green. DNS for
`worldbuilders.quicktoolry.com` already points to hetzner-prod (proxied via
Cloudflare, A 178.104.48.160). Operator needs to install the reverse-proxy
vhost so the origin actually serves the new `web-deploy/current/` directory.
A live curl to https://worldbuilders.quicktoolry.com/ currently returns 520
(Cloudflare cannot reach the origin — no vhost wired yet).

**What agent prepared:**
- `/home/deploy/projects/worldbuilders/web/` — Astro 4 project; `npm run build`
  emits `dist/`.
- `/home/deploy/projects/worldbuilders/web/scripts/deploy.sh` — builds + rsyncs
  to `../web-deploy/releases/<ts>/`, atomically swaps `../web-deploy/current`.
- `/home/deploy/projects/worldbuilders/web/scripts/worldbuilders.caddy` — Caddy
  vhost snippet (preferred — matches Q470 pattern).
- `/home/deploy/projects/worldbuilders/web/scripts/worldbuilders.nginx.conf` —
  nginx alternative.

**What needs to happen (one-time, on hetzner-prod, as root):**
1. Run `bash /home/deploy/projects/worldbuilders/web/scripts/deploy.sh` once as
   the `deploy` user so `web-deploy/current` resolves.
2. Copy the Caddy snippet into the shared Caddyfile:
   ```
   cp /home/deploy/projects/worldbuilders/web/scripts/worldbuilders.caddy \
      /srv/shared/caddy/sites/worldbuilders.caddy
   caddy reload --config /srv/shared/caddy/Caddyfile
   ```
   (Or paste contents into the global Caddyfile; agent cannot touch /srv/*.)
3. Verify: `curl -sSI https://worldbuilders.quicktoolry.com/ | head -3` returns
   `HTTP/2 200` and `content-type: text/html`.

**Resolves when:** the homepage HTML is served by Caddy/nginx (200 OK) and the
`/press`, `/developers`, `/blog`, `/status` paths return 200 as well.
