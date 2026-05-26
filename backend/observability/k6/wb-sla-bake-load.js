// Q084 k6 load test — synchronous-bake p95 SLA conformance.
//
// Verifies that ≥95% of POST /v1/tile/{z}/{x}/{y}/bake requests complete
// within the 8 s soft deadline, at 50 RPS sustained for 60 s.
//
// Run:
//   k6 run wb-sla-bake-load.js -e BASE=http://127.0.0.1:9090 -e HMAC_KEY=<hex>
//
// k6 thresholds fail the run on breach.

import http from 'k6/http';
import { check } from 'k6';
import crypto from 'k6/crypto';
import { Trend, Rate, Counter } from 'k6/metrics';

const BASE = __ENV.BASE || 'http://127.0.0.1:9090';
const HMAC_KEY_HEX = __ENV.HMAC_KEY || '';

const bakeLatency = new Trend('wb_bake_latency_ms');
const slaBreachRate = new Rate('wb_bake_sla_breach_rate');
const placeholderCount = new Counter('wb_bake_placeholders');
const fastBakeCount = new Counter('wb_bake_fast_2xx');

export const options = {
  scenarios: {
    cold_bake: {
      executor: 'constant-arrival-rate',
      rate: 50,
      timeUnit: '1s',
      duration: '60s',
      preAllocatedVUs: 50,
      maxVUs: 200,
    },
  },
  thresholds: {
    // Q084 SLA — 95% of bake requests must complete in <= 8 s wall-clock.
    'wb_bake_latency_ms': ['p(95)<8000'],
    // No more than 5% of requests may breach the soft deadline (i.e.
    // fall back to placeholder + poll).
    'wb_bake_sla_breach_rate': ['rate<0.05'],
    // Hard ceiling for any failed request.
    'http_req_failed': ['rate<0.01'],
  },
};

function hexToBytes(hex) {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substr(i * 2, 2), 16);
  }
  return out;
}

const KEY_BYTES = HMAC_KEY_HEX ? hexToBytes(HMAC_KEY_HEX) : null;

function sign(ts, path) {
  if (!KEY_BYTES) return '';
  return crypto.hmac('sha256', KEY_BYTES.buffer, `${ts}\n${path}`, 'hex');
}

export default function () {
  // Spread requests across a wide tile range so each call exercises a
  // distinct cold-bake (not just cache reads).
  const z = 15;
  const x = 17000 + Math.floor(Math.random() * 1000);
  const y = 9000 + Math.floor(Math.random() * 1000);
  const path = `/v1/tile/${z}/${x}/${y}/bake`;
  const ts = Math.floor(Date.now() / 1000);

  const headers = {};
  if (KEY_BYTES) {
    headers['x-wb-ts'] = String(ts);
    headers['x-wb-sig'] = sign(ts, path);
  }

  const t0 = Date.now();
  const res = http.post(`${BASE}${path}`, null, { headers });
  const elapsedMs = Date.now() - t0;

  bakeLatency.add(elapsedMs);
  const breached = res.status === 202;
  slaBreachRate.add(breached);
  if (breached) {
    placeholderCount.add(1);
  } else if (res.status === 200) {
    fastBakeCount.add(1);
  }

  check(res, {
    'status is 200 or 202': (r) => r.status === 200 || r.status === 202,
    'p95 budget (8 s)': () => elapsedMs <= 8000,
  });
}
