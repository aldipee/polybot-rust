## 1. Executive translation of the analysis into system intent

* The product is a **paired-inventory accumulator** for **BTC 5-minute Up/Down markets**, not a price-prediction bot.
* The bot’s core unit of trading is a **market pair**, not a single contract. Up and Down are managed as one position object.
* The primary edge is **pair-pricing discipline**: buy both sides only when the next matched unit is cheap enough, with best performance in the **< 0.94** zone and acceptable behavior in the **< 0.97** zone.
* The bot must treat **inventory balance as alpha protection**. Unmatched inventory is not opportunity; it is risk leakage.
* The bot must **seed both sides quickly**, with submission almost simultaneously and second-side completion targeted inside **15 seconds**, hard limit **30 seconds**.
* The bot must **not scale** until both sides have at least one fill. Small seed first, then controlled accumulation.
* The bot must be **maker-first**. Taker flow is a restricted exception for pair completion or imbalance repair, not normal behavior.
* The bot must **hold to settlement by default**. Intrawindow flipping is not the product.
* The bot must explicitly forbid the losing behaviors seen in the source behavior: **adds at pair cost >= 1.00**, **dangerous adds at >= 1.03**, **large unmatched residuals**, **cheap-side overweighting**, **accidental single-side carry**, and **hidden directional drift**.
* The bot may support the model-edge field only as an **optional overlay**, never as the core engine.
* The success definition is behavioral first: **paired entry, price discipline, low taker share, low imbalance, accurate settlement accounting, and positive paired PnL decomposition**.
* MVP must stay narrow: **single venue, single strategy, buy-only, one process, strong reconciliation, deterministic accounting, no BTC forecasting subsystem**.

---

## 2. Bot requirements specification

REQ-001 | P0 | strategy logic | The bot shall trade only BTC 5-minute Up/Down markets that pass strict registry validation and have an unambiguous Up/Down pair mapping. | The evidence only supports this exact market family. | Zero live or paper orders are emitted for markets outside the BTC 5-minute whitelist or for incomplete pairs.

REQ-002 | P0 | strategy logic | The bot shall model each Up/Down pair as a single paired instrument with shared risk, ledger, and state machine. | The edge is two-sided, not side-specific. | All strategy decisions reference `pair_id`; no order decision can be made from a single-side state object alone.

REQ-003 | P0 | strategy logic | The bot shall be buy-only in MVP and shall default to hold-to-settlement. | The observed profitable behavior is accumulate-and-hold; no meaningful sell behavior exists in the clean tape. | No sell orders are generated in MVP; settlement closes positions through exchange settlement flow only.

REQ-004 | P0 | market entry timing | The bot shall pre-warm market state before official open and begin strategy evaluation immediately when the pair becomes tradable. | Median first fill was 9 seconds after open; missing the first seconds loses the core behavior. | In paper and live shadow mode, the bot loads metadata and subscribes to both sides before open for 100% of tradable pairs.

REQ-005 | P0 | market entry timing | The bot shall submit seed orders on both sides within 5 seconds of confirmed market open or first tradable post-open event, whichever arrives first. | The source behavior seeds early and fast. | In replay and paper runs, 99% of entered pairs show first seed submission <= 5 seconds from open confirmation.

REQ-006 | P0 | pair completion timing | The bot shall submit first seed orders on both sides in the same scheduler cycle, with submission timestamps no more than 1 second apart. | Median gap between first Up and first Down fill was 4 seconds; the bot must not create self-inflicted asymmetry. | For every entered pair, seed order submit delta between sides is <= 1 second.

REQ-007 | P0 | pair completion timing | The bot shall not scale position size until both sides have at least one fill; if only one side fills, the bot enters completion mode and prioritizes the missing side. | The profitable behavior is paired. Single-side early scaling creates the leak. | Unit and replay tests show zero scale-up decisions before both sides are filled.

REQ-008 | P0 | pair completion timing | The bot shall target second-side completion within 15 seconds of first-side fill and treat 30 seconds as a hard deadline for new pair accumulation. | 82.98% completed within 15 seconds; 95.54% within 30 seconds. | In paper and shadow mode, no market remains in normal accumulation if second side is still missing after 30 seconds; it transitions to completion-only or pause.

REQ-009 | P0 | inventory balance control | The bot shall target unmatched inventory fraction below 7% and treat values above 7% as a throttle condition. | The best repeatable setup had unmatched inventory below 7%. | The strategy state switches out of normal accumulation whenever projected unmatched fraction exceeds 7%.

REQ-010 | P0 | inventory balance control | The bot shall treat unmatched inventory fraction above 12% as a warning state and 20% or more as a hard disable/reduce-only state. | Winners averaged 9.29%; losers averaged 12.26%; 20%+ is disallowed. | The risk engine blocks any new size-increasing order when projected unmatched fraction >= 20%; open orders are canceled and the pair is marked disabled.

REQ-011 | P0 | pricing thresholds | The bot shall classify add opportunities into price zones using effective marginal pair cost: preferred `< 0.94`, acceptable `0.94-<0.97`, caution `0.97-<1.00`, stop-add `>=1.00`, danger `>=1.03`. | PnL sharply degrades above 1.00 and becomes strongly negative above 1.03. | All order decisions carry a price zone reason code; zero size-increasing orders are emitted in stop-add or danger zones.

REQ-012 | P0 | pricing thresholds | The bot shall evaluate new balanced adds using marginal pair sum for the next matched unit, not only current book averages. | The edge comes from what the next matched unit costs. | For balanced adds, decision logs show `marginal_pair_sum = quote_up + quote_down`; for rebalance adds, they show residual-lot cost plus lagging-side quote.

REQ-013 | P0 | pricing thresholds | The bot shall block any new pair add or rebalance add that would create an effective marginal pair cost of 1.00 or more, except operator-approved emergency logic that is disabled in MVP. | Adds above 1.00 were losing behavior. | Automated tests confirm zero live intents with `effective_marginal_pair_sum >= 1.00` in MVP.

REQ-014 | P0 | execution style | The bot shall prefer passive maker orders by default and shall use post-only orders when supported by the venue. | Observed taker share was only 10.42%; profitable behavior is maker-first. | In paper and live, the default order intent is passive; all aggressive orders must carry an explicit exception reason.

REQ-015 | P0 | execution style | The bot shall target taker share below 5% and enforce a hard cap of 10% at market and daily aggregate levels. | The observed trader was profitable with low taker usage; the copy should be stricter. | Metrics and risk rules block new aggressive orders when market or day taker share reaches 10%.

REQ-016 | P0 | order sizing / clip logic | The bot shall use a discrete clip ladder with small seeds and capped larger adds; MVP defaults are `12`, `20`, `40`, `80` shares with `80` as the hard single-order cap. | Median fill size was 12; 80-share clips were dominant in volume. | No emitted order exceeds 80 shares; seed orders default to 12 unless overridden by config.

REQ-017 | P1 | order sizing / clip logic | The bot shall only escalate clip size above 20 when all green conditions hold: both sides filled, effective marginal pair sum < 0.94, projected unmatched fraction < 7%, time into market < 180 seconds, and budget available. | Large clips should copy the profitable pattern, not the sloppy one. | In replay and paper, every 40- or 80-share order is accompanied by a decision log proving all green conditions were true.

REQ-018 | P0 | residual directional control | The bot shall forbid intentional single-side trading outside the second-side completion path. | There was no meaningful Up-vs-Down directional edge. | Zero decisions of type `single_side_speculative_add` exist in MVP.

REQ-019 | P0 | residual directional control | The bot shall block any order that increases residual inventory on the cheaper/underdog side. | Cheap-side residual overweighting degraded results badly. | Decision logs show zero approved intents where `projected_residual_side == underdog_side` and residual magnitude increases.

REQ-020 | P1 | residual directional control | If the optional model-edge overlay is enabled, one-sided catch-up or directional exceptions shall require `edge_model_minus_price > +0.02` on the added side and must still satisfy all inventory and price guards. | Positive model edge was helpful, but secondary. | Feature-flag tests confirm the overlay is off by default; when on, no one-sided add passes with edge <= 0.02.

REQ-021 | P0 | settlement handling | The bot shall stop initiating new exposure late in the window, switch to balance-only after 225 seconds, and fully stop new orders after 240 seconds. | Median last fill was around 233 seconds; the bot should stop before the tail risk zone. | No order create events exist after 240 seconds into market time.

REQ-022 | P0 | settlement handling | The bot shall cancel all working orders before close and hold any remaining inventory through official settlement and reconciliation. | The strategy is not an intrawindow exit strategy. | At or before the configured hard cutoff, open order count for the pair goes to zero; positions remain until settlement.

REQ-023 | P0 | config management | The bot shall load all thresholds, budgets, feature flags, and adapter settings from versioned config, persist every loaded config snapshot, and require explicit config versions in decision logs. | The strategy is threshold-driven and must be auditable. | Every order, fill, and decision row references a `config_version`; hot reload keeps the previous version active on parse failure.

REQ-024 | P0 | observability | The bot shall emit structured logs and metrics for every state transition, risk block, order intent, order ack, fill, reconciliation event, and settlement event. | This strategy must prove discipline, not just PnL. | Dashboards show pair-price zone, imbalance, taker share, second-side timing, and reconciliation status per active pair.

REQ-025 | P0 | observability | The bot shall persist strategy decision logs with all load-bearing inputs: time into market, time remaining, marginal pair sum, combined average paid, unmatched fraction, match ratio, favorite/underdog flag, taker share, and reason code. | Future audits need exact causal records. | For any live order, an operator can trace back to a single persisted decision event with full input state.

REQ-026 | P0 | replay/backtesting | The bot shall support deterministic replay from captured exchange event logs using the same strategy, risk, and ledger code paths as live mode. | The historical analysis files are good for calibration, but not enough for execution-accurate backtests. | Running replay twice on the same event log produces identical decisions, orders, and ledger outcomes.

REQ-027 | P0 | replay/backtesting | The bot shall include an analysis-import path that ingests the provided parquet/csv/schema fields exactly and reproduces the reference metrics used to set the strategy rules. | The analytics pipeline must match the evidence before the live bot can be trusted. | The importer reproduces the reference counts and metrics within defined tolerances, including pair counts, two-sided rate, taker share, and price-zone summaries.

REQ-028 | P0 | paper trading | The bot shall support a paper adapter that uses live market data and the same decision engine as live mode, differing only in execution/fill simulation. | Paper trading is the lowest-risk way to validate behavior. | Paper mode runs without code-path forks in strategy or risk modules; only the adapter implementation changes.

REQ-029 | P0 | live trading safeguards | Live mode shall start in `shadow` or `paper` by default and require explicit operator enablement, clean startup reconciliation, and healthy market data before any real order is sent. | The main failure mode is accidental directional exposure. | With default config, the bot sends zero live orders until `live.enabled=true` and startup checks pass.

REQ-030 | P0 | error handling | The bot shall use idempotent client order IDs, deduplicate fills, reject stale data, and pause trading on adapter, database, or reconciliation failures. | This strategy cannot survive orphan orders or missing fills. | Integration tests prove duplicate fill events do not change inventory twice; DB failure or unreconciled adapter failure halts new trading.

REQ-031 | P0 | reconciliation | The bot shall reconcile open orders, fills, positions, and settlement outcome on startup, every 5 seconds during active markets, after reconnect, and after settlement. | Paired inventory only works if local and venue state match. | Any unresolved mismatch triggers pair pause; startup stays in safe mode until reconciliation is clean.

REQ-032 | P1 | live trading safeguards | The bot shall enforce configurable per-market and portfolio-level gross cost caps before approving any order. | The analysis supports behavior, not account-size assumptions. | No market or portfolio exposure exceeds configured limits in live, paper, or replay runs.

REQ-033 | P1 | error handling | The bot shall treat market data older than 2 seconds as stale for new adds and older than 5 seconds as a hard pause/cancel condition. | Fast two-side coordination requires fresh data. | In integration tests, stale feed injection pauses new orders at 2 seconds and cancels working orders at 5 seconds.

REQ-034 | P1 | execution style | The bot shall cap quote refresh activity to a conservative default of one amend/cancel-replace cycle per side per second until exchange rate limits are confirmed. | The market is short-lived, but API unknowns must not cause self-throttling failures. | Scheduler logs show no side exceeds the configured refresh cap.

---

## 3. Rust system architecture

### Deployment model

MVP should be a **single Rust service** with internal actor-style components, not a distributed system.

Reason:

* The key coordination problem is **inside one pair over 15-30 seconds**.
* The strategy is short-horizon and event-driven.
* A single binary reduces failure modes and makes restart reconciliation simpler.

### Workspace layout

```text
pairbot/
  Cargo.toml
  rust-toolchain.toml
  crates/
    domain/
    config/
    market_registry/
    ledger/
    risk_engine/
    strategy_paired/
    execution/
    exchange_core/
    adapter_polymarket/
    adapter_paper/
    persistence/
    replay/
    service/
    admin_api/
  tools/
    analysis_importer/
    dry_run/
    db_migrate/
  config/
    base.toml
    paper.toml
    live.toml
  migrations/
  dashboards/
  deploy/
  runbooks/
  tests/
    integration/
    fixtures/
```

### Major components and ownership

**`domain`**

* Owns core types and invariants.
* Newtypes for `Price`, `Qty`, `Cost`, `PairId`, `MarketId`, `ClientOrderId`, `FillIdentityKey`.
* Enums for `OutcomeSide`, `MarketPhase`, `RiskAction`, `PriceZone`, `OrderIntentKind`, `SettlementStatus`.
* No exchange code. No database code.

**`config`**

* Owns typed config schema, defaults, feature flags, validation.
* Supports runtime reload with version stamps.
* Emits `ConfigVersion` and a config hash for persistence.

**`market_registry`**

* Owns pair discovery and canonical mapping of Up/Down markets.
* Maintains schedule fields: `opens_at`, `closes_at`, expected settlement timing.
* Only exposes fully validated `MarketPair` objects.

**`ledger`**

* Owns deterministic accounting, fill dedupe, FIFO lot pairing, inventory math, paired PnL decomposition, and residual PnL decomposition.
* This is where “paired engine vs residual leak” becomes code.
* Pure logic crate plus serialization helpers.

**`risk_engine`**

* Owns threshold evaluation and approval/block logic.
* Stateless rules over current `PairSnapshot`, config, and proposed `OrderIntent`.
* Produces `RiskDecision` with explicit reason codes.

**`strategy_paired`**

* Owns per-pair state machine and strategy policy.
* Consumes normalized market events and fill events.
* Produces `OrderIntent`s only; it never talks directly to the exchange.

**`execution`**

* Owns quote scheduling, clip ladder selection, order lifecycle state, replace/cancel rules, and routing into the adapter.
* Maintains one working order per side in MVP, plus at most one catch-up order.

**`exchange_core`**

* Owns common adapter traits and normalized event/command models.
* No venue details.

**`adapter_polymarket`**

* Production adapter.
* Owns websocket subscriptions, REST sync, auth/signing, order placement, cancellation, open-order fetch, fill ingest, settlement ingest.
* Must normalize exchange semantics into the `exchange_core` types.

**`adapter_paper`**

* Paper execution adapter.
* Uses the same `OrderCommand` inputs and emits the same `ExecutionReport` outputs.
* Conservative fill model only.

**`persistence`**

* Owns SQL schema, event writes, snapshot writes, recovery loads, and reconciliation storage.
* Postgres in production; SQLite optional for local tests.

**`replay`**

* Owns raw event capture replay, deterministic clock, and scenario runner.
* Uses the same strategy, risk, ledger, and execution scheduling code paths as live.

**`service`**

* Wires the whole application.
* Starts the async runtime, supervisors, channel topology, adapter tasks, market actors, reconciler, and admin endpoints.

**`admin_api`**

* Owns health endpoints, metrics serving, read-only market inspection, config reload, per-pair disable, global kill switch.

### Communication model

Use `tokio` with explicit channels.

* `broadcast<MarketEvent>` from adapter to registry and market actors
* `mpsc<OrderIntent>` from market actor to risk engine / order manager
* `broadcast<ExecutionReport>` from adapter to market actors and ledger persistence
* `watch<ConfigSnapshot>` for hot config updates
* `mpsc<AdminCommand>` for manual actions
* `mpsc<ReconCommand>` for startup, periodic, and post-settlement reconciliation

Per active pair, run a dedicated `MarketActor`.

* The actor owns its **ephemeral state** for that pair.
* It serializes state transitions.
* It updates its local `PairLedger` on fills.
* It emits intents, not exchange calls.

### Async runtime assumptions

* `tokio` multi-threaded runtime.
* One actor task per active pair.
* One adapter task per websocket connection plus one REST sync task.
* One order manager task.
* One reconciler task.
* One metrics/admin task.

Why this shape:

* Pairs are independent.
* The critical race is between two sides of the same pair.
* Actor isolation makes it easier to enforce “no accidental single-side scale-up”.

### Persistence layer

Production recommendation: **Postgres + `sqlx`**.

Why:

* Strong transactional semantics for event write + snapshot update.
* Easy operational tooling.
* Works well with append-only event tables and periodic snapshots.

Persistence rules:

* Append every normalized market event, decision, order event, fill event, and settlement event.
* Update pair snapshots transactionally after fills and after major decisions.
* Persist config snapshots and config versions.
* On startup, rebuild active pair state from event journal + exchange reconciliation.

### Config layer

Use `serde` + `toml` + `figment` or `config`.
Config sections:

* `market_filter`
* `timing`
* `pricing`
* `inventory`
* `execution`
* `risk`
* `portfolio`
* `adapter`
* `paper`
* `features`
* `observability`

Secrets:

* Use `secrecy` and environment or secret manager injection.
* Never persist secrets in config snapshots.

### Logging and metrics

Use:

* `tracing`
* `tracing-subscriber`
* `metrics`
* `metrics-exporter-prometheus`

Every log line should include at least:

* `pair_id`
* `up_market_id`
* `down_market_id`
* `state`
* `config_version`
* `decision_id` where applicable

Core metrics:

* `pair_seed_submit_latency_ms`
* `pair_second_fill_latency_s`
* `pair_marginal_sum`
* `pair_combined_avg_paid`
* `pair_unmatched_fraction`
* `pair_match_ratio`
* `pair_taker_share`
* `pair_price_zone`
* `pair_residual_side_is_underdog`
* `pair_state_transitions_total`
* `risk_blocks_total{reason=...}`
* `reconciliation_mismatches_total`
* `settlement_delay_seconds`

### Admin and control interfaces

Expose:

* `GET /health`
* `GET /ready`
* `GET /metrics`
* `GET /pairs/:id`
* `GET /pairs/:id/orders`
* `GET /pairs/:id/ledger`
* `POST /pairs/:id/disable`
* `POST /kill-switch`
* `POST /config/reload`
* `POST /reconcile`

Also provide a CLI:

* `pairbotctl kill`
* `pairbotctl disable-pair <id>`
* `pairbotctl replay <capture>`
* `pairbotctl dry-run <date>`
* `pairbotctl reconcile-now`

### Fault tolerance and restart behavior

On restart:

1. Load latest valid config snapshot.
2. Load active pairs from DB.
3. Fetch open orders and current positions from exchange.
4. Reconcile local vs exchange state.
5. Rebuild in-memory actor state from persisted events.
6. Resume only when pair state is clean.
7. Otherwise remain in `RecoveryPaused`.

Hard rule:

* If persistence is unavailable, **do not send new orders**.
* If exchange reconciliation is not clean, **do not resume active trading**.

### Suggested Rust crates and why

* `tokio` — async runtime
* `rust_decimal` — deterministic accounting
* `rust_decimal_macros` — safe decimal literals
* `serde`, `serde_json`, `toml` — config and event serialization
* `sqlx` — async DB layer with compile-time checked queries
* `time` — robust timestamp handling
* `thiserror`, `anyhow` — error typing and propagation
* `tracing`, `tracing-subscriber` — structured logs
* `metrics`, `metrics-exporter-prometheus` — metrics
* `axum` — admin API
* `reqwest` — REST adapter calls
* `tokio-tungstenite` — websocket adapter calls
* `uuid` or `ulid` — deterministic local IDs
* `secrecy` — secret handling
* `clap` — CLI tools
* `proptest` — invariants and property tests
* `insta` — decision-log snapshot tests

### Key domain types

* `PairId`
* `MarketId`
* `OutcomeSide { Up, Down }`
* `MarketPair`
* `MarketPhase { PreOpen, Seeding, AwaitSecondFill, Accumulating, BalanceOnly, Paused, AwaitSettlement, SettledPendingRecon, Reconciled, Disabled, RecoveryPaused }`
* `Price`
* `Qty`
* `Cost`
* `PriceZone { Preferred, Acceptable, Caution, StopAdd, Danger }`
* `PairMetrics`
* `SideInventory`
* `PairInventory`
* `ResidualLot`
* `PairedLot`
* `OrderIntent`
* `RiskDecision`
* `OrderCommand`
* `ExecutionReport`
* `SettlementOutcome`
* `DecisionReasonCode`

### Trait interfaces that should exist

`trait MarketDataSource`

* `subscribe_pairs(...)`
* `snapshot_book(pair_id, side)`
* `heartbeat()`

`trait ExecutionVenue`

* `place_limit(order_cmd)`
* `cancel(order_id)`
* `replace(order_id, new_price, new_qty)`
* `fetch_open_orders()`
* `fetch_fills(since)`
* `fetch_positions()`

`trait SettlementSource`

* `subscribe_settlements(...)`
* `fetch_settlement(pair_id)`

`trait StrategyPolicy`

* `on_event(&mut self, event, snapshot, cfg) -> Vec<OrderIntent>`

`trait RiskPolicy`

* `evaluate(intent, snapshot, cfg) -> RiskDecision`

`trait PersistenceStore`

* `append_market_event(...)`
* `append_decision(...)`
* `append_order_event(...)`
* `append_fill(...)`
* `append_settlement(...)`
* `load_active_pairs()`
* `load_pair_state(pair_id)`

`trait ReplaySource`

* `next_event()`
* `reset()`

### Scope split

**MVP scope**

* Single venue
* BTC 5-minute Up/Down only
* Paired buy-only strategy
* Maker-first execution
* Hold-to-settlement
* FIFO lot pairing
* Postgres persistence
* Replay from captured raw events
* Paper mode and guarded live mode
* Admin kill switch and reconciliation

**Phase 2 improvements**

* Optional model-edge overlay
* Adaptive clip escalation
* Mild clip/refresh randomization
* Multi-level passive quoting
* Multi-account scaling
* Better queue-position fill model
* Cross-day analytics and optimizer

**Out of scope**

* BTC forecasting engine
* External candle or BTC price alpha as core logic
* Intrawindow flipping
* Cross-venue smart routing
* Leveraged hedging
* ML or RL as primary strategy logic
* Fully distributed microservices

---

## 4. Trading logic and state machine

### Per-market lifecycle

#### `PreOpen`

* Pair discovered, both sides mapped, schedule known.
* Subscribe to book/trade/settlement feeds.
* No orders live.

Transition to `Seeding` when:

* market open confirmed and both sides healthy.

#### `Seeding`

* Submit small passive seed orders on both sides.
* Seed size default: `12` each.
* No scale-up allowed.

Transition to `AwaitSecondFill` when:

* first fill arrives on exactly one side.

Transition to `Accumulating` when:

* both sides have at least one fill and price zone is acceptable.

Transition to `Paused` when:

* data stale, pair invalid, or marginal pair sum already blocked.

#### `AwaitSecondFill`

* One side filled, the other side missing.
* Freeze any additional adds on the filled side.
* Focus only on completing the missing side.
* Taker completion may be allowed under strict conditions.

Transition to `Accumulating` when:

* second side fills and price/inventory guards are healthy.

Transition to `BalanceOnly` when:

* 15 seconds pass without second-side fill.

Transition to `Paused` when:

* 30 seconds pass without second-side fill and no safe rescue exists.

#### `Accumulating`

* Both sides filled.
* Maker-first passive paired adds continue.
* Clip ladder and schedule depend on price zone, imbalance, time, and budget.
* This is the main state for copying the profitable engine.

Transition to `BalanceOnly` when:

* price enters caution zone,
* imbalance exceeds target,
* taker usage climbs,
* time into market >= 225 seconds.

Transition to `Paused` when:

* price enters stop-add or danger zone,
* data is stale,
* reconciliation mismatch appears.

Transition to `AwaitSettlement` when:

* time into market >= 240 seconds or manual stop.

#### `BalanceOnly`

* No new symmetric size increases.
* Only catch-up orders that reduce imbalance are allowed.
* Never add to underdog residual.
* Large clips are disabled.

Transition back to `Accumulating` when:

* imbalance recovers below target,
* price returns to acceptable zone,
* time is still before balance-only cutoff.

Transition to `AwaitSettlement` when:

* hard cutoff reached.

#### `Paused`

* Cancel working orders.
* No new orders.
* Wait for recovery, manual action, or settlement.

Transition to `BalanceOnly` or `Accumulating` only after:

* data healthy,
* reconciliation clean,
* risk re-approval,
* no hard-block reason active.

#### `AwaitSettlement`

* No working orders.
* Hold inventory.
* Wait for official resolution.
* Reconciler keeps checking state.

Transition to `SettledPendingRecon` when:

* official settlement event received.

#### `SettledPendingRecon`

* Apply settlement outcome.
* Compute paired PnL and residual PnL.
* Reconcile with exchange settlement status.

Transition to `Reconciled` when:

* local and exchange records match.

#### `Reconciled`

* Terminal state for the pair.

#### `Disabled`

* Hard risk breach or manual disable.
* No further orders for the pair.
* Still reconcile and settle.

#### `RecoveryPaused`

* Startup or reconnect safety state.
* No orders until local and venue state match.

---

### Core strategy rules

1. **Open quickly**

   * Start immediately after market open.
   * Both seed orders issued almost together.

2. **Fill both sides fast**

   * No accumulation until both sides have a fill.
   * Missing side completion is priority one.

3. **Price discipline first**

   * Preferred adds happen only when effective marginal pair cost is `< 0.94`.
   * Normal adds permitted `< 0.97`.
   * Caution zone `0.97-<1.00`: no new size increases, only safe balancing.
   * `>= 1.00`: stop-add.
   * `>= 1.03`: danger; disable pair for new risk.

4. **Inventory discipline second**

   * Target unmatched fraction `< 7%`.
   * Balance-only above `7%`.
   * Warning above `12%`.
   * Hard disable/reduce-only at `20%+`.

5. **Residual exposure is risk**

   * Residual on favorite side is tolerated more than underdog side.
   * Residual on underdog side must never be intentionally increased.

6. **Late-window discipline**

   * 0-15s: seed and complete pair.
   * 15-180s: normal paired accumulation.
   * 180-225s: smaller clips; no aggressive scaling.
   * 225-240s: balance-only.
   * 240s+: no new orders.

---

### Exceptional states

* Missing market metadata
* One side suspended or halted
* Stale book/trade feed
* Repeated order rejects
* DB unavailable
* Exchange reconnect in progress
* Settlement delayed
* Pair mapping mismatch
* Open order mismatch on restart

All exceptional states force `Paused`, `Disabled`, or `RecoveryPaused`.

---

### Kill-switch conditions

Global kill switch if any of the following occurs:

* DB unavailable for writes
* exchange auth failure or repeated order rejects
* startup reconciliation not clean
* stale market data persists beyond hard threshold
* duplicate or missing fill state cannot be resolved
* any pair breaches hard imbalance and cannot be contained
* any live order is found created in stop-add or danger price zone
* operator manual kill

Per-pair kill switch if:

* effective marginal pair sum enters danger zone `>= 1.03`
* unmatched fraction projected `>= 20%`
* underdog residual increase is about to occur
* second side remains incomplete after hard deadline and no safe rescue exists

---

### Pseudocode

#### Market open handling

```text
on_market_open(pair):
    if !pair.is_valid_btc_5m_pair:
        disable(pair, reason="invalid_pair")
        return

    if !books_fresh(pair.up) or !books_fresh(pair.down):
        pause(pair, reason="stale_open_books")
        return

    init_pair_timers(pair)
    state = Seeding
    emit_seed_orders(pair)
```

#### First pair seeding

```text
emit_seed_orders(pair):
    zone = classify_pair_zone(passive_price(up) + passive_price(down))

    if zone in {StopAdd, Danger}:
        pause(pair, reason="open_pair_too_expensive")
        return

    seed_clip = cfg.execution.seed_clip   // default 12

    submit_passive_buy(pair.up, seed_clip)
    submit_passive_buy(pair.down, seed_clip)

    record_decision("seed_both", zone, seed_clip)
```

#### Incremental fill scheduling

```text
on_fill(pair, fill):
    ledger.apply_fill(fill)
    snapshot = ledger.snapshot(pair)

    if state == Seeding or state == AwaitSecondFill:
        if snapshot.has_both_sides_filled():
            if snapshot.price_zone() <= Acceptable:
                state = Accumulating
            else:
                state = BalanceOnly
        else:
            state = AwaitSecondFill
            schedule_second_side_completion(pair)
        return

    if state == Accumulating:
        schedule_next_add(pair, snapshot)
        return

    if state == BalanceOnly:
        schedule_rebalance_only(pair, snapshot)
        return
```

#### Pair-price guardrails

```text
schedule_next_add(pair, snapshot):
    marginal_pair_sum = passive_price(up) + passive_price(down)
    projected_avg_paid = snapshot.project_combined_avg_paid_for_balanced_add(next_clip)

    if marginal_pair_sum >= 1.03:
        cancel_all(pair)
        disable(pair, reason="danger_zone")
        return

    if marginal_pair_sum >= 1.00:
        cancel_pair_adds(pair)
        state = BalanceOnly
        return

    if snapshot.unmatched_fraction >= 0.07:
        state = BalanceOnly
        schedule_rebalance_only(pair, snapshot)
        return

    clip = choose_clip(snapshot, marginal_pair_sum, time_into_market)
    submit_passive_buy(pair.up, clip)
    submit_passive_buy(pair.down, clip)
```

#### Unmatched-inventory guardrails

```text
schedule_rebalance_only(pair, snapshot):
    if snapshot.unmatched_fraction >= 0.20:
        cancel_all(pair)
        disable(pair, reason="hard_imbalance")
        return

    lagging_side = snapshot.lagging_side()
    residual_side = snapshot.residual_side()
    residual_kind = snapshot.residual_kind()   // favorite, underdog, none

    if residual_kind == "underdog" and add_would_increase_underdog_residual():
        block("cheap_side_overweight")
        cancel_orders_on(residual_side)
        return

    rebalance_qty = min(snapshot.unmatched_qty(), cfg.execution.max_rebalance_clip)
    marginal_rebalance_sum = ledger.next_residual_lot_cost(residual_side) + price(lagging_side)

    if marginal_rebalance_sum >= 1.00:
        cancel_all(pair)
        state = AwaitSettlement
        return

    submit_rebalance_order(lagging_side, rebalance_qty, passive_or_allowed_taker())
```

#### Optional model-edge overlay

```text
model_edge_allows(intent, side):
    if !cfg.features.model_edge_overlay:
        return true

    edge = latest_edge_model_minus_price(side)
    if intent.kind in {DirectionalException, OneSideCatchup}:
        return edge > 0.02

    if intent.kind == AcceleratedBalancedAdd:
        return edge_up >= 0 and edge_down >= 0

    return true
```

#### Settlement bookkeeping

```text
on_settlement(pair, final_outcome):
    cancel_all(pair)
    ledger.apply_settlement(final_outcome)

    paired_pnl = ledger.locked_paired_pnl()
    residual_pnl = ledger.residual_directional_pnl()
    total_pnl = paired_pnl + residual_pnl - fees

    persist_pnl(pair, paired_pnl, residual_pnl, total_pnl, final_outcome)
    reconcile_with_exchange_settlement(pair)

    if reconciliation_clean:
        state = Reconciled
    else:
        state = SettledPendingRecon
```

---

## 5. Risk engine requirements

### Hard blocks vs soft throttles

**Hard blocks**

* New exposure in stop-add or danger price zones
* Projected unmatched fraction `>= 20%`
* Any order that increases underdog residual
* Missing pair mapping
* Stale data beyond hard threshold
* Unresolved reconciliation mismatch
* DB or adapter health failure
* Market past hard add cutoff
* Taker share at or above hard cap
* Operator/manual disable

**Soft throttles**

* Marginal pair cost in `0.94-<0.97`
* Caution zone `0.97-<1.00`
* Unmatched fraction `> 7%`
* Taker share `> 5%`
* Time into market `>= 180 seconds`
* Stale data in warning range
* Partial completion pressure before 30 seconds

### Market-level limits

* **Price zone**

  * Preferred `< 0.94`: full normal logic
  * Acceptable `0.94-<0.97`: balanced adds allowed
  * Caution `0.97-<1.00`: no new symmetric size increase
  * Stop-add `>= 1.00`: block new size
  * Danger `>= 1.03`: cancel and disable pair

* **Imbalance**

  * Target `< 7%`
  * Warning `> 12%`
  * Hard block `>= 20%`

* **Time**

  * Seed orders by open + 5s
  * Second-side target within 15s of first fill
  * Second-side hard deadline 30s
  * Reduce clip after 180s
  * Balance-only after 225s
  * No new orders after 240s

* **Order shape**

  * Max working orders per side in MVP: `1`
  * Max single order size: `80`
  * Max rebalance order size: config default `20`, can escalate to `40` only if it reduces imbalance and stays in acceptable pricing

### Side-level limits

* No scale on a side until opposite side has filled at least once.
* No approval for an order that would make the residual side the underdog side.
* No one-sided speculative add, ever, in MVP.
* If one side fills faster:

  * cancel or reduce that side’s outstanding orders,
  * allow only lagging-side orders,
  * if lagging-side price is blocked, stop and hold.

### Portfolio-level limits

Config-driven because the analysis does not provide account size.

Required limits:

* max concurrent active pairs
* max gross deployed cost
* max gross unmatched cost
* max number of pairs in `BalanceOnly`
* max number of pairs with unresolved settlement delay

Recommended first-live defaults:

* `max_concurrent_pairs = 4`
* `max_pairs_in_balance_only = 2`
* `max_pairs_unreconciled = 0`

### Time-based limits

Hard:

* no new pair after hard cutoff
* no recovery from `AwaitSecondFill` into `Accumulating` after 30 seconds without explicit safe completion

Soft:

* downshift clip ladder after 180 seconds
* stop 80-share clips after 180 seconds

### Order-rate limits

Until exchange limits are confirmed:

* max one replace cycle per side per second
* max one new order per side at a time
* max one taker exception per side per 10 seconds

If venue rate limits are lower, adapter config must override.

### Taker-usage limits

* Target `< 5%`
* Warning `5%-<10%`
* Hard stop `>= 10%`

Aggressive orders allowed only when all are true:

* second side missing or lagging,
* order reduces imbalance,
* marginal cost remains `< 1.00`,
* taker cap not breached,
* market not past hard cutoff.

### Bad-price limits

Use **effective** price, not raw quoted price.

Effective price must include:

* known maker/taker fees if available
* conservative fee buffer if fees not known yet

Rules:

* balanced adds require effective marginal pair sum `< 0.97` for normal adds
* accelerated clip escalation requires effective marginal pair sum `< 0.94`
* rebalancing can be allowed up to but not including `1.00`
* nothing new at `>= 1.00`
* force disable at `>= 1.03`

### Stale-data limits

* Warning stale threshold: `2 seconds`

  * pause new adds
* Hard stale threshold: `5 seconds`

  * cancel working orders
  * transition pair to `Paused`
* Persisting stale condition beyond configurable grace:

  * global kill if systemic

### API/connectivity degradation rules

* Websocket disconnect:

  * pause new orders immediately
  * attempt reconnect
  * reconcile open orders once back
* REST failure burst:

  * stop replace logic
  * maintain safe mode
* Order reject burst:

  * after 3 consecutive rejects on a pair, disable the pair
  * after 5 across the service in a short interval, global pause
* DB write failure:

  * global kill for new orders

### Daily stop conditions

Because account capital is unknown, use config-driven quote-currency limits plus behavior breach limits.

Required daily stops:

* configurable `daily_loss_limit_quote_ccy`
* configurable `daily_residual_loss_limit_quote_ccy`
* configurable `daily_hard_breach_limit`

Recommended behavioral stop:

* stop new live entries if 3 hard-breach events occur in one day, even if PnL is flat

### Emergency flat / disable logic

In MVP, “emergency flat” means:

* cancel all working orders
* disable all new entries
* hold residual positions to settlement
* reconcile aggressively

Actual active position liquidation before settlement is out of scope for MVP unless the exchange requires it and semantics are confirmed.

---

## 6. Execution engine requirements

### Maker-first order placement

* Use post-only limit orders if supported.
* If post-only is not supported, place non-crossing passive limits only.
* MVP should maintain **at most one working order per side**.
* Priority is queue-safe passive price, not best theoretical price.

### Quote refresh / amend / cancel policy

Refresh when any of the following is true:

* top-of-book changed by at least one tick
* working order is no longer best passive candidate
* order age exceeds refresh threshold
* price zone changed
* imbalance state changed
* pair moved into late-window state

Default thresholds:

* normal refresh age: `5 seconds`
* near cutoff refresh age: `2 seconds`
* max refresh cadence: `1/s/side`

If replace is unsupported:

* cancel then create
* never leave stale working orders live across a state transition

### Passive order aging

* Seed orders should not sit unchanged too long; the goal is early presence.
* If a seed order is untouched for 10 seconds and second side is still missing, cancel/re-price or move to safe completion logic.
* In `BalanceOnly`, working rebalance orders should be short-lived and aggressively re-evaluated.

### Clip sizing logic

Recommended ladder:

* `seed_clip = 12`
* `base_clip = 20`
* `accelerated_clip = 40`
* `max_clip = 80`

Escalation conditions:

* both sides already filled
* marginal pair sum `< 0.94`
* projected unmatched fraction `< 7%`
* time into market `< 180s`
* no stale data
* taker share `< 5%`
* market-level budget available

De-escalation conditions:

* marginal pair sum `>= 0.94`
* imbalance `>= 7%`
* time `>= 180s`
* no fill progress
* repeated partial fills on one side only

### Repeated 80-share clip behavior

Do not make 80 the default.
Make 80 a **capped acceleration mode**.

Use 80 only when:

* pair already established,
* price zone is preferred,
* inventory is balanced,
* early/mid-window,
* no risk warning active.

This copies the profitable high-volume behavior without copying loose risk.

### Randomization / anti-pattern controls

MVP:

* disabled

Phase 2:

* optional small clip jitter and refresh jitter to reduce detectability
* must not alter accounting, guardrails, or determinism in replay mode

### Fill handling

On every fill:

* persist raw fill event
* dedupe by `fill_identity_key`
* update side inventory
* pair residual lots using FIFO
* recompute pair metrics
* cancel or amend opposite-side orders if needed
* re-run risk and strategy state

### Partial fill response

If both sides partially fill in roughly matched amounts:

* continue normal accumulation

If one side partially fills materially faster:

* freeze additional growth on the faster side
* reduce/cancel remaining faster-side order
* route fill opportunity to lagging side only

### One-side fill drift management

If only one side gets filled:

* do not add more on the filled side
* keep only completion logic alive on the missing side
* if missing side still not filled by 15 seconds, switch to `BalanceOnly`
* if still missing by 30 seconds:

  * allow one small taker rescue only if it improves balance and keeps marginal pair sum `< 1.00`
  * otherwise stop and hold the small residual

### Rebalancing when one side fills faster

When one side is ahead:

* identify `residual_side`
* identify `lagging_side`
* identify `favorite_side` and `underdog_side`

Rules:

* never add to the residual side if it is already the underdog
* only add to the lagging side
* use passive first
* taker allowed only if it reduces imbalance and is still below the stop-add threshold

### Preventing cheap-side over-accumulation

The execution engine must explicitly compute:

* current residual side
* current underdog side
* whether the next order increases residual magnitude on the underdog side

If true:

* hard block
* cancel any outstanding orders on that side
* leave pair in `BalanceOnly`, `Paused`, or `AwaitSettlement`

### When taker orders are allowed

Taker orders are allowed only for:

* second-side completion within the first 30 seconds
* lagging-side rebalance that reduces an existing residual
* only when the effective marginal pair sum remains `< 1.00`
* only while taker share remains below cap
* never for pure directional adds
* never after hard cutoff

### Explicit behavior in edge cases

**Only one side gets filled**

* Freeze filled side
* Quote missing side
* No scale-up
* One possible small taker rescue
* Else pause and hold the small seed residual

**Combined paid price rises too far**

* Cancel all pair-add orders
* Enter `BalanceOnly`
* If danger zone, disable pair

**Market time is running out**

* After 180s, shrink clips
* After 225s, rebalance-only
* After 240s, cancel all and await settlement

**Fills arrive too quickly on one side**

* Cancel or reduce faster-side working order
* Recompute imbalance
* Route only lagging-side orders

**Model edge is negative**

* In MVP: ignored because overlay is disabled
* If overlay enabled: no one-sided exception orders on a negative-edge side

**Settlement data is delayed**

* No new orders
* Keep pair in `AwaitSettlement`
* Poll settlement source
* Alert after configured delay threshold
* Do not “guess” outcome

---

## 7. Data model and persistence plan

### Persistence model

Use an append-only event journal plus snapshot tables.

### Core tables / streams

**`market_pairs`**

* `pair_id`
* `up_market_id`
* `down_market_id`
* `market_family`
* `opens_at`
* `closes_at`
* `status`
* `created_at`

**`market_events`**

* `event_id`
* `pair_id`
* `market_id`
* `event_type`
* `exchange_ts`
* `ingest_ts`
* `payload_json`
* `source_seq`

**`strategy_decisions`**

* `decision_id`
* `pair_id`
* `phase_before`
* `phase_after`
* `decision_type`
* `reason_code`
* `time_into_market_s`
* `time_remaining_s`
* `marginal_pair_sum`
* `projected_combined_avg_paid`
* `projected_unmatched_fraction`
* `projected_match_ratio`
* `favorite_side`
* `underdog_side`
* `taker_share`
* `config_version`
* `created_at`

**`orders`**

* `client_order_id`
* `exchange_order_id`
* `pair_id`
* `market_id`
* `outcome_side`
* `intent_kind`
* `limit_price`
* `qty`
* `remaining_qty`
* `liquidity_intent` (`maker`, `taker_exception`)
* `status`
* `submission_state`
* `created_at`
* `updated_at`

**`order_events`**

* `order_event_id`
* `client_order_id`
* `exchange_order_id`
* `event_type`
* `payload_json`
* `exchange_ts`
* `ingest_ts`

**`fills`**

* `fill_identity_key`
* `exchange_fill_id`
* `client_order_id`
* `exchange_order_id`
* `pair_id`
* `market_id`
* `outcome_side`
* `price`
* `qty`
* `gross_cost`
* `fee`
* `maker_taker`
* `exchange_ts`
* `ingest_ts`

**`side_inventory_snapshots`**

* `pair_id`
* `outcome_side`
* `filled_qty`
* `open_order_qty`
* `cum_cost`
* `avg_paid`
* `maker_qty`
* `taker_qty`
* `last_fill_ts`
* `snapshot_ts`

**`residual_lots`**

* `lot_id`
* `pair_id`
* `outcome_side`
* `remaining_qty`
* `entry_price`
* `entry_cost`
* `created_fill_identity_key`
* `created_ts`

**`paired_lots`**

* `paired_lot_id`
* `pair_id`
* `qty`
* `up_entry_price`
* `down_entry_price`
* `up_cost`
* `down_cost`
* `locked_value` (`qty * 1`)
* `locked_paired_pnl`
* `paired_at`

**`pair_snapshots`**

* `pair_id`
* `phase`
* `qty_up`
* `qty_down`
* `avg_paid_up`
* `avg_paid_down`
* `combined_avg_paid`
* `unmatched_qty`
* `unmatched_fraction`
* `match_ratio`
* `favorite_side`
* `underdog_side`
* `residual_side`
* `residual_kind`
* `taker_share`
* `time_into_market_s`
* `time_remaining_s`
* `last_action`
* `snapshot_ts`

**`pnl_ledger`**

* `pair_id`
* `ts`
* `locked_paired_pnl`
* `residual_directional_pnl`
* `realized_pnl`
* `fees`
* `final_outcome`
* `formula_version`

**`settlement_events`**

* `pair_id`
* `market_id`
* `final_outcome`
* `exchange_ts`
* `ingest_ts`
* `payload_json`

**`reconciliation_runs`**

* `recon_id`
* `scope` (`startup`, `periodic`, `post_reconnect`, `post_settlement`)
* `pair_id`
* `local_open_orders`
* `exchange_open_orders`
* `local_positions`
* `exchange_positions`
* `result`
* `diff_json`
* `created_at`

**`risk_events`**

* `risk_event_id`
* `pair_id`
* `severity`
* `code`
* `message`
* `metrics_json`
* `action_taken`
* `created_at`

**`config_snapshots`**

* `config_version`
* `config_hash`
* `loaded_at`
* `config_text`

**`raw_capture_events`**

* append-only raw event log for replay

### Continuous derived metrics

#### Time metrics

* `time_into_market_s = max(0, now - opens_at)`
  Runtime equivalent of `t_into_s`.

* `time_remaining_s = max(0, closes_at - now)`
  Runtime equivalent of `t_remain_s`.

#### Cost basis metrics

* `avg_paid_up = cum_cost_up / filled_qty_up` if `filled_qty_up > 0`
* `avg_paid_down = cum_cost_down / filled_qty_down` if `filled_qty_down > 0`
* `combined_avg_paid = avg_paid_up + avg_paid_down` when both sides exist

#### Pair price metrics

* `marginal_pair_sum_balanced = quote_up + quote_down`
* `marginal_pair_sum_rebalance = next_residual_lot_entry_price + quote_lagging_side`
* `effective_marginal_pair_sum = raw_marginal_pair_sum + fee_buffer`

#### Inventory metrics

* `unmatched_qty = abs(filled_qty_up - filled_qty_down)`
* `unmatched_inventory_fraction = unmatched_qty / (filled_qty_up + filled_qty_down)` if total > 0 else 0
* `match_ratio = min(filled_qty_up, filled_qty_down) / max(filled_qty_up, filled_qty_down)` if max > 0 else 1

#### Execution metrics

* `taker_share = taker_qty / (maker_qty + taker_qty)` if total > 0 else 0
* `fill_cadence = fills_in_last_10s / 10`
* `time_to_second_side = abs(first_fill_ts_up - first_fill_ts_down)` once both exist

#### Favorite / underdog metrics

Assumption:

* `favorite_side` = side with higher current reference price
* `underdog_side` = side with lower current reference price
* if equal within one tick, set both to `None`

Then:

* `residual_side` = side with larger filled qty
* `favorite_residual_flag = residual_side == favorite_side`
* `underdog_residual_flag = residual_side == underdog_side`

#### PnL metrics

Use FIFO pairing for exact matched-vs-residual decomposition.

* `locked_paired_pnl = Σ (paired_lot.qty * 1 - paired_lot.up_cost - paired_lot.down_cost)`
* `residual_directional_pnl_at_settlement = Σ residual_lot.qty * (settlement_value(side) - entry_price)`
* `realized_pnl = locked_paired_pnl + residual_directional_pnl_at_settlement - total_fees`

`settlement_value(side)`:

* `1` if side matches `final_outcome`
* `0` otherwise

If pre-settlement mark is needed:

* keep it separate from realized accounting
* use a configurable mark source
* never mix marked PnL into realized PnL

### Audit trail requirements

For every live order, the system must answer:

* which pair state emitted it
* which risk decision approved it
* which config version was active
* which marginal pair sum was used
* whether it was intended to seed, add, or rebalance
* whether it increased or reduced residual risk

---

## 8. Implementation checklist

TASK-001 | M0 | Repo setup | Create Rust workspace, toolchain pin, workspace lints, formatting, clippy, and deny accidental float arithmetic in strategy/ledger crates. | None | `cargo test`, `cargo clippy`, and float-arithmetic lint policy pass in CI.

TASK-002 | M0 | CI/CD | Add GitHub Actions or equivalent for fmt, clippy, tests, sqlx checks, and container build. | TASK-001 | CI passes on every PR.

TASK-003 | M0 | Crate boundaries | Scaffold crates: domain, config, registry, ledger, risk, strategy, execution, exchange_core, adapter_paper, adapter_polymarket, persistence, replay, service, admin_api. | TASK-001 | Workspace compiles with empty crate skeletons.

TASK-004 | M0 | Dev environment | Add docker-compose for Postgres, Prometheus, Grafana; add local `.env.example`. | TASK-001 | `docker compose up` produces a working local stack.

TASK-005 | M0 | Domain types | Implement decimal-backed newtypes for price, qty, cost, fees, pair IDs, market IDs, fill identity key, reason codes. | TASK-003 | Unit tests prove arithmetic is deterministic and no floats leak into domain math.

TASK-006 | M0 | Config loading | Implement typed TOML config, validation, feature flags, config version hashing, and hot-reload watch channel. | TASK-003 | Config loads, validates, reloads, and persists a version hash.

TASK-007 | M0 | Secrets handling | Wire `secrecy` and environment-based secret loading for adapter credentials. | TASK-006 | Secrets are available to adapter code and never logged.

TASK-008 | M0 | DB migrations | Create migration framework and baseline schema for config snapshots, market pairs, events, orders, fills, decisions, settlements, reconciliation. | TASK-004 | Local migration applies cleanly from zero.

TASK-009 | M0 | Analytics importer | Build `analysis_importer` tool that reads the provided parquet/csv and maps schema fields exactly, including `trade_identity_key`, `t_into_s`, `t_remain_s`, `final_outcome`, and `edge_model_minus_price`. | TASK-005, TASK-008 | Import tool loads the files and persists normalized rows.

TASK-010 | M0 | Analytics validation | Reproduce the reference analysis metrics from the provided files and store them as fixture expectations. | TASK-009 | Counts and major ratios match expected values within tolerance.

TASK-011 | M1 | Market registry | Implement pair discovery and strict BTC 5-minute whitelist matching with Up/Down mapping and schedule fields. | TASK-005, TASK-006 | Registry emits valid `MarketPair` objects and rejects ambiguous markets.

TASK-012 | M1 | Exchange core traits | Define normalized market event, execution report, order command, settlement event, and adapter traits. | TASK-005 | Strategy and adapters compile against shared traits.

TASK-013 | M1 | Raw capture model | Create raw event capture format and writer for future replay inputs. | TASK-008, TASK-012 | Live or simulated adapter events can be appended as raw capture files.

TASK-014 | M1 | Ledger lot engine | Implement FIFO residual lots, paired lot creation, fill dedupe, average cost, unmatched qty, and match ratio logic. | TASK-005 | Property tests prove quantity conservation and exact pairing invariants.

TASK-015 | M1 | PnL decomposition | Implement locked paired PnL and residual directional PnL formulas using lot data. | TASK-014 | Unit tests prove paired + residual + fees = total realized PnL at settlement.

TASK-016 | M1 | Pair metrics | Implement combined average paid, marginal pair sum helpers, favorite/underdog flags, taker share, and time metrics. | TASK-014 | Snapshot tests verify metric values for fixture inputs.

TASK-017 | M1 | Persistence repository | Implement `PersistenceStore` methods for append-only events and snapshot writes. | TASK-008, TASK-012 | Integration tests prove writes and reads are transactional.

TASK-018 | M1 | Replay clock | Implement deterministic clock abstraction for replay and simulation. | TASK-005 | Replay uses simulated time without wall-clock dependencies.

TASK-019 | M1 | Replay runner | Build replay engine over captured raw events and the deterministic clock. | TASK-013, TASK-018 | Replay can stream events into the service in deterministic order.

TASK-020 | M1 | Strategy states | Implement market phase enum, transition rules, and state serialization. | TASK-005 | Unit tests cover all valid and invalid transitions.

TASK-021 | M2 | Price zone evaluator | Implement effective marginal pair cost classification and stop/danger logic. | TASK-016 | Tests cover all five price zones and fee-buffer behavior.

TASK-022 | M2 | Inventory risk rules | Implement unmatched fraction target/warning/hard-block rules and underdog residual block logic. | TASK-016, TASK-020 | Risk engine blocks all projected underdog-residual increases and hard imbalance breaches.

TASK-023 | M2 | Timing rules | Implement seed deadlines, second-side deadlines, clip downshift timing, balance-only timing, and hard cutoff timing. | TASK-020 | Replay tests show correct phase transitions by time.

TASK-024 | M2 | Seeding logic | Implement simultaneous seed intent generation, default seed clip, and no-scale-before-both-filled logic. | TASK-020, TASK-021, TASK-023 | Replay tests show zero scale-up before both sides are filled.

TASK-025 | M2 | Accumulation logic | Implement paired add generation for preferred and acceptable price zones. | TASK-024, TASK-021, TASK-022 | Strategy emits balanced add intents only under allowed conditions.

TASK-026 | M2 | Balance-only logic | Implement lagging-side-only rebalancing and underdog-side block logic. | TASK-025 | Replay tests prove no orders increase underdog residual.

TASK-027 | M2 | Clip ladder policy | Implement clip ladder, escalation, de-escalation, and hard 80-share cap. | TASK-025 | Tests prove 40/80 clips only happen under green conditions.

TASK-028 | M2 | Taker exception logic | Implement second-side rescue and rebalance taker exceptions under capped conditions. | TASK-026 | Tests show taker intents are impossible outside approved exception paths.

TASK-029 | M2 | Order intent model | Define order intent types, reason codes, and projected metric payloads. | TASK-024 | Every strategy output is a typed intent with full audit context.

TASK-030 | M2 | Risk engine integration | Wire strategy intents through risk evaluation before execution. | TASK-021, TASK-022, TASK-029 | No order intent reaches execution without a persisted risk decision.

TASK-031 | M3 | Execution state | Implement order manager state for one working order per side, replace/cancel logic, and per-side refresh caps. | TASK-012, TASK-030 | Integration tests prove no duplicate live orders per side.

TASK-032 | M3 | Paper adapter | Implement conservative paper execution adapter against normalized market data. | TASK-012, TASK-031 | Paper mode accepts real intents and emits normalized fills and acks.

TASK-033 | M3 | Simulator fill rules | Add conservative passive-fill and taker-fill model for paper/replay. | TASK-032 | Fill simulation behaves deterministically under replay.

TASK-034 | M3 | Service wiring | Build the single-process supervisor, channels, market actors, and adapter wiring. | TASK-017, TASK-030, TASK-031, TASK-032 | Service starts, routes events, and shuts down cleanly.

TASK-035 | M3 | Metrics and logging | Emit structured logs and Prometheus metrics for decisions, prices, imbalance, taker share, and settlement. | TASK-034 | Dashboards can display core operational metrics.

TASK-036 | M3 | Admin API | Build health, readiness, metrics, pair inspection, disable-pair, kill-switch, and config-reload endpoints. | TASK-034 | Operators can inspect and control the service.

TASK-037 | M3 | Reconciliation service | Implement startup, periodic, reconnect, and post-settlement reconciliation flows. | TASK-017, TASK-034 | Simulated mismatches force pause and recovery behavior.

TASK-038 | M3 | Recovery safe mode | Implement `RecoveryPaused` and startup gating until reconciliation is clean. | TASK-037 | Live and paper service refuse trading until startup checks pass.

TASK-039 | M3 | Dry-run CLI | Build dry-run and replay CLI tools that output decisions and pair metrics without live orders. | TASK-019, TASK-034 | Engineers can replay a capture and inspect decisions from the command line.

TASK-040 | M3 | Integration tests | Add end-to-end tests from market open through settlement using the paper adapter. | TASK-034, TASK-037 | CI runs full lifecycle tests.

TASK-041 | M4 | Live adapter auth | Implement production adapter auth/signing and credential validation. | TASK-012, TASK-007 | Adapter can authenticate against the venue in a safe environment.

TASK-042 | M4 | Live market data | Implement venue market-data subscriptions and normalization for books, trades, and settlement events. | TASK-041 | Live feed produces normalized market events.

TASK-043 | M4 | Live order routing | Implement place/cancel/replace/fetch-open-orders/fetch-fills flows with idempotent client order IDs. | TASK-041, TASK-031 | Live adapter passes sandbox or staging integration tests.

TASK-044 | M4 | Live startup sync | Implement exchange snapshot sync for open orders and positions before resuming. | TASK-043, TASK-037 | Service can restart cleanly and rebuild active market state.

TASK-045 | M4 | Shadow mode | Add live shadow mode that runs full strategy/risk/ledger paths without sending live orders. | TASK-042, TASK-034 | Shadow decisions and hypothetical orders are logged and inspectable.

TASK-046 | M4 | Settlement resolver | Implement official settlement ingestion and PnL finalization path. | TASK-042, TASK-015 | Settled pairs move to `Reconciled` when venue confirms outcome.

TASK-047 | M4 | Alerts | Add alert routing for stale data, reconciliation failures, hard-risk blocks, and settlement delays. | TASK-035, TASK-036 | Operators receive alerts in staging.

TASK-048 | M4 | Deployment scripts | Add container image, env templates, deployment manifests, and staged rollout scripts. | TASK-041 to TASK-047 | Service can deploy to staging reproducibly.

TASK-049 | M5 | Property tests | Expand property-based tests for lot matching, PnL decomposition, and state invariants. | TASK-014, TASK-015, TASK-020 | Invariants hold across randomized event streams.

TASK-050 | M5 | Replay certification suite | Build replay scenarios that test good, bad, stale, delayed settlement, and imbalance edge cases. | TASK-019, TASK-040 | Replay suite becomes part of release gating.

TASK-051 | M5 | Paper KPI dashboard | Build dashboards and reports for seed timing, price zones, imbalance, taker share, and residual flags. | TASK-035 | Daily paper/shadow reports are generated automatically.

TASK-052 | M5 | Live canary controls | Add canary config profiles with low concurrency and low budget caps. | TASK-048 | First-live deployment can run with intentionally tiny exposure.

TASK-053 | M5 | Hot reload hardening | Make config reload atomic and rollback-safe. | TASK-006, TASK-036 | Bad config reload leaves old config active and raises an alert.

TASK-054 | M5 | Runbooks | Write runbooks for startup, restart, kill-switch, settlement delay, stale data, and reconciliation mismatch. | TASK-047, TASK-048 | Ops can run the service without engineer intervention.

TASK-055 | M5 | Backtester report tool | Build report generation over replay runs: zone mix, imbalance histogram, paired vs residual PnL, breach counts. | TASK-050 | Engineers can compare configuration variants.

TASK-056 | M5 | Model-edge overlay | Add optional overlay feature using `edge_model_minus_price > 0.02`, disabled by default. | TASK-021, TASK-029 | Feature flag exists and integration tests pass when enabled.

TASK-057 | M5 | Clip jitter feature | Add optional size and timing jitter under deterministic seeded replay. | TASK-027, TASK-019 | Feature remains deterministic when replay seed is fixed.

TASK-058 | M5 | Multi-level quoting | Add optional layered passive quotes as a Phase 2 feature. | TASK-031 | Feature flag exists and is disabled in MVP.

TASK-059 | M5 | Performance profiling | Profile hot paths under many active pairs and optimize DB writes and channel fanout. | TASK-034 | Service meets target CPU and latency envelope in load test.

TASK-060 | M5 | Production readiness review | Run formal checklist over validation, runbooks, replay, paper KPIs, and staging results. | TASK-049 to TASK-059 | Signed launch decision package exists.

---

## 9. Milestone roadmap

### M0 — Strategy freeze and analytics parity

**Goal**
Lock the rules that will actually be built and prove the analytics code matches the evidence.

**Scope**
Workspace, domain types, config, DB migrations, analysis importer, analytics parity checks.

**Deliverables**

* Workspace skeleton
* Deterministic decimal types
* Config schema
* DB schema
* Importer for provided parquet/csv
* Reference metric fixture suite

**Dependencies**

* Provided analysis files and schema
* Agreement on strict BTC 5-minute market filtering

**Exit criteria**

* Importer reproduces reference counts and headline metrics
* Core types compile
* CI green

**Demo artifact**

* CLI that loads the historical files and prints verified strategy metrics

**Risks**

* Schema mismatch
* Ambiguous pair mapping in historical files

---

### M1 — Deterministic ledger and replay foundation

**Goal**
Build the accounting spine before any live execution.

**Scope**
FIFO lot engine, paired/residual PnL, persistence repositories, raw capture format, replay clock and runner.

**Deliverables**

* Ledger crate
* PnL decomposition
* Replay engine
* Transactional persistence layer

**Dependencies**

* M0 complete

**Exit criteria**

* Property tests for quantity conservation pass
* PnL decomposition exactness tests pass
* Replay can deterministically drive the service

**Demo artifact**

* Replay run showing ledger and state changes from a captured event stream

**Risks**

* Lot matching bugs
* Too much logic leaking into adapter layer

---

### M2 — Strategy state machine and risk engine

**Goal**
Encode the paired engine and explicitly eliminate the losing behaviors.

**Scope**
State machine, timing rules, price zones, imbalance rules, clip ladder, taker exception logic.

**Deliverables**

* Strategy crate
* Risk engine crate
* Full decision logging
* Unit and replay tests for state/risk logic

**Dependencies**

* M1 complete

**Exit criteria**

* No prohibited behavior reachable in tests
* Replay produces valid seeding, accumulation, balance-only, and cutoff behavior

**Demo artifact**

* Decision trace for synthetic and replayed markets showing price and inventory discipline

**Risks**

* State machine complexity
* Hidden loopholes allowing underdog residual growth

---

### M3 — Paper trading MVP

**Goal**
Run the exact bot behavior on live data without live financial risk.

**Scope**
Paper adapter, supervisor wiring, admin API, metrics, dashboards, reconciliation, dry-run tooling.

**Deliverables**

* Single-process service
* Paper mode
* Dashboards
* Admin controls
* Periodic reconciliation

**Dependencies**

* M2 complete

**Exit criteria**

* End-to-end lifecycle from open to settlement works in paper mode
* Operational dashboards live
* Reconciliation and restart safety proven

**Demo artifact**

* Paper trading session with active pair inspection and final PnL decomposition

**Risks**

* Fill model optimism
* Operational noise from too many pair actors

---

### M4 — Guarded live MVP

**Goal**
Trade live with low exposure and heavy controls.

**Scope**
Production adapter, shadow mode, startup sync, live routing, settlement finalization, alerts, canary deployment.

**Deliverables**

* Live adapter
* Shadow mode
* Live startup recovery
* Canary deployment manifests
* Alerts

**Dependencies**

* M3 complete
* Venue API details confirmed

**Exit criteria**

* Shadow mode stable
* Startup reconciliation clean
* Canary live mode works with tiny exposure and no hard breaches

**Demo artifact**

* Shadow-to-live canary runbook execution and live market traces

**Risks**

* Exchange semantics differ from assumptions
* Unexpected rate limits or order lifecycle edge cases

---

### M5 — Production hardening and Phase 2 controls

**Goal**
Make the system operationally durable and prepare optional improvements.

**Scope**
Expanded replay suite, runbooks, performance tuning, canary controls, optional model-edge overlay, optional jitter.

**Deliverables**

* Release gate suite
* Runbooks
* Production readiness checklist
* Optional feature flags for Phase 2

**Dependencies**

* M4 complete

**Exit criteria**

* Validation gates met
* Release review signed
* Rollback procedures tested

**Demo artifact**

* Release candidate package with replay cert report and ops playbooks

**Risks**

* Optimization pressure before behavioral correctness
* Premature scope creep into forecasting or intrawindow trading

---

## 10. Validation and launch gates

### Unit tests

Required:

* decimal arithmetic and rounding
* price zone classification
* unmatched fraction calculation
* match ratio calculation
* favorite/underdog classification
* clip ladder escalation and de-escalation
* hard cutoff timing
* no-scale-before-both-filled
* underdog residual block logic
* settlement PnL decomposition

### Property tests

Required invariants:

* fill dedupe is idempotent
* total side qty equals paired qty plus residual qty
* locked paired PnL plus residual PnL plus fees equals realized PnL at settlement
* unmatched fraction stays in `[0, 1]`
* no state transition can emit orders in `Paused`, `AwaitSettlement`, `Disabled`, or `RecoveryPaused`
* no approved order may increase underdog residual
* no approved order may exceed 80 shares in MVP

### Replay tests

There are two separate replay classes.

**Analytics parity replay**

* Run importer over provided files
* Reproduce the calibration metrics used for bot thresholds

Minimum parity checks:

* detailed trade tape market count matches expected filtered count
* closed-position pair count matches expected filtered count
* two-sided participation rate matches expected rate
* weighted pair-sum median within tolerance
* taker share within tolerance
* price-zone outcome summaries within tolerance

**Execution replay**

* Use captured raw live exchange events
* Run the exact service stack
* Verify deterministic decisions and ledger outputs

### Simulation tests

Required scenarios:

* happy path with early pair completion
* one-side-only fill path
* caution zone transition
* stop-add and danger zone transitions
* imbalance breach and balance-only mode
* stale feed pause
* reconnect and startup recovery
* settlement delay
* duplicate fill event
* order reject burst

### Paper-trading KPIs

Run paper mode for at least **10 trading days or 500 settled pairs**, whichever is larger.

Must pass all:

* seed order submit delta <= 1s on >= 99% of entered pairs
* no scale-up before both sides filled
* median unmatched fraction < 7%
* 95th percentile unmatched fraction < 12%
* zero pairs with unmatched fraction >= 20%
* zero approved intents with effective marginal pair sum >= 1.00
* zero approved intents increasing underdog residual
* taker share < 10% hard, target < 5%
* zero intentional single-side speculative markets
* 100% settlement reconciliation
* paired PnL component positive over the sample
* absolute residual loss <= 50% of paired gain over the sample

### Live shadow-mode KPIs

Run shadow mode for at least **3 trading days**.

Must pass all:

* no adapter disconnect without recovery
* no unreconciled startup state
* no decision logging gaps
* no state machine deadlocks
* hypothetical order intents obey all price and imbalance rules
* zero hypothetical underdog residual increases
* settlement events fully observed and matched

### Deployment gates

Before live canary:

* all unit, property, integration, and replay suites green
* paper KPI gate green
* shadow KPI gate green
* runbooks reviewed
* operator controls verified
* canary config loaded
* low-exposure budget limits enabled

### Live canary gates

Start with:

* `max_concurrent_pairs = 4`
* very small per-market budget
* operator on call
* alerts active
* manual kill switch tested

Canary must show:

* zero hard rule violations
* zero unreconciled mismatches
* zero underdog residual increases
* zero size-increasing adds after 240s
* zero live orders in stop-add or danger zone
* low taker usage

### Rollback triggers

Immediate rollback if any occur:

* any live order intended to add size with effective marginal pair sum >= 1.00
* any approved order increases underdog residual
* any pair reaches unmatched fraction >= 20% due to strategy logic
* any startup or periodic reconciliation mismatch persists beyond configured grace
* any DB write failure while live trading
* any settlement PnL decomposition mismatch
* any duplicate fill changes position twice
* any live sell order appears in MVP
* any hidden directional drift metric breaches portfolio cap

---

## 11. Open assumptions and unknowns

### 1. Exchange order semantics

**Risk**
Maker-first design depends on post-only support and predictable cancel/replace behavior.

**Temporary assumption**
The venue supports limit orders, cancellation, and either post-only or at least non-crossing passive placement.

**Blocks MVP?**
No, for paper and replay.
**Blocks production?**
Yes.

### 2. Tick size and lot size

**Risk**
Invalid price or size rounding can cause rejects or unintended aggression.

**Temporary assumption**
Tick and lot constraints can be discovered from market metadata at runtime and enforced in `domain`.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 3. Maker/taker fee schedule

**Risk**
The true profitable threshold may shift if fees materially change effective pair cost.

**Temporary assumption**
Use a configurable fee buffer, initially conservative, until exact fee rules are confirmed.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 4. Market-open timing precision

**Risk**
If open events are late or ambiguous, the bot may miss the seed window or seed too early.

**Temporary assumption**
Use official `opens_at` plus first tradable post-open book/trade event as confirmation.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 5. Market data shape

**Risk**
Without at least top-of-book and trades, the execution engine and paper model are too weak.

**Temporary assumption**
Live adapter will provide book/trade events; if not, strategy stays shadow-only.

**Blocks MVP?**
No, if paper uses a simplified source.
**Blocks production?**
Yes.

### 6. Position and open-order query semantics

**Risk**
Restart recovery and reconciliation depend on accurate exchange snapshots.

**Temporary assumption**
Venue exposes open orders and fills/positions per market.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 7. Settlement event format and timing

**Risk**
The bot cannot realize PnL or close lifecycle cleanly without authoritative outcome events.

**Temporary assumption**
Venue emits or exposes official settlement outcome per market.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 8. Historical raw event availability

**Risk**
The provided parquet/csv files calibrate logic, but do not provide enough venue microstructure for execution-accurate backtesting.

**Temporary assumption**
Start capturing raw live events immediately and use them for replay certification.

**Blocks MVP?**
No.
**Blocks production?**
Not strictly, but it weakens confidence; strong pre-live blocker.

### 9. Rate limits

**Risk**
Replace-heavy logic can trip venue throttles and create stale orders.

**Temporary assumption**
Conservative cap of one refresh per side per second until documented limits are confirmed.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 10. Order reject semantics and idempotency

**Risk**
A live adapter without strong order-id semantics can create duplicate or orphan orders.

**Temporary assumption**
Client order IDs can be attached and retrieved consistently across order and fill streams.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 11. Optional model-edge source

**Risk**
The historical field `edge_model_minus_price` may not exist live in any venue feed.

**Temporary assumption**
Disable model-edge overlay in MVP.

**Blocks MVP?**
No.
**Blocks production?**
No.

### 12. Persistence backend

**Risk**
Operational instability if persistence is chosen poorly.

**Temporary assumption**
Use Postgres in production and SQLite only for local tests.

**Blocks MVP?**
No.
**Blocks production?**
No.

### 13. Clock synchronization

**Risk**
Timing thresholds are central to this strategy.

**Temporary assumption**
Host clocks are NTP-synchronized; service also uses monotonic elapsed timers per pair.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

### 14. Venue cancel-on-disconnect support

**Risk**
Open passive orders could remain live during a network split.

**Temporary assumption**
If cancel-on-disconnect is not supported, the service must enter global pause and reconcile aggressively after reconnect.

**Blocks MVP?**
No.
**Blocks production?**
Yes.

---

## Recommended MVP defaults

### Initial thresholds

* Seed submit deadline: `5s from confirmed open`
* Second-side target: `15s`
* Second-side hard deadline: `30s`
* Preferred effective marginal pair sum: `< 0.94`
* Acceptable effective marginal pair sum: `< 0.97`
* Caution zone: `0.97-<1.00`
* Stop-add: `>= 1.00`
* Danger: `>= 1.03`
* Target unmatched fraction: `< 7%`
* Warning unmatched fraction: `> 12%`
* Hard disable unmatched fraction: `>= 20%`
* Reduce clip after: `180s`
* Balance-only after: `225s`
* Hard stop new orders after: `240s`
* Seed clip: `12`
* Base clip: `20`
* Accelerated clip: `40`
* Max clip: `80`
* Taker target: `< 5%`
* Taker hard cap: `10%`

### Feature flags

* `enable_live_trading = false`
* `enable_shadow_mode = true`
* `enable_model_edge_overlay = false`
* `enable_clip_jitter = false`
* `enable_multi_level_quotes = false`
* `enable_taker_completion = true`
* `enable_taker_rebalance = true`
* `enable_pre_settlement_sells = false`

### Guardrails

* No single-side speculative adds
* No underdog residual increases
* No size-increasing adds in stop-add or danger zones
* No scale-up before both sides filled
* No new orders after 240s
* No live trading without clean startup reconciliation
* No orders if persistence is down

### What is disabled in MVP

* BTC direction forecasting
* ML core signal
* Intrawindow flipping
* Multi-level quoting
* Randomized execution
* Active liquidation before settlement
* Cross-market or cross-venue hedging

---

## First 30 engineering tickets to start tomorrow

1. Create the Rust workspace, toolchain pin, lints, and CI skeleton.
2. Add domain newtypes for deterministic decimal `Price`, `Qty`, `Cost`, and ID types.
3. Enforce no-float arithmetic in strategy, risk, and ledger crates.
4. Scaffold config loading, validation, feature flags, and config hashing.
5. Add Postgres migrations framework and baseline operational schema.
6. Spin up local Postgres, Prometheus, and Grafana via docker-compose.
7. Build the `analysis_importer` tool and map the provided schema fields exactly.
8. Reproduce the reference analytics metrics and lock them into fixture tests.
9. Implement the strict BTC 5-minute pair registry and Up/Down pairing rules.
10. Define normalized exchange traits and event models in `exchange_core`.
11. Implement raw event capture format for future replay.
12. Build the FIFO lot ledger with fill dedupe.
13. Add paired-lot creation and residual-lot tracking.
14. Implement paired PnL and residual PnL decomposition.
15. Add pair metrics: combined avg paid, unmatched fraction, match ratio, taker share, favorite/underdog.
16. Implement deterministic replay clock and replay event runner.
17. Create the market phase enum and state transition tests.
18. Implement price-zone evaluator with preferred/acceptable/caution/stop/danger states.
19. Implement imbalance rules, including 7%, 12%, and 20% thresholds.
20. Implement the underdog residual hard block.
21. Build seed intent generation with simultaneous two-side seeding.
22. Enforce no scale-up before both sides fill.
23. Implement second-side completion deadlines and completion-only mode.
24. Implement balanced accumulation logic for the `<0.97` zone.
25. Implement balance-only logic for caution or imbalance states.
26. Implement clip ladder logic with 12/20/40/80 and hard cap 80.
27. Implement taker exception logic for second-side rescue and safe rebalance only.
28. Define risk decisions and wire strategy intents through risk before execution.
29. Build order manager state with one working order per side and refresh caps.
30. Build the paper adapter and run the first end-to-end open-to-settlement simulation.
