# Worldbuilders — Pre-launch Alpha Program

Top-level artefacts for the three-stage tester funnel (alpha → closed beta → soft launch)
defined in `docs/grill/q243-prelaunch-alpha-tester-program.md`.

Stages:

| Stage | Size | Window     | NDA      | Cadence       |
|-------|------|------------|----------|---------------|
| 1     | 50   | T-12..T-8  | hard     | bi-weekly 30m |
| 2     | 500  | T-8..T-4   | soft     | weekly survey |
| 3     | 5000 | T-4..T-0   | none     | dashboard     |

Layout:

```
alpha/
  recruitment/        — landing copy + applicant form schema
  invite-system/      — Rust crate, wb-invite CLI, code minting & redemption
  migrations/         — sqlx migrations (wb.invite_codes + wb.applicants)
  nda/                — alpha NDA v1 (EN + NO)
  feedback/           — Discord layout, in-game survey plan, GitHub issue templates
  churn-monitor/      — Python poll script writing into docs/HEARTBEAT.md
```

International-from-day-one: recruitment is global. Norwegian and English NDA both
land in `nda/`. Form schema does not gate by country.

See `docs/alpha-program-runbook.md` for the operator runbook.
