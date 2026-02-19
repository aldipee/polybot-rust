#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <instance-name> [cpu-set]"
  echo "Example: $0 bot-a 2"
  exit 1
fi

instance_name="$1"
cpu_set="${2:-}"

root_dir="${POLYBOT_ROOT:-/opt/polybot}"
template_env="${POLYBOT_TEMPLATE_ENV:-$root_dir/.env}"
instance_dir="$root_dir/instances/$instance_name"

mkdir -p \
  "$instance_dir/state" \
  "$instance_dir/data" \
  "$instance_dir/output" \
  "$instance_dir/logs" \
  "$instance_dir/signals"

if [[ -f "$template_env" && ! -f "$instance_dir/.env" ]]; then
  cp "$template_env" "$instance_dir/.env"
fi

if [[ ! -f "$instance_dir/.env" ]]; then
  cat > "$instance_dir/.env" <<'EOF'
# Required minimum:
# POLYMARKET_PRIVATE_KEY=0x...
# POLYMARKET_FUNDER=0x...
# BOT_ID=instance-name
# MARKET_SLUG=...
EOF
fi

if grep -q '^BOT_ID=' "$instance_dir/.env"; then
  sed -i "s/^BOT_ID=.*/BOT_ID=$instance_name/" "$instance_dir/.env"
else
  printf "\nBOT_ID=%s\n" "$instance_name" >> "$instance_dir/.env"
fi

if [[ ! -f "$instance_dir/service.env" ]]; then
  cat > "$instance_dir/service.env" <<EOF
# Optional, used by scripts/run-instance.sh
# POLYBOT_CPUSET=2
POLYBOT_CPUSET=${cpu_set}
EOF
fi

echo "Instance created: $instance_name"
echo "Path: $instance_dir"
echo "Next:"
echo "  1) Edit $instance_dir/.env and set strategy vars (BOT_ID already set)"
echo "  2) sudo cp $root_dir/deploy/systemd/polybot@.service /etc/systemd/system/"
echo "  3) sudo systemctl daemon-reload"
echo "  4) sudo systemctl enable --now polybot@$instance_name"
