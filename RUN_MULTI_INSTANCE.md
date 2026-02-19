# Run Multiple OS-Level Instances (Simple)

This setup lets you run many instances with separate env/state/data directories.

## 1) Build once

```bash
sudo useradd -r -m -d /home/polybot -s /usr/sbin/nologin polybot || true
cd /opt/polybot
cargo build --release --locked
```

## 2) Make scripts executable

```bash
chmod +x /opt/polybot/scripts/create-instance.sh
chmod +x /opt/polybot/scripts/run-instance.sh
```

## 3) Create instances

```bash
/opt/polybot/scripts/create-instance.sh bot-a 2
/opt/polybot/scripts/create-instance.sh bot-b 3
```

Each instance gets:

- `/opt/polybot/instances/<name>/.env`
- `/opt/polybot/instances/<name>/service.env`
- `/opt/polybot/instances/<name>/{state,data,output,logs,signals}`

`BOT_ID` is automatically set to the instance name during creation.

## 4) Edit each env

At minimum, set market/strategy per instance:

- `/opt/polybot/instances/bot-a/.env`
- `/opt/polybot/instances/bot-b/.env`

## 5) Install systemd unit

```bash
sudo cp /opt/polybot/deploy/systemd/polybot@.service /etc/systemd/system/
sudo chown -R polybot:polybot /opt/polybot
sudo systemctl daemon-reload
```

## 6) Start and enable instances

```bash
sudo systemctl enable --now polybot@bot-a
sudo systemctl enable --now polybot@bot-b
```

## 7) Useful commands

```bash
sudo systemctl status polybot@bot-a
sudo journalctl -u polybot@bot-a -f
sudo systemctl restart polybot@bot-a
sudo systemctl stop polybot@bot-b
```

If your repo path is not `/opt/polybot`, set an override:

```bash
echo "POLYBOT_ROOT=/your/path/polybot" | sudo tee /etc/default/polybot
sudo systemctl daemon-reload
sudo systemctl restart polybot@bot-a
```
