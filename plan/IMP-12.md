# IMP-12 Plan: Versioned Env Snapshots and Rollback-Safe Config Activation

## Summary
Implement `REQ-023` by turning the current env-loaded config into one authoritative, versioned runtime snapshot.

Chosen decisions:
- env remains the source of truth for `IMP-12`
- a newly loaded valid config activates on the next market, not mid-market
- the existing `configuration` table is evolved into the persisted snapshot store instead of creating a parallel table now

The goal is to make every active trade run under an immutable `config_version`, persist the exact loaded snapshot, and guarantee that a bad reload never displaces the last good version.

## Key Changes
### 1. Unified versioned config snapshot
- Add a new typed snapshot in `src/config.rs`, e.g. `VersionedConfigSnapshotV1`, containing:
  - base `BotConfig`
  - BOT runtime policy config (`BotRuntimeConfigSnapshot`)
  - any runtime-affecting execution/adapter overrides currently read outside `BotConfig::from_env()`
  - metadata: `config_version`, `config_hash`, `loaded_at`, `schema_version`, `source = env`
- Build this snapshot from env in one place.
- Stop treating `BotConfig::from_env()`, `bot_runtime_config_from_env()`, and `_apply_cfg_overrides_from_env()` as independent live sources.
- Persist a sanitized canonical JSON snapshot text.
  - secrets must be excluded from persisted `config_text`
  - `config_version` is a deterministic human-visible identifier derived from the sanitized snapshot
  - `config_hash` remains the full canonical hash for dedupe

### 2. DB and load-path changes
- Extend the existing `configuration` table with:
  - `config_version`
  - `config_text`
  - `loaded_at`
- Keep `configuration_id` for backward compatibility, but make `config_version` the authoritative runtime identifier.
- Change `upsert_configuration(...)` to dedupe on the full sanitized snapshot, not only the old flat `BotConfig` fields.
- Change config reconstruction in `main.rs` to prefer `config_text` JSON hydration.
  - legacy rows without `config_text` continue to hydrate from the current flat columns as a fallback
- Add `config_version` to the active trade record so each trade is pinned to one version from start to finish.

### 3. Activation and rollback-safe reload
- Introduce one config loader/manager in `main.rs` that:
  - loads env
  - builds the full snapshot
  - validates the full snapshot
  - persists it if new
  - returns the active `config_version` plus resolved runtime config bundle
- Reload checks happen before starting a new market/trade.
- If reload parsing or validation fails:
  - keep the previous active version unchanged
  - do not partially activate any new settings
  - emit a clear structured reload failure log
- Once a market starts, that bot instance keeps its version fixed until the trade is over.
- Remove live env mutation after bot construction as an authoritative path; bot init should receive the already-resolved versioned snapshot.

### 4. Propagation to decisions, orders, and fills
- Extend current persisted/audited surfaces with explicit `config_version`:
  - `trade`
  - `trade_decisions`
  - existing order/fill execution records and latency/submit context payloads
- Extend `TradeDecisionUpsert` and execution-context tracking so every decision, submit, ack, and fill can be tied back to one `config_version`.
- Use the same pinned trade-level `config_version` for all artifacts produced during that trade.
- Keep `configuration_id` alongside `config_version` only where existing code still depends on it.

## Important Interface Changes
- New versioned snapshot type in `src/config.rs` for the full effective config.
- `ConfigurationRow` gains `config_version`, `config_text`, and `loaded_at`.
- `TradeDecisionUpsert` gains `config_version`.
- Trade persistence gains `config_version`.
- Bot construction should accept the resolved versioned config bundle instead of re-reading env internally.

## Test Plan
- Snapshot tests:
  - canonical snapshot JSON is stable
  - identical effective config reuses the same `config_version`
  - changing a runtime-only BOT policy knob creates a new version
  - secrets are excluded from persisted `config_text`
- DB tests:
  - `upsert_configuration` dedupes full snapshots, not just old flat fields
  - legacy configuration rows still hydrate correctly without `config_text`
- Reload tests:
  - valid env change persists a new version but activates only on the next market
  - invalid reload leaves the old version active
  - active trades keep their original version even if env changes mid-run
- Propagation tests:
  - trade row stores `config_version`
  - trade decision upserts store `config_version`
  - submit/fill execution records include `config_version`
  - all artifacts from one trade share the same pinned version

## Assumptions and Defaults
- `IMP-12` does not introduce TOML or an admin reload API yet; env is still the authoritative source.
- Next-market activation is the only supported reload mode in this task.
- Secret-only changes do not produce a new persisted `config_version`.
- No new dedicated order/fill DB tables are added in `IMP-12`; current persisted execution artifacts get `config_version`, and fuller append-only event logging remains `IMP-13`.
