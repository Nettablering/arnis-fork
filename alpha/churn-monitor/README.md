# Churn monitor

`churn_monitor.py` polls Roblox Open Cloud Analytics + Discord activity once
a week and appends a markdown block to `docs/HEARTBEAT.md` under the
`## Alpha pulse` heading.

## Run

```
# Weekly cron (Mondays 08:00 UTC)
DATABASE_URL=postgres://... \
ROBLOX_UNIVERSE_ID=... \
ROBLOX_OPEN_CLOUD_KEY=... \
DISCORD_GUILD_ID=... \
DISCORD_BOT_TOKEN=... \
DISCORD_CHANNELS=111,222,333 \
python3 churn_monitor.py
```

## Optional deps

`psycopg[binary]` is only required for the cohort-size query. Without it the
size column reads 0 and the rest of the report still emits.

## Stop condition

If any cohort row carries a `CHURN ALERT` note (>30% inactive in trailing 7d),
pause recruitment for that stage and triage. Per Q243.
