#!/usr/bin/env bash
# Build + deploy the worldbuilders web companion to its serving directory.
#
# Constraints (DECISIONS.md):
#   - Worldbuilders-owned paths ONLY. NEVER /srv/* or other project trees.
#   - The serving dir is /home/deploy/projects/worldbuilders/web-deploy/current/
#   - Operator (human) wires nginx/Caddy to that path; see BLOCKED/needs-human.md.
#
# Per Q325 (DELIVERABLE #6) the script rsyncs the build artefact into the
# project-owned web-deploy/ tree, atomically swaps the `current` symlink, and
# prunes old releases. No sudo, no /etc, no /srv writes.
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_ROOT="$(cd "$PROJECT_ROOT/.." && pwd)/web-deploy"
RELEASES_DIR="$DEPLOY_ROOT/releases"
CURRENT_LINK="$DEPLOY_ROOT/current"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
RELEASE_DIR="$RELEASES_DIR/$TS"

cd "$PROJECT_ROOT"

if [ ! -d node_modules ]; then
  echo "[deploy] installing deps"
  npm ci || npm install
fi

echo "[deploy] astro build (site=${WB_SITE_URL:-https://worldbuilders.quicktoolry.com})"
WB_SITE_URL="${WB_SITE_URL:-https://worldbuilders.quicktoolry.com}" npm run build

mkdir -p "$RELEASE_DIR"
echo "[deploy] rsync dist/ -> $RELEASE_DIR"
rsync -a --delete "$PROJECT_ROOT/dist/" "$RELEASE_DIR/"

# Use a RELATIVE symlink target so the chain resolves identically from inside
# the shared-nginx container, which bind-mounts $DEPLOY_ROOT to /srv/worldbuilders-web.
ln -sfn "releases/$TS" "$CURRENT_LINK.new"
mv -Tf "$CURRENT_LINK.new" "$CURRENT_LINK"
echo "[deploy] current -> $(readlink "$CURRENT_LINK")"

# Keep 5 most recent releases.
( cd "$RELEASES_DIR" && ls -1tr | head -n -5 | xargs -r rm -rf )

echo "[deploy] done. Operator action: point Caddy/nginx vhost at $CURRENT_LINK"
