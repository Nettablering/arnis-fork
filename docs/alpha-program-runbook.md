# Operator runbook — pre-launch alpha programme (Q243)

Owner: Rolf. Updated when something breaks; never deleted (append-only at the bottom).

## What this is

The three-stage tester funnel from `docs/grill/q243-prelaunch-alpha-tester-program.md`:

| Stage | Size | Window     | NDA      | Discord    | Cadence       |
|-------|------|------------|----------|------------|---------------|
| 1     | 50   | T-12..T-8  | hard     | private    | bi-weekly 30m |
| 2     | 500  | T-8..T-4   | soft     | moderated  | weekly survey |
| 3     | 5000 | T-4..T-0   | none     | public     | dashboard     |

Source of truth for code/state: `/home/deploy/projects/worldbuilders/alpha/`.

---

## Day-to-day

### 1. Recruit

- Stage 1: hand-pick from Rolf's network + Twitter DMs + Brookhaven/Adopt Me power users.
- Stage 2: open `/alpha` form on the live site. Drain weekly from `wb.applicants`.
- Stage 3: open the floodgates; geographic soft launch in PH, NZ, NO (per Q248).

### 2. Triage applications

Daily, while a wave is open:

```sh
psql -h 127.0.0.1 -U worldbuilders -d worldbuilders \
  -c "SELECT applicant_id, display_name, country, primary_device, age_band \
      FROM wb.applicants WHERE status='pending' ORDER BY received_at LIMIT 20;"
```

Score by:

1. Device diversity (need iPhone SE 2, budget Android, iPad, desktop).
2. Regional spread (no more than 30% from any one country in a wave).
3. Roblox tenure (lookup the username via Roblox API; ≥1 year preferred).
4. Stated hours/week ≥ 2.

Mark accepted: `UPDATE wb.applicants SET status='invited', invited_at=now(), invite_code=$CODE WHERE applicant_id=$ID;`

### 3. Mint invite codes

```sh
cd alpha/invite-system
DATABASE_URL=postgres://worldbuilders@127.0.0.1/worldbuilders \
  cargo run --bin wb-invite -- create --count 50 --batch alpha-wave-1 --stage 1
```

Output: 50 codes printed to stdout, persisted to `wb.invite_codes`. Email the
code + the Dropbox Sign NDA link to each accepted applicant.

### 4. Process NDA signatures (Stage 1 only)

Use Dropbox Sign (formerly HelloSign). Template lives at
`alpha/nda/alpha-nda-v1.md` — copy into Dropbox Sign as a new template each
time the version bumps. Under-18 applicants need the parent/guardian co-sign.

### 5. Redeem on the in-game side

When a tester enters the invite code in the Roblox game, the client posts to
`/v1/invite/redeem` on bake-server (handler to be added; uses the
`wb_invite::redeem_code` library function).

For manual redemption (e.g. fixing a misclick):

```sh
DATABASE_URL=... cargo run --bin wb-invite -- redeem --code XXXXXXXXXXXX --roblox-user-id 12345
```

### 6. Run feedback loops

- **In-game surveys** fire at minute 5 / 30 / 180 (see `alpha/feedback/in-game-survey-design.md`).
- **Discord** — channels per `alpha/feedback/discord-server-layout.md`.
- **GitHub Issues** — private repo `klokk-nettablering/alpha-feedback-private`.

Weekly Monday: dump survey results, post a "State of Alpha" Loom (5 min).

### 7. Churn monitor

Cron Monday 08:00 UTC:

```
0 8 * * 1 cd /home/deploy/projects/worldbuilders/alpha/churn-monitor && \
  DATABASE_URL=... DISCORD_BOT_TOKEN=... DISCORD_GUILD_ID=... \
  DISCORD_CHANNELS=... ROBLOX_UNIVERSE_ID=... ROBLOX_OPEN_CLOUD_KEY=... \
  python3 churn_monitor.py
```

Output appended to `docs/HEARTBEAT.md` under `## Alpha pulse`. A `CHURN ALERT`
note (>30% inactive 7d) is a stop-condition — pause recruitment for that stage
until diagnosed.

---

## Graduation gates

A stage cannot expand until:

| From | To | Gate                                                            |
|------|----|-----------------------------------------------------------------|
| 1    | 2  | D7 retention ≥ 25%, <3 P0 crashes/1000 sessions, NPS ≥ 40       |
| 2    | 3  | D7 ≥ 18%, ARPDAU ≥ 0.015 Robux, server CPU within budget, 0 live exploits |

Gates are computed manually from the survey + churn data and the bake-server
analytics dashboard.

---

## Failure modes & responses

- **NDA leak.** Watermark on every clip identifies the leaker. Pull from cohort
  same day; public PSA in `#announcements`; document in `BLOCKED/`.
- **Cohort burnout (week 6).** Rotate in 10 fresh testers; keep originals on
  with reduced cadence.
- **Region-mismatch bugs in Stage 3.** Spin up 1 emergency triage call with
  affected testers; ship targeted patch within 72h.
- **Discord platform-ban.** Fall back to the Guilded mirror (per
  `alpha/feedback/discord-server-layout.md`) and the `alpha@worldbuilders.app`
  email list. Restore Discord within a week if appeal succeeds.
- **Tester payment legal.** Robux gifts <USD 20/tester/year. Cash never.

---

## Glossary

- **Founder badge** — permanent in-game cosmetic granted to Stage-1 alpha testers.
- **Wave** — a single recruitment cycle within a stage (e.g. alpha-wave-1).
- **Graduation gate** — quantitative threshold a cohort must clear before next stage opens.
