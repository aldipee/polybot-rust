# Python to Rust Library Map (for `main.py`)

This is the direct mapping used for the native Rust migration plan.

| Python import / package | Rust equivalent | Notes |
|---|---|---|
| `requests` | `reqwest` | HTTP calls (Gamma/CLOB REST) |
| `websocket.WebSocketApp` (`websocket-client`) | `tokio-tungstenite` | WS feed + user stream |
| `dotenv.load_dotenv` (`python-dotenv`) | `dotenvy` | Same `.env` loading behavior |
| `logging`, `loguru` | `tracing`, `tracing-subscriber`, `tracing-appender` | Structured logs + file rotation setup |
| `json` | `serde_json` | JSON encode/decode |
| `csv` | `csv` | Append-only CSV service |
| `re` | `regex` | Slug parsing / validation |
| `datetime`, `timezone`, `timedelta` | `chrono` | Time arithmetic |
| `zoneinfo.ZoneInfo` | `chrono-tz` | Timezone handling (`America/New_York`) |
| `dataclasses` | Rust structs + `serde` derives | Typed config/events |
| `decimal.Decimal` | `rust_decimal` | Deterministic rounding and quantization |
| `threading` + `deque` | `tokio`/`std::thread` + `VecDeque` | Inbox and background WS workers |
| `signal` | `tokio::signal` or `ctrlc` | Graceful shutdown |
| `ssl` | `rustls` + `tokio-rustls` | TLS options for WS/HTTP |
| `py_clob_client` | custom CLOB client (`reqwest` + EIP-712 signing) | No mature 1:1 Rust crate |
| `sqlalchemy` | `sqlx` or `sea-orm` | DB engine + repository layer |
| `db.session`, `db.repository`, `db.utils` (project-local) | local Rust modules (`db/session.rs`, `db/repository.rs`, `db/utils.rs`) | Keep same DB contract and metrics writes |

## Suggested Rust Crates (full native port)

```toml
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"
csv = "1"
dotenvy = "0.15"
regex = "1"
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
rust_decimal = { version = "1", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "sqlite", "postgres", "chrono"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "time"] }
tokio-tungstenite = { version = "0.24", features = ["rustls-tls-native-roots"] }
tracing = "0.1"
tracing-appender = "0.2"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
```

