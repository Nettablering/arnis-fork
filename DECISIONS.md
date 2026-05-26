# DECISIONS.md — standing user decisions (binding, never forget)

> Read this at the start of every turn. It records what the user has decided about how this
> product is **built, deployed, and tested**, plus the **rationale** behind each. It survives
> session compaction (it lives on disk). New user input is **appended** here with a date and a
> why — never overwritten unless the user explicitly retracts it. A fix or test that contradicts
> a standing decision is a defect.

Format: `- [date] DECISION — rationale (basis)`

## Infrastructure & access
- [2026-05] Use ONLY the existing Hetzner production box (`178.104.48.160`) and the user's laptop
  (for `--chrome`). — Rationale: that is the only infrastructure the agent has access to.
- [2026-05] NEVER provision a new VPS / cloud server. "Cold machine" realism must come from a
  nested VM/microVM (Kata/QEMU/Firecracker) *inside* the existing box. — Rationale: no authority
  or budget to create new servers; nested VMs on the existing box are explicitly OK.
- [2026-05] This is the PRODUCTION box: cap every sandbox's CPU/memory and never start/stop/modify
  the live services or their data. — Rationale: testing must never harm the live application.
- [2026-05] NEVER touch other projects — do not read-for-mutation, modify, delete, or prune
  anything not labelled `converge=1`. On disk pressure: prune own scratch, else use a *reachable*
  tailnet node, else PAUSE and ask. — Rationale: other projects must remain untouchable; never
  free space destructively.
- [2026-05] Deploy/staging over Tailscale; staging subdomain provisioned via the Cloudflare API
  (full access). — Rationale: matches the user's existing deploy path.

## How work is decided
- [2026-05] FIX BY DEFAULT: implement every low-risk improvement, even marginal, and **regardless
  of effort** (a tiny gain worth two weeks of work still gets done). — Rationale: unlimited time
  and tokens; effort is never a reason to skip.
- [2026-05] The ONLY non-fix outcome is `deferred-risky` — material risk only (regression,
  security, scope into other systems), with the risk stated. — Rationale: low value/high effort
  are not valid skip reasons; only risk is.
- [2026-05] No token/iteration/€ budget gates completion. Only earned convergence + KPI targets +
  DEFINITION_OF_DONE.md. — Rationale: unlimited resources.

## Testing & simulation
- [2026-05] Test across the FULL environment matrix (multiple Docker images, Podman, gVisor, Kata,
  nested VM, laptop Chrome `--chrome`, headless Playwright chromium/firefox/webkit, DevTools MCP,
  curl, Windows-native). A method that fails is a finding. — Rationale: ensure ALL methods work,
  not just one convenient one.
- [2026-05] Personas must be COGNITIVELY DIVERSE and think UNLIKE the builder — span the
  kognitiv_profil matrix, include genuine contrarians who reject a core premise, generated only
  from what a visitor sees + independent signal. — Rationale: a closed AI loop otherwise produces
  testers that echo the builder; we want something closer to real, varied feedback.
- [2026-05] Simulated conversion/retention are hypotheses that RANK what to fix — never a forecast
  or validated demand. Real proof comes from real people/money/cohorts via VALIDATE.md, run in
  parallel. — Rationale: synthetic users cannot validate real demand.

## Reliability
- [2026-05] Health-monitor every agent/shell (timeouts + heartbeats + watchdog); the orchestrator
  must never hang waiting on a dead agent. — Rationale: long autonomous runs must not stall.
- [2026-05] The run must survive automatic session compaction by reconstructing state from disk
  every turn. — Rationale: compaction happens periodically and must never stop the work.

<!-- APPEND NEW USER DECISIONS BELOW, with date + rationale. Do not delete the above. -->

## Bake-queue (Q081) — shipped 2026-05-26
- [2026-05-26] Redis Streams is the bake-queue transport (cold lane `wb:bake.requests.cold` +
  hot lane `wb:bake.requests.hot`; consumer group `bake-workers`). — Rationale: zero new
  operational surface vs. NATS/RabbitMQ/Kafka; consumer groups + PEL + XAUTOCLAIM give at-least-
  once + crash recovery; throughput need (~10–50 msg/s peak) is 3+ orders below Redis Streams'
  capability. See `docs/grill/q081-bake-queue-technology.md`.
- [2026-05-26] ALL Worldbuilders Redis keys are prefixed `wb:` (streams, idempotency markers,
  any future SETs). — Rationale: shared Redis on hetzner-prod (`/srv/shared/redis/`) is multi-
  tenant; the prefix makes namespace collision structurally impossible without us having to
  read or modify any other project's keys. Worldbuilders agents NEVER touch `/srv/*`; operator
  hands over `WB_REDIS_URL` via the systemd env file (see BLOCKED/needs-human.md).
- [2026-05-26] The worker loop lives in ONE binary, `wb-bake-worker`, shipped by the
  `bake-queue` crate. `arnis-cli --consume-stream` re-execs that binary so we don't fork the
  consumer logic across two crates. Real arnis bake integration lands in Q082.

## Cache eviction (Q082) — shipped 2026-05-26
- [2026-05-26] Layered cache lives inside `bake-server` (`backend/bake-server/src/cache.rs`)
  as a `TileCache` trait + `LayeredCache` impl. Three tiers: PINNED (in-memory, never evict,
  ops-pinned) + HOT (in-memory LRU, default 10 000 entries) + COLD (on-disk, 30-day soft
  TTL, reuses the existing Q465 `<z>-<x>-<y>.json` layout). — Rationale: gives the
  bake-server a single chokepoint for hot/cold separation without standing up new storage
  surfaces; the on-disk tier is the same one workers already write to, so no migration is
  required. The full HOT-in-Redis variant from `q082-cache-eviction-policy.md` (db 2,
  db 0 WARM with `allkeys-lru`, Postgres TOAST COLD) is deferred until shared Redis +
  Postgres are wired into a converged staging — the in-process tier captures the
  eviction-policy ladder we needed today without taking on additional ops surface area.
- [2026-05-26] Pin/unpin mirror into the Redis sorted-set `wb:cache.hot.tiles`
  (`ZADD <unix-ts> <tile-id>` / `ZREM`). The Q083 preheat cron reads this set
  (`ZRANGEBYSCORE -inf +inf`) and enqueues each member into `wb:bake.requests.hot`. The
  cache module itself never writes to the bake stream — that boundary belongs to Q083 so
  preheat policy stays in one place. — Rationale: minimal coupling; Q083 can ship without
  reaching into the cache crate's internals.
- [2026-05-26] Admin endpoints (`POST/DELETE /v1/admin/cache/pin/{z}/{x}/{y}`,
  `GET /v1/admin/cache/stats`) authenticate with the same per-path HMAC scheme as
  `/v1/tile/*`. — Rationale: zero new auth surface — ops already hold the BAKE_HMAC_KEY
  to drive the existing tile endpoint, and the HMAC path is signed with the admin URL so
  signing a `/v1/tile/*` request cannot be replayed against the admin surface.
