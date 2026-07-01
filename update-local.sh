#!/usr/bin/env bash
#
# One-shot updater for the local Docker deployment.
#   pull latest code  ->  rebuild image  ->  recreate the app container
#
# Data is safe: Postgres (pgdata), storage (archiver-data), Meilisearch and
# Redis (valkeydata) all live in named volumes that recreate never touches.
# Schema migrations run automatically on container start (docker-entrypoint.sh
# runs `pnpm db:migrate` before the app boots).
#
# Usage:  ./update-local.sh
#
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE="open-archiver:local"
# docker-compose.yml builds the app from local source (apps/open-archiver/Dockerfile)
# and tags it open-archiver:local — no override file needed.
COMPOSE=(docker compose -p openarchiver)

cd "$REPO_DIR"

echo "==> [1/5] Backing up the database (safety net)..."
mkdir -p backups
if docker exec postgres pg_dump -U admin open_archive 2>/dev/null | gzip > "backups/open_archive_$(date +%Y%m%d_%H%M%S).sql.gz"; then
	echo "    backup written to backups/"
else
	echo "    (skipped — is the postgres container running?)"
fi

echo "==> [2/5] Pulling latest code..."
git pull --ff-only

echo "==> [3/5] Building image..."
# Stamp the image with the commit we just checked out so the in-app update
# check can compare it against the fork.
GIT_SHA="$(git rev-parse HEAD)"
# Cached build is safe: .dockerignore excludes dist/build/*.tsbuildinfo, so the
# image always compiles clean and BuildKit busts the COPY layer on any content
# change (including deletions). Add --no-cache manually only if you suspect staleness.
docker build --build-arg GIT_SHA="$GIT_SHA" -t "$IMAGE" -f apps/open-archiver/Dockerfile .

echo "==> [4/5] Recreating the app (migrations run automatically on start)..."
"${COMPOSE[@]}" up -d --force-recreate open-archiver

echo "==> [5/5] Waiting for health..."
ok=0
for _ in $(seq 1 40); do
	code="$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3000/mailbox || true)"
	if [ "$code" = "200" ]; then ok=1; echo "    app is up (200)"; break; fi
	sleep 4
done
[ "$ok" = "1" ] || { echo "    WARNING: app did not return 200 — check: docker logs --tail 50 open-archiver"; exit 1; }

echo "==> Done. Updated to $(git rev-parse --short HEAD). Data volumes were untouched."
