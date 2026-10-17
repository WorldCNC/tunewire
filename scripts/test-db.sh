#!/usr/bin/env bash
# Applies every migration to a throwaway Postgres (Docker) with Supabase
# stand-ins, then runs the row level security tests in supabase/tests.
#   scripts/test-db.sh
set -euo pipefail
cd "$(dirname "$0")/.."
name="tunewire-dbtest-$$"
docker run -d --rm --name "$name" -e POSTGRES_PASSWORD=pw postgres:17-alpine >/dev/null
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
for _ in $(seq 60); do docker exec "$name" pg_isready -U postgres -q 2>/dev/null && break; sleep 0.5; done
sleep 1
run() { docker exec -i "$name" psql -U postgres -v ON_ERROR_STOP=1 -q -o /dev/null "$@"; }
run < supabase/tests/stubs.sql
for f in supabase/migrations/*.sql; do echo "apply $f"; run < "$f"; done
for f in supabase/tests/test_*.sql; do echo "test  $f"; run < "$f"; done
echo "database tests passed"
