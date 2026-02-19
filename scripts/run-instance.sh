#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <instance-name>"
  exit 1
fi

instance_name="$1"
root_dir="${POLYBOT_ROOT:-/opt/polybot}"
instance_dir="$root_dir/instances/$instance_name"
bin_path="${POLYBOT_BIN:-$root_dir/target/release/polybot_convert_rust}"
env_file="$instance_dir/.env"
service_env_file="$instance_dir/service.env"

if [[ ! -x "$bin_path" ]]; then
  echo "Binary not found or not executable: $bin_path"
  echo "Build first: cargo build --release --locked"
  exit 1
fi

if [[ ! -f "$env_file" ]]; then
  echo "Missing env file: $env_file"
  exit 1
fi

mkdir -p \
  "$instance_dir/state" \
  "$instance_dir/data" \
  "$instance_dir/output" \
  "$instance_dir/logs" \
  "$instance_dir/signals"

if [[ -f "$service_env_file" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$service_env_file"
  set +a
fi

set -a
# shellcheck disable=SC1090
source "$env_file"
set +a

export DB_URL="${DB_URL:-sqlite:///$instance_dir/data/bot.sqlite3}"
export LOG_DIR="${LOG_DIR:-$instance_dir/output}"
export EXEC_LATENCY_LOG_DIR="${EXEC_LATENCY_LOG_DIR:-$instance_dir/logs}"
export SIGNAL_FILE_DIR="${SIGNAL_FILE_DIR:-$instance_dir/signals}"

cd "$instance_dir/state"

if [[ -n "${POLYBOT_CPUSET:-}" ]]; then
  exec taskset -c "$POLYBOT_CPUSET" "$bin_path"
fi

exec "$bin_path"
