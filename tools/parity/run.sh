#!/usr/bin/env bash
# The release check `docs/docs/cli/index.md` describes: one application run
# under `jwc serve` and as its `jwc build` binary, each on a fresh database
# and its own Redis database, both driven through the same scenario, every
# response diffed.
#
#   tools/parity/run.sh <app-dir> <scenario>
#
#   scenario   shortener | mywallet | eschool   (see parity.py)
#
# Environment, with defaults:
#   JWC        target/debug/jwc — built with `--features redis`
#   PG_URL     postgres://postgres:postgres@127.0.0.1:5432
#   REDIS_URL  redis://127.0.0.1:6379
#
# Exits non-zero when any response differs. The application has to be on
# the language this compiler accepts: the three it was written for still pin
# rc.6/rc.8 in their own repositories.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
app=$(cd "$1" && pwd)
scenario=$2
JWC=${JWC:-$here/../../target/debug/jwc}
PG_URL=${PG_URL:-postgres://postgres:postgres@127.0.0.1:5432}
REDIS_URL=${REDIS_URL:-redis://127.0.0.1:6379}
name=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["name"])' "$app/jwcproj.json")
db=jwc_parity_$(echo "$scenario" | tr -c 'a-z0-9\n' '_')
work=$(mktemp -d)
# `exec` below makes each subshell the server itself, so this reaches it.
trap 'kill ${pids:-} 2>/dev/null || true; wait 2>/dev/null || true; rm -rf "$work"' EXIT
for port in 9101 9102; do
  if curl -fsS -o /dev/null "localhost:$port/healthz" 2>/dev/null; then
    echo "port $port is already serving something; stop it first" >&2
    exit 1
  fi
done

"$JWC" build "$app" > "$work/build.log" 2>&1 || { tail -20 "$work/build.log"; exit 1; }
"$JWC" gen-sql "$app" > "$work/schema.sql"
for side in serve native; do
  psql "$PG_URL/postgres" -q -c "DROP DATABASE IF EXISTS ${db}_$side" -c "CREATE DATABASE ${db}_$side" 2>&1 | grep -v NOTICE || true
  psql "$PG_URL/${db}_$side" -q -v ON_ERROR_STOP=1 -f "$work/schema.sql" > /dev/null 2>&1
done
redis-cli -u "$REDIS_URL/14" flushdb > /dev/null
redis-cli -u "$REDIS_URL/15" flushdb > /dev/null

common=(JWT_SECRET=parity-secret-abcdefghijklmnopqrstuvwxyz JWT_TTL_MINUTES=60
        CURSOR_SECRET=parity-cursor-0123456789abcdef BOOTSTRAP_TOKEN=eschool-bootstrap-token-2026
        PASS_MARK=3 PUBLIC_BASE_URL=http://short.test RUST_BACKTRACE=0)
(cd "$app" && exec env "${common[@]}" JWC_REDIS_URL="$REDIS_URL/14" DATABASE_URL="$PG_URL/${db}_serve" \
   "$JWC" serve . --port 9101 > "$work/serve.log" 2>&1) &
pids="$!"
(cd "$app" && exec env "${common[@]}" JWC_REDIS_URL="$REDIS_URL/15" DATABASE_URL="$PG_URL/${db}_native" \
   PORT=9102 "./bin/debug/$name" > "$work/native.log" 2>&1) &
pids="$pids $!"
for _ in $(seq 1 60); do
  curl -fsS -o /dev/null localhost:9101/healthz 2>/dev/null \
    && curl -fsS -o /dev/null localhost:9102/healthz 2>/dev/null && break
  sleep 0.5
done
python3 "$here/parity.py" "$scenario" "$app" http://127.0.0.1:9101 http://127.0.0.1:9102
