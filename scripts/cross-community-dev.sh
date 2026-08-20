#!/usr/bin/env bash
# =============================================================================
# cross-community-dev.sh — multi-community test environment (BUZZ-10)
# =============================================================================
# Stands up TWO isolated relays as separate communities, seeds them with a
# remote-invocable marketplace agent, and fronts both with Tailscale TLS so
# cross-community coordinates satisfy the wss requirement.
#
#   Community A (agent home):  relay :3040, DB buzz_cc_a, wss :3445
#   Community B (caller):      relay :3041, DB buzz_cc_b, wss :3446
#
# Seeded on A: agent "Bumble" (kind:30177 listing, marketplace.listed=true,
# remote_invocation any_community, USD 0.10/hour) plus a #general channel on
# both relays. Keys persist in .tmp/cross-community/keys.env so re-runs are
# stable — safe to run repeatedly.
#
# BUZZ_UNSAFE_ALLOW_PRIVATE_REMOTE_HOSTS=1 is set on both relays (and must be
# set on any agent harness) because tailnet/localhost addresses are otherwise
# rejected by the SSRF guard. Dev only.
#
# Usage:
#   ./scripts/cross-community-dev.sh up      # infra + relays + tailscale + seed
#   ./scripts/cross-community-dev.sh seed    # (re)seed only
#   ./scripts/cross-community-dev.sh status
#   ./scripts/cross-community-dev.sh down    # stop relays + remove TLS routes
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

STATE_DIR=".tmp/cross-community"
KEYS_ENV="${STATE_DIR}/keys.env"
TS_HOST="$(tailscale status --json 2>/dev/null | python3 -c 'import sys,json; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))' 2>/dev/null || true)"

A_PORT=3040 B_PORT=3041
A_TLS=3445 B_TLS=3446
A_DB=buzz_cc_a B_DB=buzz_cc_b
A_BUCKET=buzz-cc-a B_BUCKET=buzz-cc-b
PG=postgres://buzz:buzz_dev@localhost:5432

mkdir -p "${STATE_DIR}"

gen_key() { python3 -c 'import secrets; print(secrets.token_hex(32))'; }

ensure_keys() {
  if [[ ! -f "${KEYS_ENV}" ]]; then
    cat > "${KEYS_ENV}" <<EOF
RELAY_A_KEY=$(gen_key)
RELAY_B_KEY=$(gen_key)
OWNER_KEY=$(gen_key)
BUMBLE_KEY=$(gen_key)
EOF
    chmod 600 "${KEYS_ENV}"
  fi
  # shellcheck disable=SC1090
  source "${KEYS_ENV}"
}

pubkey_of() { # hex privkey -> hex pubkey (via the Rust seeding helper)
  SEED_PRIVATE_KEY="$1" ./target/debug/publish_event --pubkey
}

seed_hosts() { # $1=db $2=port $3=tlsport
  local hosts="('localhost:$2'),('127.0.0.1:$2'),('localhost'),('127.0.0.1')"
  if [[ -n "${TS_HOST}" ]]; then
    hosts="${hosts},('${TS_HOST}:$3')"
  fi
  psql "${PG}/$1" -q -v ON_ERROR_STOP=1 -c \
    "INSERT INTO communities (host) SELECT host FROM (VALUES ${hosts}) v(host) ON CONFLICT (lower(host)) DO NOTHING;"
}

start_relay() { # $1=name $2=port $3=tlsport $4=db $5=bucket $6=key $7=health $8=metrics
  if lsof -ti ":$2" >/dev/null 2>&1; then
    echo "[cc] relay $1 already listening on :$2"
    return
  fi
  local relay_url="ws://localhost:$2"
  [[ -n "${TS_HOST}" ]] && relay_url="wss://${TS_HOST}:$3"
  env -i PATH="${PATH}" HOME="${HOME}" \
    DATABASE_URL="${PG}/$4" \
    PGHOST=localhost PGPORT=5432 PGUSER=buzz PGPASSWORD=buzz_dev PGDATABASE="$4" \
    BUZZ_EXTERNAL_POSTGRES=1 BUZZ_EXTERNAL_REDIS=1 \
    REDIS_URL=redis://localhost:6379 \
    BUZZ_BIND_ADDR="0.0.0.0:$2" \
    RELAY_URL="${relay_url}" \
    BUZZ_RELAY_PRIVATE_KEY="$6" \
    BUZZ_S3_ENDPOINT=http://localhost:9000 BUZZ_S3_ACCESS_KEY=buzz_dev \
    BUZZ_S3_SECRET_KEY=buzz_dev_secret BUZZ_S3_BUCKET="$5" \
    BUZZ_S3_REGION=us-east-1 BUZZ_S3_ADDRESSING_STYLE=path \
    BUZZ_HEALTH_PORT="$7" BUZZ_METRICS_PORT="$8" \
    BUZZ_UNSAFE_ALLOW_PRIVATE_REMOTE_HOSTS=1 \
    RUST_LOG=buzz_relay=debug \
    nohup ./target/debug/buzz-relay > "${STATE_DIR}/relay-$1.log" 2>&1 &
  echo "[cc] relay $1 starting on :$2 (log: ${STATE_DIR}/relay-$1.log)"
}

wait_relay() { # $1=port
  for _ in $(seq 1 45); do
    if curl -sf "localhost:$1" -H 'Accept: application/nostr+json' >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "[cc] relay on :$1 did not become healthy" >&2
  return 1
}

cmd_up() {
  ensure_keys
  # DBs + buckets
  for db in "${A_DB}" "${B_DB}"; do
    psql "${PG}/postgres" -tAc "SELECT 1 FROM pg_database WHERE datname='${db}'" | grep -q 1 \
      || psql "${PG}/postgres" -q -c "CREATE DATABASE ${db} OWNER buzz;"
  done
  docker exec buzz-minio mc alias set local http://localhost:9000 buzz_dev buzz_dev_secret >/dev/null 2>&1 || true
  docker exec buzz-minio mc mb --ignore-existing "local/${A_BUCKET}" "local/${B_BUCKET}" >/dev/null 2>&1 || true

  for db in "${A_DB}" "${B_DB}"; do
    DATABASE_URL="${PG}/${db}" ./target/debug/buzz-admin migrate \
      || { echo "[cc] migrations failed for ${db}"; exit 1; }
  done

  start_relay a "${A_PORT}" "${A_TLS}" "${A_DB}" "${A_BUCKET}" "${RELAY_A_KEY}" 8083 9203
  start_relay b "${B_PORT}" "${B_TLS}" "${B_DB}" "${B_BUCKET}" "${RELAY_B_KEY}" 8084 9204
  wait_relay "${A_PORT}"; wait_relay "${B_PORT}"

  seed_hosts "${A_DB}" "${A_PORT}" "${A_TLS}"
  seed_hosts "${B_DB}" "${B_PORT}" "${B_TLS}"

  if [[ -n "${TS_HOST}" ]]; then
    tailscale serve --bg --https="${A_TLS}" "http://127.0.0.1:${A_PORT}" >/dev/null
    tailscale serve --bg --https="${B_TLS}" "http://127.0.0.1:${B_PORT}" >/dev/null
    echo "[cc] tailscale: wss://${TS_HOST}:${A_TLS} -> :${A_PORT}, wss://${TS_HOST}:${B_TLS} -> :${B_PORT}"
  else
    echo "[cc] WARNING: tailscale unavailable — no wss front; cross-community dispatch will fail the wss check"
  fi

  cmd_seed
  cmd_status
}

publish() { # $1=relay_url $2=key $3=kind $4=tags-json $5=content
  BUZZ_RELAY_URL="$1" SEED_PRIVATE_KEY="$2" \
    ./target/debug/publish_event "$3" "$4" "$5"
}

cmd_seed() {
  ensure_keys
  local bumble_pub owner_pub
  bumble_pub="$(pubkey_of "${BUMBLE_KEY}")"
  owner_pub="$(pubkey_of "${OWNER_KEY}")"
  [[ -n "${bumble_pub}" && -n "${owner_pub}" ]] || { echo "[cc] cannot derive pubkeys"; exit 1; }

  # NIP-42 verification matches the relay's configured RELAY_URL, so seed
  # over the same authority the relay advertises (the wss front when up).
  local a_url="ws://localhost:${A_PORT}" b_url="ws://localhost:${B_PORT}"
  local a_http="http://localhost:${A_PORT}" b_http="http://localhost:${B_PORT}"
  if [[ -n "${TS_HOST}" ]]; then
    a_url="wss://${TS_HOST}:${A_TLS}"; b_url="wss://${TS_HOST}:${B_TLS}"
    a_http="https://${TS_HOST}:${A_TLS}"; b_http="https://${TS_HOST}:${B_TLS}"
  fi

  # Channels so both communities have a place for results.
  BUZZ_RELAY_URL="${a_http}" BUZZ_PRIVATE_KEY="${OWNER_KEY}" \
    ./target/debug/buzz channels create --name general --type stream --visibility open \
    || echo "[cc] channel create on A skipped (may already exist)"
  BUZZ_RELAY_URL="${b_http}" BUZZ_PRIVATE_KEY="${OWNER_KEY}" \
    ./target/debug/buzz channels create --name general --type stream --visibility open \
    || echo "[cc] channel create on B skipped (may already exist)"

  # Marketplace listings require the author to be the agent's verified owner
  # (users.agent_owner_pubkey). Register Bumble under the owner in the
  # community the desktop uses (the wss authority when tailscale is up).
  local a_host="localhost:${A_PORT}"
  [[ -n "${TS_HOST}" ]] && a_host="${TS_HOST}:${A_TLS}"
  psql "${PG}/${A_DB}" -q -v ON_ERROR_STOP=1 -c "
    INSERT INTO users (community_id, pubkey, display_name)
    SELECT c.id, decode('${owner_pub}','hex'), 'Seed Owner'
    FROM communities c WHERE lower(c.host) = lower('${a_host}')
    ON CONFLICT (community_id, pubkey) DO NOTHING;
    INSERT INTO users (community_id, pubkey, display_name, agent_type, agent_owner_pubkey)
    SELECT c.id, decode('${bumble_pub}','hex'), 'Bumble', 'managed', decode('${owner_pub}','hex')
    FROM communities c WHERE lower(c.host) = lower('${a_host}')
    ON CONFLICT (community_id, pubkey)
    DO UPDATE SET agent_owner_pubkey = EXCLUDED.agent_owner_pubkey;"

  # Bumble marketplace listing on A (owner-signed, d = agent pubkey).
  local listing
  listing=$(cat <<EOF
{"name":"Bumble","persona_id":"builtin:bumble","respond_to":"anyone","marketplace":{"listed":true,"description":"Cross-community researcher","capabilities":["research"],"deployment":"local","pricing":{"currency":"USD","microunits_per_hour":100000},"remote_invocation":{"policy":"any_community"}}}
EOF
)
  publish "${a_url}" "${OWNER_KEY}" 30177 "[[\"d\",\"${bumble_pub}\"]]" "${listing}" \
    && echo "[cc] seeded Bumble listing on A (agent ${bumble_pub:0:8}…)"

  # Agent profile so relay agent lists can resolve it.
  publish "${a_url}" "${BUMBLE_KEY}" 10100 "[[\"name\",\"Bumble\"]]" '{"name":"Bumble","about":"Cross-community researcher"}' \
    >/dev/null 2>&1 || true

  cat > "${STATE_DIR}/summary.txt" <<EOF
Cross-community dev environment
================================
Community A (agent home) : ws://localhost:${A_PORT}$( [[ -n "${TS_HOST}" ]] && echo "  |  wss://${TS_HOST}:${A_TLS}" )
Community B (caller)     : ws://localhost:${B_PORT}$( [[ -n "${TS_HOST}" ]] && echo "  |  wss://${TS_HOST}:${B_TLS}" )
Owner key                : ${OWNER_KEY}
Owner pubkey             : ${owner_pub}
Bumble key               : ${BUMBLE_KEY}
Bumble pubkey            : ${bumble_pub}

Add to the desktop app with the wss URLs (import the owner key as identity).
Agent harness (home A):
  BUZZ_UNSAFE_ALLOW_PRIVATE_REMOTE_HOSTS=1 BUZZ_PRIVATE_KEY=${BUMBLE_KEY} \\
  BUZZ_RELAY_URL=ws://localhost:${A_PORT} ./target/debug/buzz-acp
EOF
  echo "[cc] summary: ${STATE_DIR}/summary.txt"
}

cmd_status() {
  for spec in "a:${A_PORT}" "b:${B_PORT}"; do
    local_name="${spec%%:*}"; local_port="${spec##*:}"
    if curl -sf "localhost:${local_port}" -H 'Accept: application/nostr+json' >/dev/null 2>&1; then
      self=$(curl -s "localhost:${local_port}" -H 'Accept: application/nostr+json' | python3 -c 'import sys,json; print(json.load(sys.stdin).get("self"))')
      echo "[cc] relay ${local_name} :${local_port} UP  self=${self}"
    else
      echo "[cc] relay ${local_name} :${local_port} DOWN"
    fi
  done
}

cmd_down() {
  for port in "${A_PORT}" "${B_PORT}"; do
    pid=$(lsof -ti ":${port}" 2>/dev/null || true)
    [[ -n "${pid}" ]] && kill "${pid}" && echo "[cc] stopped relay on :${port}"
  done
  if [[ -n "${TS_HOST}" ]]; then
    tailscale serve --https="${A_TLS}" off >/dev/null 2>&1 || true
    tailscale serve --https="${B_TLS}" off >/dev/null 2>&1 || true
  fi
}

case "${1:-up}" in
  up) cmd_up ;;
  seed) cmd_seed ;;
  status) cmd_status ;;
  down) cmd_down ;;
  *) echo "Usage: $0 [up|seed|status|down]"; exit 1 ;;
esac
