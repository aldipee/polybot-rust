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
`DB_URL` from `.env` is ignored by the runner unless you set `POLYBOT_DB_URL` in `service.env`.

## 4) Edit each env

At minimum, set market/strategy per instance:

- `/root/polybot-rust/instances/bot-a/.env`
- `/root/polybot-rust/instances/bot-b/.env`

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
journalctl -u polybot@bot-a -f
systemctl restart polybot@bot-a
systemctl stop polybot@bot-b
```

If your repo path is not `/root/polybot-rust`, set an override:

```bash
echo "POLYBOT_ROOT=/your/path/polybot-rust" > /etc/default/polybot
systemctl daemon-reload
systemctl restart polybot@bot-a
```
