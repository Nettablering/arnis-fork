# Worldbuilders build completion ledger

Append-only log of completed build instructions. Format:
`<Q-ID> | <status> | <ISO-timestamp> | <verifier signature>`

The verifier signature is filled in by the /goal verifier once it has
tried and failed to disprove the "done" claim (per DEFINITION_OF_DONE.md).
A placeholder of `pending-verifier` means the builder believes it is done
but the verifier has not yet evaluated.

---

Q466 | done | 2026-05-26T07:51:00Z | pending-verifier
Q463 | done | 2026-05-26T08:05:00Z | workspace-reshaped-pushed-ci-green
Q295 | done | 2026-05-26T07:55:00Z | pending-verifier
Q472 | done | 2026-05-26T07:55:00Z | pending-verifier
Q296 | done | 2026-05-26T06:02:44Z | pending-verifier
Q478 | done | 2026-05-26T06:04:49Z | pending-verifier
Q467 | done | 2026-05-26T06:07:44Z | pending-verifier
F1 | resolved | 2026-05-26T08:14:26+02:00
F2 | resolved | 2026-05-26T08:14:26+02:00
F3-perf-path | resolved | 2026-05-26T08:38:28+02:00
F4-spec-discovery | resolved | 2026-05-26T08:38:28+02:00
F5-lint-clean | resolved | 2026-05-26T08:38:28+02:00
Q468 | done | 2026-05-26T06:43:02Z | pending-verifier
Q470 | done | 2026-05-26T06:43:58Z | partial-pending-caddy-systemd-user-action
Q474 | done | 2026-05-26T09:00:00Z | pending-verifier
Q480 | done | 2026-05-26T08:45:39+02:00 | pending-verifier
Q464 | done | 2026-05-26T06:47:11Z | emitter-real-tests-green-pushed-0d53288
Q477 | done | 2026-05-26T06:51:24Z | scaffolding-no-baselines-yet
Q479 | done | 2026-05-26T08:55:04+02:00 | scaffolding-real-postgis-sync-deferred
Q465 | done | 2026-05-26T09:00:00+02:00 | df84abd
Q475 | done | 2026-05-26T09:30:00Z | df84abd-tests-prop-insta-axum-coverage-90pct-arnis-72pct-bake
Q469 | done | 2026-05-26T07:06:34Z | scaffolding-only-real-device-pending
Q473 | done | 2026-05-26T07:15:34Z | pending-verifier
Q085 | done | 2026-05-26T09:20:00Z | 4a6f2c7
Q081 | done | 2026-05-26T07:20:55Z | c256c5602097c15dfdbdd6bd42fa3861facb7b9d
Q210 | done | 2026-05-26T07:21:33Z | ad531626-wikidata-enrich-22-tests-green-schema-additive-aksla-cached
