#!/bin/sh
set -eu

APP_UID="${APP_UID:-10001}"
APP_GID="${APP_GID:-999}"

ensure_dir() {
  d="$1"
  if [ ! -d "$d" ]; then
    mkdir -p "$d"
  fi
}

ensure_dir /app/data
ensure_dir /app/signals
ensure_dir /app/state
ensure_dir /app/state/state
ensure_dir /app/output
ensure_dir /app/logs

if [ "$(id -u)" = "0" ]; then
  # Host bind mounts may be created as root; normalize ownership/permissions on boot.
  chown -R "${APP_UID}:${APP_GID}" /app/data /app/signals /app/state /app/output /app/logs || true
  chmod -R u+rwX,g+rwX /app/data /app/signals /app/state /app/output /app/logs || true
  if [ "$#" -eq 0 ]; then
    exec gosu "${APP_UID}:${APP_GID}" /usr/local/bin/polybot_convert_rust
  fi
  exec gosu "${APP_UID}:${APP_GID}" "$@"
fi

if [ "$#" -eq 0 ]; then
  exec /usr/local/bin/polybot_convert_rust
fi
exec "$@"
