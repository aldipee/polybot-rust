# Run Multiple OS-Level Instances (Simple)

This setup lets you run many instances with separate env/state/data directories.

## 1) Build once

```bash
cd /root/polybot-rust
cargo build --release --locked
```

## 2) Make scripts executable

```bash
chmod +x /root/polybot-rust/scripts/create-instance.sh
chmod +x /root/polybot-rust/scripts/run-instance.sh
sed -i 's/\r$//' /root/polybot-rust/scripts/create-instance.sh /root/polybot-rust/scripts/run-instance.sh
```

## 3) Create instances

```bash
/root/polybot-rust/scripts/create-instance.sh bot-a 2
/root/polybot-rust/scripts/create-instance.sh bot-b 3
```

Each instance gets:

- `/root/polybot-rust/instances/<name>/.env`
- `/root/polybot-rust/instances/<name>/service.env`
- `/root/polybot-rust/instances/<name>/{state,data,output,logs,signals}`

`BOT_ID` is automatically set to the instance name during creation.
`DB_URL` from `.env` is used by default. You can override per instance via `POLYBOT_DB_URL` in `service.env`.

## 4) Edit each env

At minimum, set market/strategy per instance:

- `/root/polybot-rust/instances/bot-a/.env`
- `/root/polybot-rust/instances/bot-b/.env`

Market targeting options:

- Explicit slug: set `MARKET_SLUG=btc-updown-5m-1771642500`
- Auto slug: leave `MARKET_SLUG` empty and set `MARKET_SYMBOL=btc` (or `RTDS_SYMBOL=btc/usd`) plus `MARKET_SEGMENT`  
  Startup will generate `<asset>-updown-<segment>-<slot_ts>` from current time.

## 5) Install systemd unit

```bash
cp /root/polybot-rust/deploy/systemd/polybot@.service /etc/systemd/system/
systemctl daemon-reload
```

## 6) Start and enable instances

```bash
systemctl enable --now polybot@bot-a
systemctl enable --now polybot@bot-b
```

## 7) Useful commands

```bash
systemctl status polybot@bot-a
journalctl -u polybot@bot-a -f -n 100
systemctl restart polybot@bot-a
systemctl stop polybot@bot-b
systemctl list-units --type=service --state=running
```

If you see `status=203/EXEC` or `Permission denied` on `run-instance.sh`:

```bash
chmod 755 /root/polybot-rust/scripts/create-instance.sh /root/polybot-rust/scripts/run-instance.sh
sed -i 's/\r$//' /root/polybot-rust/scripts/create-instance.sh /root/polybot-rust/scripts/run-instance.sh
cp /root/polybot-rust/deploy/systemd/polybot@.service /etc/systemd/system/
systemctl daemon-reload
systemctl reset-failed polybot@bot-a
systemctl restart polybot@bot-a
journalctl -u polybot@bot-a -n 120 -f
```

If your repo path is not `/root/polybot-rust`, set an override:

```bash
echo "POLYBOT_ROOT=/your/path/polybot-rust" > /etc/default/polybot
systemctl daemon-reload
systemctl restart polybot@bot-a
```
