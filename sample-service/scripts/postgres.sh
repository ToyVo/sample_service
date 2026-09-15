#!/usr/bin/env bash
# Local Postgres for sample-service development.
#
# Data lives in ./dev.postgresql/ (gitignored, alongside ./dev.db for SQLite).
# Uses port 5433 (not 5432) to avoid clashing with any system Postgres, stores
# its Unix socket inside the data dir, and uses trust auth so no password is
# needed for local dev.
#
#   DATABASE_URL=postgresql://postgres@localhost:5433/sample
#
# Usage:
#   ./scripts/postgres.sh start|stop|restart|status|logs|clean
#
# Requires Postgres binaries (initdb, pg_ctl, psql, createdb, pg_isready).
# They are provided by the nix dev shell (`nix develop` adds `postgresql`);
# if missing, the script tells you how to get them.
set -euo pipefail

SERVICE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="$SERVICE_DIR/dev.postgresql"
LOG_FILE="$DATA_DIR/logfile"
PORT="5433"
SUPERUSER="postgres"
DB_NAME="sample"
export DATABASE_URL="postgresql://$SUPERUSER@localhost:$PORT/$DB_NAME"

need_bin() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: '$1' not found in PATH." >&2
    echo "  Run inside the nix dev shell ('nix develop' from the repo root)," >&2
    echo "  which provides postgresql, or install Postgres 16+ another way." >&2
    exit 1
  fi
}

is_running() {
  pg_ctl -D "$DATA_DIR" status >/dev/null 2>&1
}

wait_ready() {
  local tries=0
  while ! pg_isready -h localhost -p "$PORT" -q; do
    tries=$((tries + 1))
    if [ "$tries" -ge 30 ]; then
      echo "error: Postgres did not become ready in time. See $LOG_FILE" >&2
      exit 1
    fi
    sleep 0.5
  done
}

ensure_db() {
  if psql -h localhost -p "$PORT" -U "$SUPERUSER" -d postgres -tAc \
    "SELECT 1 FROM pg_database WHERE datname='$DB_NAME'" | grep -q 1; then
    return 0
  fi
  echo "Creating database '$DB_NAME'..."
  createdb -h localhost -p "$PORT" -U "$SUPERUSER" "$DB_NAME"
}

cmd_init() {
  need_bin initdb
  if [ ! -f "$DATA_DIR/PG_VERSION" ]; then
    echo "Initializing Postgres data dir: $DATA_DIR"
    mkdir -p "$DATA_DIR"
    # Trust local connections so no password is needed for dev.
    # --no-locale avoids macOS locale warnings; UTF8 encoding throughout.
    initdb -D "$DATA_DIR" -U "$SUPERUSER" --auth=trust -E UTF8 --no-locale
  fi
}

cmd_start() {
  need_bin pg_ctl
  need_bin pg_isready
  need_bin psql
  need_bin createdb
  cmd_init
  if is_running; then
    echo "Postgres already running (data dir $DATA_DIR)."
  else
    echo "Starting Postgres (data dir $DATA_DIR, port $PORT)..."
    # -p overrides the port from postgresql.conf; -k puts the Unix socket
    # inside the data dir so it can't clash with a system Postgres.
    pg_ctl -D "$DATA_DIR" -l "$LOG_FILE" -o "-p $PORT -k \"$DATA_DIR\"" start
  fi
  wait_ready
  ensure_db
  echo "Postgres ready."
  echo "DATABASE_URL=$DATABASE_URL"
  echo "  SQLite dev DB is still ./dev.db — pick a backend via DATABASE_URL."
}

cmd_stop() {
  need_bin pg_ctl
  if [ ! -f "$DATA_DIR/PG_VERSION" ]; then
    echo "Nothing to stop (no data dir at $DATA_DIR)."
    return 0
  fi
  if is_running; then
    echo "Stopping Postgres..."
    pg_ctl -D "$DATA_DIR" stop -m fast
  else
    echo "Postgres already stopped."
  fi
}

cmd_status() {
  need_bin pg_ctl
  need_bin pg_isready
  if [ ! -f "$DATA_DIR/PG_VERSION" ]; then
    echo "Not initialized (no $DATA_DIR yet — run '$0 start')."
    exit 1
  fi
  pg_ctl -D "$DATA_DIR" status || true
  pg_isready -h localhost -p "$PORT" || true
  echo "DATABASE_URL=$DATABASE_URL"
}

cmd_logs() {
  if [ ! -f "$LOG_FILE" ]; then
    echo "No logfile yet at $LOG_FILE (run '$0 start' first)." >&2
    exit 1
  fi
  tail -f "$LOG_FILE"
}

cmd_clean() {
  cmd_stop || true
  if [ -d "$DATA_DIR" ]; then
    echo "Removing $DATA_DIR..."
    rm -rf "$DATA_DIR"
  fi
  echo "Clean. Next '$0 start' will re-init a fresh database."
}

case "${1:-start}" in
  start) cmd_start ;;
  stop) cmd_stop ;;
  restart)
    cmd_stop || true
    cmd_start
    ;;
  status) cmd_status ;;
  logs) cmd_logs ;;
  clean) cmd_clean ;;
  *)
    echo "Usage: $0 start|stop|restart|status|logs|clean" >&2
    exit 1
    ;;
esac
