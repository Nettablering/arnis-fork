// k6 load test for Q097 SLO conformance.
//
// Run:
//   k6 run wb-slo-load.js -e BASE=http://127.0.0.1:9090 -e HMAC_KEY=<hex>
//
// Target load: 50 RPS for 5 minutes, warm-cache mix (all tiles in a tight
// (z,x,y) range so the cache hits dominate). Asserts:
//   - p95 cached fetch < 150 ms (SLO-1)
//   - 5xx rate < 0.1% (SLO-3)
// k6 thresholds fail the run if breached, matching the SLO targets.

import http from 'k6/http';
import { check } from 'k6';
import crypto from 'k6/crypto';
import { Trend, Rate } from 'k6/metrics';

const BASE = __ENV.BASE || 'http://127.0.0.1:9090';
const HMAC_KEY_HEX = __ENV.HMAC_KEY || '';

const fetchLatency = new Trend('wb_fetch_latency_ms');
const fiveXxRate = new Rate('wb_5xx_rate');

export const options = {
  scenarios: {
    warm_cache: {
      executor: 'constant-arrival-rate',
      rate: 50,
      timeUnit: '1s',
      duration: '5m',
      preAllocatedVUs: 50,
      maxVUs: 100,
    },
  },
  thresholds: {
    // SLO-1 mirror.
    'wb_fetch_latency_ms': ['p(95)<150'],
    // SLO-3 mirror.
    'wb_5xx_rate': ['rate<0.001'],
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
  const hmac = crypto.hmac('sha256', KEY_BYTES.buffer, `${ts}\n${path}`, 'hex');
  return hmac;
}

export default function () {
  // Warm tile set: 100 tiles → high cache-hit ratio.
  const z = 15;
  const x = 1 + Math.floor(Math.random() * 10);
  const y = 1 + Math.floor(Math.random() * 10);
  const path = `/v1/tile/${z}/${x}/${y}`;
  const ts = Math.floor(Date.now() / 1000);

  const headers = {};
  if (KEY_BYTES) {
    headers['x-wb-ts'] = String(ts);
    headers['x-wb-sig'] = sign(ts, path);
  }

  const t0 = Date.now();
  const res = http.get(`${BASE}${path}`, { headers });
  const elapsedMs = Date.now() - t0;

  fetchLatency.add(elapsedMs);
  fiveXxRate.add(res.status >= 500 && res.status < 600);
  check(res, {
    'status < 500': (r) => r.status < 500,
  });
}
