# Observability — hetzner-prod attachment guide

> Bake-server exports Prometheus metrics on `GET /metrics` (no auth — bind it
> to loopback or behind the shared scrape ingress on hetzner-prod). This doc
> tells Rolf how to wire the existing shared Prometheus + Grafana stack at
> `/srv/shared/` to the bake-server. Agents must **never modify /srv/\***;
> snippets below are copy-paste ready.

## 1. Prometheus scrape config

Add this scrape job to `/srv/shared/prometheus/conf.d/wb.yml`:

```yaml
scrape_configs:
  - job_name: worldbuilders_bake
    metrics_path: /metrics
    scrape_interval: 15s
    static_configs:
      - targets: ["127.0.0.1:9090"]
        labels:
          service: bake-server
          region: eu-nbg
    # Drop high-cardinality universe label if the count explodes (>50).
    # metric_relabel_configs:
    #   - source_labels: [universe]
    #     regex: ".+"
    #     target_label: universe
    #     replacement: aggregate
```

If bake-server binds elsewhere, replace `127.0.0.1:9090` with that
`host:port`. The shared Prometheus must be able to reach the address (loopback
works because both run on hetzner-prod).

## 2. Install the rules file

Copy `backend/observability/prometheus/wb-rules.yml` to
`/srv/shared/prometheus/conf.d/wb-rules.yml` and ensure `prometheus.yml`
includes that conf.d glob:

```yaml
rule_files:
  - /etc/prometheus/conf.d/*.yml
```

Then `promtool check rules /srv/shared/prometheus/conf.d/wb-rules.yml` and
reload (`kill -HUP $(pidof prometheus)` or via systemd).

## 3. Import the Grafana dashboard

In the shared Grafana, **Dashboards → Import → Upload JSON** and pick
`backend/observability/grafana/wb-slo-dashboard.json`. The dashboard expects a
Prometheus datasource with UID `prometheus` (the shared default). If your
shared Grafana uses a different UID, edit the JSON's `datasource.uid` fields
or override at import time.

The dashboard has 4 panels:

1. SLO-1 — Tile fetch p95 by tier
2. SLO-2 — Cold bake p95
3. SLO-4 — Manifest p99 size
4. SLO-3 — Error budget remaining (30 d)

## 4. Alertmanager routing

The rules emit three severity labels:

- `severity=page` → primary Pagerduty (Rolf), any-hour.
- `severity=page-bizhours` → Pagerduty business-hours-only schedule.
- `severity=warn` → Slack only, no page.

Existing shared Alertmanager already routes by `severity`; no new routes are
needed unless the shared config lacks `page-bizhours`. Snippet for that case:

```yaml
route:
  routes:
    - matchers: [severity="page-bizhours"]
      receiver: pagerduty_bizhours
```

## 5. Verifying the wire-up

After install, on hetzner-prod:

```bash
# Bake-server is exposing metrics
curl -s http://127.0.0.1:9090/metrics | grep -c '^wb_'

# Prometheus is scraping
curl -s http://127.0.0.1:9091/api/v1/targets | jq '.data.activeTargets[] | select(.labels.job=="worldbuilders_bake")'

# Rules are loaded
curl -s http://127.0.0.1:9091/api/v1/rules | jq '.data.groups[] | select(.name | startswith("wb_"))'
```

Ports above match the typical hetzner-prod layout; substitute as needed.

## 6. Load test (k6 conformance check)

Run from the deploy host once metrics are wired:

```bash
k6 run backend/observability/k6/wb-slo-load.js \
  -e BASE=http://127.0.0.1:9090 \
  -e HMAC_KEY=$BAKE_HMAC_KEY
```

The k6 script lives in `backend/observability/k6/wb-slo-load.js` (added in a
follow-up if not present). It targets a 50 RPS warm-cache mix and asserts the
Prometheus-scraped p95 lands below 150 ms in the dashboard within 5 min.

## 7. Constraints

- This guide tells the operator (Rolf) what to install. Automation must never
  reach into `/srv/shared/`. Only the bake-server side
  (`backend/observability/*`) lives in this repo.
- If the shared stack is unavailable, bake-server's `/metrics` still works
  and can be scraped manually with `curl` for ad-hoc inspection.
