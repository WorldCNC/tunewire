#!/usr/bin/env bash
# End-to-end library sync test: two devices, one account, a local Supabase.
# Needs Docker, Deno and the Supabase CLI (`supabase` on PATH, or SUPABASE=…).
#   scripts/test-sync.sh
set -euo pipefail
cd "$(dirname "$0")/.."
sb="${SUPABASE:-supabase}"
started=0
# Already running (any HTTP answer from the API gateway)? Then leave it running.
if [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:54321/rest/v1/ || true)" = "000" ]; then
  "$sb" start -x studio,imgproxy,edge-runtime,logflare,vector,supavisor,postgres-meta,mailpit >/dev/null
  started=1
fi
"$sb" migration up --local >/dev/null
keys=$("$sb" status -o env)
export ANON=$(echo "$keys" | grep '^ANON_KEY=' | cut -d'"' -f2)
export SERVICE=$(echo "$keys" | grep '^SERVICE_ROLE_KEY=' | cut -d'"' -f2)
TUNEWIRE_DEV_PORT=1420 python3 scripts/devserver.py >/dev/null 2>&1 &
dev=$!
cleanup() { kill "$dev" 2>/dev/null || true; if [ "$started" = 1 ]; then "$sb" stop >/dev/null 2>&1 || true; fi; }
trap cleanup EXIT
for _ in $(seq 40); do curl -sf -o /dev/null http://127.0.0.1:1420/ && break; sleep 0.25; done
deno run -A scripts/e2e/sync_e2e.ts
