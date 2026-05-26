# Operator runbook — overlay backup & restore (Q099)

This is the on-call runbook for the nightly Roblox DataStore overlay backup
implemented in `backend/datastore-backup/`. Design rationale lives in
`docs/grill/q099-backup-overlay-snapshots.md`; this file is the operational
companion.

## Components

| Piece | Path |
|-------|------|
| Rust crate | `backend/datastore-backup/` |
| Backup binary | `target/release/wb-backup` |
| Restore binary | `target/release/wb-backup-restore` |
| systemd unit | `backend/datastore-backup/systemd/wb-backup.service` |
| systemd timer | `backend/datastore-backup/systemd/wb-backup.timer` (02:30 UTC) |
| age public key | `~/.claude/shared/api-keys/worldbuilders-backup-age.pub` |
| age private key | `~/.claude/shared/api-keys/worldbuilders-backup-age.key` (mode 600) |
| Local fallback | `/home/deploy/projects/worldbuilders/backups/` |

## Daily operation

The timer fires at 02:30 UTC. The service runs `wb-backup` which:

1. Calls the Roblox Open Cloud DataStore API (or, if `OPEN_CLOUD_API_KEY` is
   unset, exercises the synthetic 10-key fixture to keep the pipeline warm).
2. Streams entries to `overlays.jsonl`, compresses with zstd, encrypts to the
   single configured age recipient.
3. Writes a `manifest.json.age` with SHA-256 of the encrypted archive.
4. Uploads to every configured `StorageTarget`. Today only `local` is wired —
   Hetzner Storage Box + B2 are stubs awaiting credentials (see
   `BLOCKED/needs-human.md`).

## Restore (point-in-time)

```bash
cd /home/deploy/projects/worldbuilders/backend/datastore-backup
cargo run --release --bin wb-backup-restore -- \
    --date 2026-05-26 \
    --universe-id 99 \
    --target /tmp/restore-x
```

This will:

1. Pull `overlays.jsonl.zst.age` + `manifest.json.age` from the local
   storage root (configurable via `--local-root`).
2. Decrypt the manifest, verify SHA-256 of the encrypted archive.
3. Decrypt the archive, decompress zstd, parse jsonl into `entries.jsonl`.
4. Refuse to write any partial output if the manifest check fails.

## Weekly integrity drill

A weekly cron picks a random archive from the last 30 days, restores it to a
scratch directory and counts entries. Failure pages the operator.

Currently this is a manual run:

```bash
DATE=$(ls /home/deploy/projects/worldbuilders/backups/universes/1 | shuf -n1)
cargo run --release --bin wb-backup-restore -- --date "$DATE" --target /tmp/drill
```

The wired-in cron lands when a real universe is enrolled (Q100 follow-on).

## Key rotation

age-keygen generates a new keypair. Re-encrypt new archives to the new key;
old archives stay decryptable with the old key. Keep both in
`~/.claude/shared/api-keys/` until 90-day retention drops the last
old-encrypted archive, then archive the old key offline.

## What's blocked

- Hetzner Storage Box SSH credentials — not provisioned. Logged in
  `BLOCKED/needs-human.md`.
- Backblaze B2 API keys — not provisioned. Logged in
  `BLOCKED/needs-human.md`.
- Real `OPEN_CLOUD_API_KEY` and `WB_UNIVERSE_ID` — set in the service `.env`
  once a universe is published.
