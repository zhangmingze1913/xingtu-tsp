#!/usr/bin/env bash
set -euo pipefail

# Initialize a local Xingtu TSP .env, secrets and SQLite directory.
#
# Usage:
#   ./scripts/init-system.sh
#   ./scripts/init-system.sh --db postgres --url 'postgres://tsp:...@127.0.0.1:5432/tsp?sslmode=disable'
#   ./scripts/init-system.sh --production --admin-password '...' --build
#
# This script does not install Rust or start databases.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DB="sqlite"
URL=""
ENV_FILE=".env"
DATA_DIR="data"
FORCE=0
PRODUCTION=0
BUILD=0
RUN=0
ADMIN_PASSWORD=""

usage() {
  sed -n '2,16p' "$0"
  exit "${1:-0}"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --db) DB="${2:?}"; shift 2 ;;
    --url) URL="${2:?}"; shift 2 ;;
    --env-file) ENV_FILE="${2:?}"; shift 2 ;;
    --data-dir) DATA_DIR="${2:?}"; shift 2 ;;
    --admin-password) ADMIN_PASSWORD="${2:?}"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --production) PRODUCTION=1; shift ;;
    --build) BUILD=1; shift ;;
    --run) RUN=1; shift ;;
    -h|--help) usage 0 ;;
    *) echo "unknown option: $1" >&2; usage 1 ;;
  esac
done

case "$DB" in sqlite|postgres|mysql|kingbase) ;; *) echo "invalid --db: $DB" >&2; exit 1 ;; esac

case "$ENV_FILE" in /*) ENV_PATH="$ENV_FILE" ;; *) ENV_PATH="$ROOT/$ENV_FILE" ;; esac
case "$DATA_DIR" in /*) DATA_PATH="$DATA_DIR" ;; *) DATA_PATH="$ROOT/$DATA_DIR" ;; esac
SECRET_DIR="$ROOT/secrets"

rand_hex() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex "$1"
  else
    od -An -N "$1" -tx1 /dev/urandom | tr -d ' \n'
  fi
}

rand_b64() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -base64 "$1" | tr -d '\n'
  else
    head -c "$1" /dev/urandom | base64 | tr -d '\n'
  fi
}

if [ -e "$ENV_PATH" ] && [ "$FORCE" -ne 1 ]; then
  echo "refusing to overwrite $ENV_PATH; pass --force" >&2
  exit 1
fi
if [ -e "$ENV_PATH" ]; then
  cp "$ENV_PATH" "$ENV_PATH.bak.$(date +%Y%m%d-%H%M%S)"
fi

mkdir -p "$DATA_PATH" "$SECRET_DIR"

if [ -z "$URL" ]; then
  case "$DB" in
    sqlite) URL="$DATA_PATH/tsp.db" ;;
    postgres) echo "postgres requires --url" >&2; exit 1 ;;
    mysql) echo "mysql requires --url" >&2; exit 1 ;;
    kingbase) echo "kingbase requires --url" >&2; exit 1 ;;
  esac
fi

if [ "$PRODUCTION" -eq 1 ] && [ "$DB" = "sqlite" ]; then
  echo "production requires PostgreSQL, KingbaseES or MySQL" >&2
  exit 1
fi

if [ -n "$ADMIN_PASSWORD" ]; then
  ADMIN="$ADMIN_PASSWORD"
elif [ "$PRODUCTION" -eq 1 ]; then
  ADMIN="$(openssl rand -base64 24 | tr -d '\n')"
else
  ADMIN="admin123"
fi

JWT_KEY=""
JWT_PUB=""
JT808_KEY=""
JT808_CERT=""
if command -v openssl >/dev/null 2>&1; then
  JWT_KEY="$SECRET_DIR/jwt-private.pem"
  JWT_PUB="$SECRET_DIR/jwt-public.pem"
  JT808_KEY="$SECRET_DIR/jt808-key.pem"
  JT808_CERT="$SECRET_DIR/jt808-cert.pem"
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$JWT_KEY" 2>/dev/null
  openssl rsa -in "$JWT_KEY" -pubout -out "$JWT_PUB" 2>/dev/null
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out "$JT808_KEY" 2>/dev/null
  openssl req -new -x509 -key "$JT808_KEY" -out "$JT808_CERT" -days 365 -subj '/CN=tsp-jt808' 2>/dev/null
elif [ "$PRODUCTION" -eq 1 ]; then
  echo "production requires openssl" >&2
  exit 1
else
  echo "warning: openssl not found; JWT uses process-local dev key" >&2
fi

POSTGIS=false
if [ "$DB" = "postgres" ]; then POSTGIS=true; fi
DEMO=true
if [ "$PRODUCTION" -eq 1 ]; then DEMO=false; fi
ENVIRONMENT=development
if [ "$PRODUCTION" -eq 1 ]; then ENVIRONMENT=production; fi
PLAINTEXT=true
if [ "$PRODUCTION" -eq 1 ]; then PLAINTEXT=false; fi

{
  printf 'TSP_ENV=%s\n' "$ENVIRONMENT"
  printf 'TSP_BIND=127.0.0.1:8090\n'
  printf 'TSP_DATABASE=%s\n' "$URL"
  printf 'TSP_WEB_DIR=web\n'
  printf 'TSP_DEMO_MODE=%s\n' "$DEMO"
  printf 'TSP_CLUSTER_MODE=false\n'
  printf 'TSP_TIMESERIES_BACKEND=postgres\n'
  printf 'TSP_POSTGIS_ENABLED=%s\n' "$POSTGIS"
  printf 'TSP_JT808_ADDR=127.0.0.1:18080\n'
  printf 'TSP_JT808_ALLOW_PLAINTEXT=%s\n' "$PLAINTEXT"
  printf 'TSP_JWT_SECRET=%s\n' "$(rand_hex 32)"
  printf 'TSP_JWT_KEY_ID=tsp-rs256-1\n'
  printf 'TSP_ADMIN_PASSWORD=%s\n' "$ADMIN"
  printf 'TSP_PII_KEY=%s\n' "$(rand_b64 32)"
  printf 'TSP_EXPORT_TOKEN_SECRET=%s\n' "$(rand_hex 32)"
  printf 'TSP_EXPORT_ENCRYPTION_KEY=%s\n' "$(rand_b64 32)"
  printf 'TSP_EXPORT_ENCRYPTION_KEY_ID=current\n'
  printf 'RUST_LOG=xingtu_tsp=info\n'
  if [ -n "$JWT_KEY" ]; then
    printf 'TSP_JWT_PRIVATE_KEY_FILE=%s\n' "$JWT_KEY"
    printf 'TSP_JWT_PUBLIC_KEY_FILE=%s\n' "$JWT_PUB"
    printf 'TSP_JT808_TLS_CERT=%s\n' "$JT808_CERT"
    printf 'TSP_JT808_TLS_KEY=%s\n' "$JT808_KEY"
  fi
} > "$ENV_PATH"

chmod 600 "$ENV_PATH"
chmod 700 "$SECRET_DIR"

echo "TSP initialization complete"
echo "  env:      $ENV_PATH"
echo "  data:     $DATA_PATH"
echo "  database: $DB"
if [ "$PRODUCTION" -eq 1 ]; then
  echo "  admin:    admin / $ADMIN"
  echo "  store this password in a secret manager"
else
  echo "  demo:     admin / admin123"
fi
echo "  secrets:  $SECRET_DIR"
echo
echo "Next: cargo run --release"

if [ "$BUILD" -eq 1 ]; then
  cargo build --release
fi
if [ "$RUN" -eq 1 ]; then
  set -a
  # shellcheck disable=SC1090
  . "$ENV_PATH"
  set +a
  cargo run --release
fi
