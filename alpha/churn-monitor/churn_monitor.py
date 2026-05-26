#!/usr/bin/env python3
"""Worldbuilders alpha churn monitor (Q243).

Polls two side channels:

  1. Roblox Analytics (when the game is published) via Open Cloud / Analytics API
  2. Discord activity via the bot's `messages` audit log

…and computes, per cohort:

  - DAU (daily active testers)
  - WAU (weekly active testers)
  - Churn rate = (testers with 0 sessions in the trailing 7 days) / cohort size
  - Median session length (Roblox-side)

Output: appends a markdown block to docs/HEARTBEAT.md under "## Alpha pulse"
on each run. Designed to be cron'd weekly (Mondays 08:00 UTC).

This is a deliberately small script. It tolerates missing creds — if either
data source is unreachable, it logs "N/A" for that side. We can swap in real
APIs once the Roblox place is published.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import pathlib
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass, field, asdict
from typing import Iterable


HEARTBEAT_PATH = pathlib.Path(
    os.environ.get(
        "WB_HEARTBEAT_PATH",
        "/home/deploy/projects/worldbuilders/docs/HEARTBEAT.md",
    )
)
SECTION_HEADING = "## Alpha pulse"


@dataclass
class CohortMetrics:
    stage: int
    size: int
    dau: int | None
    wau: int | None
    churned: int | None
    churn_rate: float | None
    median_session_minutes: float | None
    discord_active_7d: int | None
    notes: list[str] = field(default_factory=list)

    def as_md_row(self) -> str:
        def fmt(v):
            return "N/A" if v is None else str(v)
        return (
            f"| {self.stage} | {self.size} | {fmt(self.dau)} | {fmt(self.wau)} | "
            f"{fmt(self.churned)} | "
            f"{'N/A' if self.churn_rate is None else f'{self.churn_rate*100:.1f}%'} | "
            f"{fmt(self.median_session_minutes)} | {fmt(self.discord_active_7d)} |"
        )


def fetch_roblox_dau(place_id: int, api_key: str) -> dict | None:
    """Stub for Roblox Open Cloud Analytics. Returns None when unavailable.

    Production version will call:
      GET https://apis.roblox.com/cloud/v2/universes/{universe_id}/analytics
    with the x-api-key header. We intentionally swallow ALL errors and return
    None — the operator runbook calls out that pre-publish, this is N/A.
    """
    if not api_key or not place_id:
        return None
    try:
        url = (
            f"https://apis.roblox.com/cloud/v2/universes/{place_id}/analytics"
            "?metricType=DAU,WAU,MedianSessionMinutes"
        )
        req = urllib.request.Request(url, headers={"x-api-key": api_key})
        with urllib.request.urlopen(req, timeout=10) as resp:
            return json.load(resp)
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError, OSError) as exc:
        print(f"[churn-monitor] roblox analytics unavailable: {exc}", file=sys.stderr)
        return None


def fetch_discord_active(
    guild_id: str, bot_token: str, channel_ids: Iterable[str], since: dt.datetime
) -> int | None:
    """Count distinct user_ids who posted in the alpha guild since `since`."""
    if not guild_id or not bot_token:
        return None
    seen: set[str] = set()
    headers = {"Authorization": f"Bot {bot_token}"}
    try:
        for chan in channel_ids:
            url = f"https://discord.com/api/v10/channels/{chan}/messages?limit=100"
            req = urllib.request.Request(url, headers=headers)
            with urllib.request.urlopen(req, timeout=10) as resp:
                msgs = json.load(resp)
            for m in msgs:
                ts = dt.datetime.fromisoformat(m["timestamp"].replace("Z", "+00:00"))
                if ts >= since:
                    seen.add(m["author"]["id"])
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError, KeyError, OSError) as exc:
        print(f"[churn-monitor] discord unreachable: {exc}", file=sys.stderr)
        return None
    return len(seen)


def fetch_cohort_size(stage: int, database_url: str | None) -> int:
    """Number of redeemed invites for the given stage. Returns 0 if DB absent."""
    if not database_url:
        return 0
    try:
        import psycopg  # type: ignore
    except ImportError:
        # psycopg is optional — without it we can't query. Operator runbook
        # documents `pip install psycopg[binary]`.
        return 0
    try:
        with psycopg.connect(database_url) as conn, conn.cursor() as cur:
            cur.execute(
                "SELECT COUNT(*) FROM wb.invite_codes "
                "WHERE stage = %s AND redeemed_at IS NOT NULL "
                "AND revoked_at IS NULL",
                (stage,),
            )
            row = cur.fetchone()
            return int(row[0]) if row else 0
    except Exception as exc:  # noqa: BLE001 — telemetry script, never raise
        print(f"[churn-monitor] db unavailable: {exc}", file=sys.stderr)
        return 0


def compute_cohort(stage: int, channels: list[str]) -> CohortMetrics:
    now = dt.datetime.now(dt.timezone.utc)
    week_ago = now - dt.timedelta(days=7)

    db_url = os.environ.get("DATABASE_URL")
    size = fetch_cohort_size(stage, db_url)

    place_id = int(os.environ.get("ROBLOX_UNIVERSE_ID", "0") or 0)
    roblox_key = os.environ.get("ROBLOX_OPEN_CLOUD_KEY", "")
    roblox = fetch_roblox_dau(place_id, roblox_key)
    dau = roblox.get("dau") if roblox else None
    wau = roblox.get("wau") if roblox else None
    median_session = roblox.get("median_session_minutes") if roblox else None

    guild_id = os.environ.get("DISCORD_GUILD_ID", "")
    bot_token = os.environ.get("DISCORD_BOT_TOKEN", "")
    discord_active = fetch_discord_active(guild_id, bot_token, channels, week_ago)

    # Churn = size minus union of (roblox WAU) and (discord active 7d).
    churned: int | None = None
    churn_rate: float | None = None
    if size > 0 and (wau is not None or discord_active is not None):
        active = max(wau or 0, discord_active or 0)
        churned = max(size - active, 0)
        churn_rate = churned / size

    notes: list[str] = []
    if churn_rate is not None and churn_rate > 0.30:
        notes.append("CHURN ALERT — >30% inactive this week; investigate per Q243 stop condition.")

    return CohortMetrics(
        stage=stage,
        size=size,
        dau=dau,
        wau=wau,
        churned=churned,
        churn_rate=churn_rate,
        median_session_minutes=median_session,
        discord_active_7d=discord_active,
        notes=notes,
    )


def emit_to_heartbeat(metrics: list[CohortMetrics]) -> str:
    now = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    lines = [
        "",
        f"### {now}",
        "",
        "| Stage | Size | DAU | WAU | Churned | Churn % | Median session (min) | Discord active 7d |",
        "|-------|------|-----|-----|---------|---------|----------------------|--------------------|",
    ]
    lines.extend(m.as_md_row() for m in metrics)
    note_lines = [n for m in metrics for n in m.notes]
    if note_lines:
        lines.append("")
        for n in note_lines:
            lines.append(f"- **{n}**")
    block = "\n".join(lines) + "\n"

    if not HEARTBEAT_PATH.exists():
        HEARTBEAT_PATH.parent.mkdir(parents=True, exist_ok=True)
        HEARTBEAT_PATH.write_text(f"# HEARTBEAT\n\n{SECTION_HEADING}\n")

    contents = HEARTBEAT_PATH.read_text()
    if SECTION_HEADING not in contents:
        contents = contents.rstrip() + f"\n\n{SECTION_HEADING}\n"

    # Insert the new block immediately after the section heading so newest is at top.
    head, _, tail = contents.partition(SECTION_HEADING)
    contents = head + SECTION_HEADING + "\n" + block + tail.lstrip("\n")
    HEARTBEAT_PATH.write_text(contents)
    return block


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--channels",
        nargs="*",
        default=[],
        help="Discord channel ids to scan (defaults to env $DISCORD_CHANNELS).",
    )
    p.add_argument("--dry-run", action="store_true", help="Print, don't write to HEARTBEAT.md")
    p.add_argument(
        "--stage",
        type=int,
        action="append",
        choices=[1, 2, 3],
        help="Restrict to a specific stage (can be passed multiple times).",
    )
    args = p.parse_args(argv)

    channels = args.channels or os.environ.get("DISCORD_CHANNELS", "").split(",")
    channels = [c for c in channels if c]

    stages = args.stage or [1, 2, 3]
    metrics = [compute_cohort(s, channels) for s in stages]

    if args.dry_run:
        for m in metrics:
            print(json.dumps(asdict(m)))
        return 0

    block = emit_to_heartbeat(metrics)
    print(block)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
