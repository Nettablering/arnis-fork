# Worldbuilders SLOs — canonical reference

> Source of truth for service-level objectives. Numbers are locked by
> `docs/grill/q097-slos-tile-fetch-bake.md`. Modify here only when that grill
> is re-opened with a new decision.

## Scope and budget

- **Target availability:** 99.9% / 30 days. Error budget = **43.2 minutes / month**.
- **Burn-rate alerts** (Google SRE multi-window): fast-burn lookback **1 h** at
  14.4× (also evaluated against a 5 m window to suppress flap), slow-burn
  lookback **6 h** at 6×, ticket-only warn at 3× over 24 h.
- **Measurement window:** 30-day rolling for budget; 28-day rolling for SLI
  reports (matches Grafana's default panel range).
- **Per-region:** SLIs computed per region (`eu-nbg`, future) then aggregated.

## Primary SLOs

### SLO-1 — Cached tile fetch latency
- **Definition:** wall-clock from `GET /v1/tile/*` received to response sent,
  filtered to responses where `X-WB-Cache-Tier ∈ {pinned, hot}`.
- **Target:** p95 < **150 ms**.
- **Hard upper bound:** p99 < 500 ms.
- **SLI:** `histogram_quantile(0.95, sum by (le) (rate(wb_tile_fetch_duration_seconds_bucket{tier=~"pinned|hot"}[5m])))`.
- **Error budget formula:** good = requests with latency ≤ 150 ms; bad = requests
  exceeding. Budget = 0.05% over 28 days.

### SLO-2 — Cold tile bake latency
- **Definition:** wall-clock from XADD enqueue to manifest emit, filtered to
  `tier=cold`.
- **Target:** p95 < **8 s**.
- **Hard upper bound:** 12 s timeout, after which we return 202 + placeholder.
- **SLI:** `histogram_quantile(0.95, sum by (le) (rate(wb_bake_duration_seconds_bucket[5m])))`.
- **Error budget formula:** good = bakes ≤ 8 s; bad = bakes > 8 s.

### SLO-3 — API availability
- **Definition:** non-5xx responses on `/v1/tile/*` and `/v1/health`.
- **Target:** 99.9% / 30 days. Floor at 99.5%.
- **SLI:** `sum(rate(wb_api_requests_total{result!="5xx"}[5m])) / sum(rate(wb_api_requests_total[5m]))`.
- **Error budget formula:** budget_remaining = 1 − (5xx_count / total_count) − 0.001,
  expressed as fraction of 43.2 min / month.

## Secondary SLOs

### SLO-4 — Manifest size
- **Definition:** uncompressed manifest bytes at emit time.
- **Target:** p99 < **2 MB** per chunk. Hard cap 2.5 MB / chunk (split via
  manifest-schema chunking).
- **SLI:** `histogram_quantile(0.99, sum by (le) (rate(wb_manifest_size_bytes_bucket[5m])))`.

### SLO-5 — Cache hit ratio
- **Target:** > 95% on `/v1/tile/*` 28-day rolling.
- **SLI:** `sum(rate(wb_cache_hits_total[5m])) / (sum(rate(wb_cache_hits_total[5m])) + sum(rate(wb_cache_misses_total[5m])))`.

## Burn-rate alert table

| Burn rate | Time to exhaust 43.2 min budget | Alert action |
|-----------|---------------------------------|--------------|
| 14.4×     | 1 h                             | Page immediately (Pagerduty primary) |
| 6×        | 2.4 h                           | Page during business hours |
| 3×        | 5 h                             | Ticket warn (no page) |
| 1×        | 30 d                            | Normal operations |

PromQL templates live in `backend/observability/prometheus/wb-rules.yml`.

## Escalation policy

1. **Fast-burn page (14.4×)** → On-call (Rolf) gets paged. Acknowledge within
   5 min; mitigate within 30 min. Document in `docs/incidents/`.
2. **Slow-burn page (6×)** → On-call ticket. Investigate within 4 h.
3. **Warn (3×)** → Slack only. Triage next business day.
4. **Budget exhausted** → Trigger error-budget policy below.

## Error-budget policy (when monthly budget hits zero)

1. **Feature freeze.** No new feature deploys until next month or until
   sustained reliability improvements restore budget.
2. **Reliability investment.** Next week is allocated to closing the largest
   reliability gap revealed by the burn.
3. **Blameless post-mortem.** Document failure mode in `docs/post-mortems/`.
   Output: one concrete prevention action shipped within two weeks.
4. **Canary auto-disable.** `wb-deploy` integration (Q095) disables canary
   rollouts while in freeze.

## SLO vs SLA

These are **internal SLOs**. External SLA for premium customers is documented
separately (99.5% / month with credits for breach). Premium SLA is softer to
preserve internal reliability buffer.

## SLI catalogue (precise definitions)

| Metric | Type | Labels | Unit |
|--------|------|--------|------|
| `wb_tile_fetch_duration_seconds` | Histogram | tier, universe | seconds |
| `wb_bake_duration_seconds` | Histogram | universe | seconds |
| `wb_manifest_size_bytes` | Histogram | universe | bytes |
| `wb_cache_hits_total` | Counter | tier, universe | requests |
| `wb_cache_misses_total` | Counter | universe | requests |
| `wb_bake_failures_total` | Counter | universe, reason | bakes |
| `wb_api_requests_total` | Counter | endpoint, result | requests |

All instrumented in `backend/bake-server/src/metrics.rs`. Cardinality bounded
via `result ∈ {2xx, 4xx, 5xx, other}` and `tier ∈ {pinned, hot, cold}`. The
`universe` label is unbounded in principle but practically ≤ 10; downstream
Prometheus scrape config drops it via `metric_relabel_configs` if needed.
