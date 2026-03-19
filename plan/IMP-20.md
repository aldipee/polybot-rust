# IMP-20 Plan: Dual-Threshold Stale-Data Enforcement

## Summary
Implement `REQ-033` with two explicit market-data thresholds:

- `MARKET_DATA_STALE_ADD_BLOCK_SECONDS`, default `2`
- `MARKET_DATA_STALE_HARD_PAUSE_SECONDS`, default `5`

Chosen product decisions:
- relaxed overrides like `3 / 6` are allowed
- any value other than `2 / 5` is treated as requirement-noncompliant and must emit an explicit warning/metric
- the old single env `MARKET_DATA_STALE_SECONDS` is no longer accepted; config load should fail fast if it is present

Behavior:
- `>= add_block` and `< hard_pause`: block all new BOT order creation and repost logic, but do not cancel working orders
- `>= hard_pause`: cancel BOT working orders and enter the existing paused/reconciliation recovery path
- recovery from hard stale uses the existing `IMP-15` gate: data must be healthy again, then reconciliation must pass before new risk resumes

## Key Changes
### Config and versioned snapshot
- Add two new stale-data fields to the effective config surface and persist them through the `IMP-12` versioned snapshot path.
- Keep old persisted snapshots compatible by making the new fields additive with serde defaults of `2` and `5`.
- Keep the old `market_data_stale_seconds` field only as a deprecated snapshot-compat field for now; runtime stale enforcement must stop reading it.
- Fail config load with a clear error if `MARKET_DATA_STALE_SECONDS` is set in env.
- Validate:
  - both values finite and `> 0`
  - `add_block < hard_pause`
- Emit a startup/config warning when thresholds differ from `2 / 5`, for example `stale_policy_noncompliant add_block=3 hard_pause=6 expected=2/5`.

### Runtime stale classification and enforcement
- Add one shared helper that computes pair stale status from the worst YES/NO quote age:
  - `Fresh`
  - `AddBlocked(age_seconds)`
  - `HardPaused(age_seconds)`
- Missing quote or invalid quote timestamp counts as stale once the runtime is outside `PreArm`.
- In `OpenBoth`, `AwaitSecondFill`, `PairBuild`, and `Taper`:
  - `AddBlocked`: skip all BOT create/repost paths and emit a stable risk-block reason such as `market_data_stale_add_block`
  - `HardPaused`: enter `DependencyPaused` with reason `dependency_pause:market_data_stale`, cancel BOT working families via the existing pause-time cancel path, and keep the pause latched until fresh data plus reconciliation clear it
- `AwaitSettlement` remains allowed during hard stale so cleanup and settlement handoff still work.
- Hard stale should reuse the existing `IMP-15` recovery flow instead of inventing a parallel resume path.

### Logging, audit, and metrics
- Add stale-threshold values and a derived `stale_policy_requirement_compliant` flag to startup/effective-config logging.
- Include `stale_age_seconds` and `stale_stage` in runtime risk-block / pause audit payloads.
- Distinguish the two operational states in logs and audit:
  - `market_data_stale_add_block`
  - `dependency_pause:market_data_stale`
- Keep the implementation pair-local; do not add the broader “systemic stale grace -> global kill” behavior in this task.

### Compatibility and scope boundaries
- `STALE_SECONDS` remains untouched; it is still the existing order-management staleness knob and is not part of `IMP-20`.
- No active-market 5-second reconciliation loop is added here.
- No live-mode rollout gating or paper/shadow behavior is added here.

## Important Interfaces
- New env keys:
  - `MARKET_DATA_STALE_ADD_BLOCK_SECONDS`
  - `MARKET_DATA_STALE_HARD_PAUSE_SECONDS`
- Deprecated/unsupported env:
  - `MARKET_DATA_STALE_SECONDS`
- New runtime helper/type:
  - a small stale-status enum or equivalent shared classifier for `Fresh / AddBlocked / HardPaused`

## Test Plan
- Config defaults resolve to `2 / 5`.
- Invalid config rejects:
  - non-finite values
  - non-positive values
  - `add_block >= hard_pause`
- Presence of `MARKET_DATA_STALE_SECONDS` fails config load explicitly.
- Old persisted `config_text` snapshots without the new fields still load and resolve to `2 / 5`.
- Relaxed `3 / 6` config is accepted and emits the noncompliant warning/flag.
- Quote age just above add-block:
  - blocks `OpenBoth` / `AwaitSecondFill` / `PairBuild` / `Taper` creates
  - does not cancel working BOT orders
  - does not enter `DependencyPaused`
- Quote age just above hard-pause:
  - cancels BOT working orders
  - enters `DependencyPaused`
  - does not resume until quotes are fresh again and reconciliation succeeds
- `AwaitSettlement` still runs during hard stale.
- Existing `IMP-15` dependency-pause and reconciliation behavior remains unchanged for websocket disconnects and persistence failures.

## Assumptions and Defaults
- Relaxed thresholds like `3 / 6` are intentionally allowed, but they are treated as requirement-noncompliant operator overrides.
- Warning stale blocks all new BOT buy/order-create activity, including repair and second-side completion creates; it does not cancel existing orders.
- Hard stale is a pair-safe pause and cancel condition, not a new global kill mechanism.
- The versioned config schema can stay additive if the new stale-threshold fields use serde defaults for old snapshots.
