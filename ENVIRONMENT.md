# Environment Reference

Date: 2026-03-07
Scope: active keys currently present in [`.env`](c:/Works/aldipranata.com/polybot-convert-rust/.env)

This file explains the variables you are actually using today.

It is written for operations, not for code generation.
Raw secret values are intentionally not copied here.

## Notes

1. `EXEC_MODE` appears twice in `.env`, both set to `MAKER_SKEW_ARB`. Effective value is unchanged.
2. `LOG_EVERY_SECONDS` appears twice in `.env`, both set to `2`. Effective value is unchanged.
3. Commented-out keys such as `POLYMARKET_WALLET_ADDRESS` are not active config.
4. Secret keys are documented by purpose only.

---

## Identity / Infra

### `BOT_ID`
- Current value: `maker-skew-arb-btc5m-test-bias-chainlink`
- What it does: unique bot identity used for logs, state files, and operational separation.
- Friendly explanation: think of this as the bot instance name. If you run two variants, this should differ so their state does not collide.

### `ACCOUNT_NAME`
- Current value: `prod`
- What it does: labels the account context for logging and persistence.
- Friendly explanation: this is the human-facing environment tag. It tells you which account profile this bot belongs to.

### `BOT_DESCRIPTION`
- Current value: `MAKER_SKEW_ARB_BTC_5M`
- What it does: descriptive label stored in trade records and status output.
- Friendly explanation: this is the short explanation of what the bot is supposed to be doing.

### `DB_URL`
- Current value: secret
- What it does: PostgreSQL connection used for trade records and runtime persistence.
- Friendly explanation: this is the database address. If it is wrong, trade rows and persistent records will not update.

---

## Polymarket Auth / Connectivity

### `POLYMARKET_PRIVATE_KEY`
- Current value: secret
- What it does: signing key for order creation and authenticated actions.
- Friendly explanation: this is the wallet key that actually authorizes trading. It is the most sensitive value in the file.

### `POLYMARKET_FUNDER`
- Current value: secret-like account identifier
- What it does: funding / signer association expected by the Polymarket client.
- Friendly explanation: this tells the client which funded account is backing the trading key.

### `CHAIN_ID`
- Current value: `137`
- What it does: selects Polygon mainnet.
- Friendly explanation: this tells the signer which blockchain network it is operating on.

### `SIGNATURE_TYPE`
- Current value: `1`
- What it does: selects the signature mode expected by the Polymarket CLOB client.
- Friendly explanation: this is a compatibility knob for how orders are signed.

### `CLOB_HOST`
- Current value: `https://clob.polymarket.com`
- What it does: REST endpoint for orderbook and order actions.
- Friendly explanation: this is the main trading API host.

### `WS_BASE`
- Current value: `wss://ws-subscriptions-clob.polymarket.com`
- What it does: websocket endpoint for subscriptions and live events.
- Friendly explanation: this is the live stream connection for market and user updates.

---

## Market Target Selection

### `MARKET_SYMBOL`
- Current value: `BTC`
- What it does: selects the underlying asset family.
- Friendly explanation: this tells the bot to trade Bitcoin prediction markets, not ETH or another asset.

### `MARKET_SEGMENT`
- Current value: `5M`
- What it does: selects the market interval family.
- Friendly explanation: this points the bot to the 5-minute market series.

### `MARKET_STEP_SECONDS`
- Current value: `300`
- What it does: expected spacing between sequential markets.
- Friendly explanation: each market starts every 5 minutes.

### `MARKET_DURATION_SECONDS`
- Current value: `300`
- What it does: expected duration of each market.
- Friendly explanation: each individual market lives for 5 minutes.

### `MARKET_SLUG_STYLE`
- Current value: `TIMESTAMP`
- What it does: tells the bot how to construct or interpret market slugs.
- Friendly explanation: the bot expects market names that encode timestamps.

### `AUTO_DETECT_MARKET_PARAMS`
- Current value: `true`
- What it does: allows the bot to infer market parameters automatically instead of requiring manual slug config.
- Friendly explanation: the bot will find the current BTC 5-minute market on its own.

---

## Execution Mode

### `EXEC_MODE`
- Current value: `MAKER_SKEW_ARB`
- What it does: selects the top-level engine.
- Friendly explanation: the main production baseline still uses `MAKER_SKEW_ARB` with the Step 1 pair-base overlays disabled, but Sprint 4 also adds `EXEC_MODE=WALLET_CLONE` as a separate wallet-clone runtime.

### `DRY_RUN`
- Current value: `false`
- What it does: enables real trading instead of simulation.
- Friendly explanation: with this set to `false`, the bot really places orders.

---

## Loop / Logging Controls

### `LOOP_WAIT_SECONDS_MAKER`
- Current value: `0.20`
- What it does: sleep interval for the maker loop.
- Friendly explanation: this controls how often the bot reevaluates maker logic. Lower means more reactive and more CPU / API churn.

### `LOG_EVERY_SECONDS`
- Current value: `2`
- What it does: periodic status log interval.
- Friendly explanation: every 2 seconds you get the main PnL / inventory heartbeat line.

### `DEBUG_MODE`
- Current value: `false`
- What it does: global debug switch.
- Friendly explanation: keep this off in normal runs unless you are actively diagnosing something noisy.

### `MAKER_DEBUG`
- Current value: `false`
- What it does: maker-path-specific debug switch.
- Friendly explanation: use this only when you want more maker lifecycle detail.

---

## Step 1 Routing Flags

### `PAIR_BASE_ENABLED`
- Current value: `true`
- What it does: enables the Step 1 pair-base path.
- Friendly explanation: this turns on the pair-first builder instead of a quote-only fallback.

### `PAIR_RECOVERY_ENABLED`
- Current value: `true`
- What it does: enables explicit recovery / merge handling after asymmetric fills.
- Friendly explanation: without this, the bot would not own the mismatch properly once only one side fills.

### `MAKER_SKEW_ENABLED`
- Current value: `false`
- What it does: disables the old directional skew path.
- Friendly explanation: this is how you keep Step 1 focused on pair-building instead of one-sided accumulation.

### `MAKER_ARB_ENABLED`
- Current value: `false`
- What it does: disables the older maker-arb path.
- Friendly explanation: this avoids a second pair-style engine interfering with Step 1.

### `MAKER_STRETCH_BIAS_ENABLED`
- Current value: `false`
- What it does: disables stretch / bias overlay logic.
- Friendly explanation: this keeps Step 1 neutral. No directional opinion is added.

---

## Step 1 Budget / Risk Envelope

### `PAIR_BASE_WINDOW_BUDGET_USDC`
- Current value: `30`
- What it does: budget for opening new pair entries.
- Friendly explanation: this is the normal working budget. Pair building should stay inside this lane.

### `PAIR_BASE_MERGE_BUDGET_USDC`
- Current value: `40`
- What it does: budget allowed for recovery / merge completion.
- Friendly explanation: this gives the bot more room to repair an already-open mismatch than it gets for opening fresh pairs.

### `PAIR_BASE_HARD_RESERVE_USDC`
- Current value: `5`
- What it does: reserve that pair entry should leave unused.
- Friendly explanation: this is the cash cushion. It stops the bot from fully consuming the market budget too early.

### `PAIR_BASE_MAX_WORST_CASE_LOSS_USDC`
- Current value: `12`
- What it does: worst-case loss threshold used by Step 1 risk logic.
- Friendly explanation: if the pair starts looking too ugly on a downside basis, this is part of what pushes the bot toward terminal handling instead of continued recovery.

### `MAX_TOTAL_COST`
- Current value: `60`
- What it does: hard absolute cost ceiling for the market instance.
- Friendly explanation: this is the hard wall. The bot should never keep adding market cost past this.

### `RESERVE_USD`
- Current value: `2`
- What it does: generic reserve used by the broader runtime config.
- Friendly explanation: this is the older global reserve cushion. Step 1 also has its own pair-base reserve above.

---

## Position Sizing

### `MIN_SHARES`
- Current value: `5`
- What it does: minimum normal order size in shares.
- Friendly explanation: this is the standard clip floor for normal maker behaviour. Smaller leftovers are treated specially by the sub-min gap policy.

### `CLIP_SHARES`
- Current value: `8`
- What it does: preferred upper clip size for pair entry.
- Friendly explanation: a normal pair attempt can go up to 8 shares if budget and gates allow it.

---

## Entry Asymmetry Controls

### `ENTRY_ACK_TIMEOUT_MS`
- Current value: `800`
- What it does: sequential first-leg ack budget for Step 1 pair entry.
- Friendly explanation: if the first pair leg takes longer than this to come back with an order id, the bot can abort the second leg instead of creating a slow asymmetric pair. This is now a real safety budget, but it is still not a fully concurrent two-leg ack timer.

### `ENTRY_FIRST_CLIP_SCALE`
- Current value: not set in `.env`
- Effective default: `0.5`
- What it does: scales down the first pair-entry clip when the book is flat.
- Friendly explanation: the first pair is the riskiest to arm because the bot has no inventory cushion yet. This keeps the opening exposure smaller than later clips.

### `ENTRY_REQUIRE_BOTH_ACKS`
- Current value: not set in `.env`
- Effective default: `true`
- What it does: requires both pair-entry legs to come back with valid order ids before treating the pair as armed.
- Friendly explanation: this stops the bot from casually accepting half-submitted pairs as normal state.

### `ENTRY_CANCEL_OTHER_ON_NO_OID`
- Current value: not set in `.env`
- Effective default: `true`
- What it does: cancels the surviving leg if the other leg rejects or returns no order id.
- Friendly explanation: if one side never really armed, the other side should not keep resting by itself.

---

## Market Safety / Timing

### `MARKET_DATA_STALE_SECONDS`
- Current value: `8`
- What it does: feed stale threshold.
- Friendly explanation: if market data is older than 8 seconds, the bot pauses and cancels rather than trading blind.

### `STOP_BUFFER_SECONDS`
- Current value: `15`
- What it does: final stop-new-orders buffer before expiry / rollover.
- Friendly explanation: this is the last hard quiet period before the market ends.
- Current runtime nuance: when `MARKET_SLUG` is auto-generated in timestamp mode, startup inside this buffer now selects the next market slot instead of attaching to the nearly-expired current slot.


### `PAIR_BASE_NEAR_EXPIRY_FORCE_TAKER_SECONDS`
- Current value: `55`
- What it does: window in which the near-expiry taker price-cap override is allowed.
- Friendly explanation: once the market is inside the last 55 seconds, terminal taker rescue is allowed to be much more aggressive on price.
- Current runtime nuance: the same override is also reused immediately for `forced_negative_economics` exits and their latched retries, so Step 1 can escalate earlier without waiting for this window.

### `PAIR_BASE_NEAR_EXPIRY_RISK_EXIT_SECONDS`
- Current value: `50`
- What it does: explicit trigger window for entering `RiskExitOnly`.
- Friendly explanation: Step 1 starts terminal cleanup when there are 50 seconds or less left, instead of waiting for the very last moment.

### `PAIR_BASE_NEAR_EXPIRY_TAKER_MAX_PRICE`
- Current value: `1`
- Effective runtime clamp: `0.99`
- What it does: maximum temporary taker-buy price allowed in the near-expiry override window.
- Friendly explanation: this is the "flatten no matter what" switch. With this near `1`, the bot stops self-blocking on price cap near expiry.
- Current runtime nuance: this same cap is now applied immediately to `forced_negative_economics` exits and to later `pair_base_latched` retries that originated from that forced-exit path.

### `PAIR_BASE_SUB_MIN_GAP_POLICY`
- Current value: `taker_immediate`
- What it does: policy for unresolved gaps below `MIN_SHARES`.
- Friendly explanation: when the leftover mismatch is too small for normal maker recovery, the bot does not just wait. In this mode it immediately routes that small tail into the exact taker exit path.

### `RISK_EXIT_TERMINAL_WINDOW_S`
- Current value: not set in `.env`
- Effective default: `45`
- What it does: terminal window used by the staged recovery scorer and a lower bound for the actual near-expiry `RiskExitOnly` trigger.
- Friendly explanation: inside this window the bot compares rescue actions directly instead of staying overly attached to maker purity, and Step 1 will not start near-expiry cleanup later than this unless a larger explicit risk-exit window is configured.

### `RISK_EXIT_ALLOW_TAKER_BUY`
- Current value: not set in `.env`
- Effective default: `true`
- What it does: allows terminal taker BUY when the missing light side must be bought back.
- Friendly explanation: if this is turned off, the scorer will treat missing-leg taker BUY as blocked even if it would flatten the risk.

### `RISK_EXIT_ALLOW_TAKER_SELL`
- Current value: not set in `.env`
- Effective default: `true`
- What it does: allows terminal exact/taker SELL when reducing the heavy side is the cheaper rescue.
- Friendly explanation: if this is off, the bot cannot use heavy-side sell as the preferred non-maker rescue path.

---

## Recovery Speed / Quote Refresh Tuning

### `PAIR_BASE_REFRESH_SECONDS`
- Current value: `1`
- What it does: refresh cadence for pair-base recovery decisions.
- Friendly explanation: the recovery loop reevaluates roughly every second. Lower is more reactive; higher is more patient.

### `MIN_ENTRY_EDGE_TICKS`
- Current value: `2`
- What it does: minimum runtime floor for entry edge.
- Friendly explanation: this is a floor, not a direct override. It prevents the bot from quoting with less than 2 ticks of edge.

### `ENTRY_EDGE_TICKS`
- Current value: `2`
- What it does: base entry edge target used by the runtime config.
- Friendly explanation: this is the actual main quote aggressiveness knob. Lower means closer to the market and easier fills; higher means more patience and better price discipline.

### `REPLACE_IF_PRICE_MOVES_TICKS`
- Current value: `2`
- What it does: repricing sensitivity when the market moves.
- Friendly explanation: if the relevant price moves by 2 ticks, the bot considers replacing the quote instead of leaving it stale.

### `STALE_SECONDS`
- Current value: `3`
- What it does: age threshold for treating quotes as stale.
- Friendly explanation: after about 3 seconds, a resting quote is considered old enough to refresh if the other conditions say it should.

### `MAKER_REPLACE_MIN_INTERVAL_SECONDS`
- Current value: `0.5`
- What it does: minimum gap between maker replacements.
- Friendly explanation: this stops the bot from cancel/reposting too rapidly.

### `MAKER_CANCEL_PENDING_TTL_SECONDS`
- Current value: `1`
- What it does: how long a cancel-pending maker order is treated as unresolved.
- Friendly explanation: this is part of why recovery can wait before placing the next quote. The bot assumes a just-canceled order may still matter for a short time.

### `RECOVERY_TICK_MS`
- Current value: not set in `.env`
- Effective default: `300`
- What it does: dedicated decision cadence while Step 1 is in `MergePending` or `RiskExitOnly`.
- Friendly explanation: this is the fast recovery loop. Lower means faster reactions to fills, cancels, and book moves during recovery.

### `RECOVERY_LIVE_ORDER_TTL_MS`
- Current value: not set in `.env`
- Effective default: `400`
- What it does: maximum age for trusting a light-side order as valid live coverage.
- Friendly explanation: after this TTL, a “live” recovery quote stops getting the benefit of the doubt unless the bot has fresh evidence that it is still useful.

### `RECOVERY_REQUOTE_ON_BID_MOVE_TICKS`
- Current value: not set in `.env`
- Effective default: `1`
- What it does: number of ticks the light-side book can move before the recovery quote is considered stale.
- Friendly explanation: small values make the bot refresh the missing-leg quote sooner instead of waiting through obvious drift.

### `RECOVERY_REQUOTE_ON_STALE_BOOK_MS`
- Current value: not set in `.env`
- Effective default: `500`
- What it does: maximum age of the orderbook snapshot before a recovery quote is treated as stale.
- Friendly explanation: Step 1 recovery should not trust an old order against an old market view.

### `RECOVERY_STALL_ESCALATION_MS`
- Current value: not set in `.env`
- Effective default: `15000`
- What it does: stall duration before the recovery scorer escalates into a later, less patient decision window.
- Friendly explanation: if recovery has already gone nowhere for this long, the bot becomes less tolerant of waiting.

### `RECOVERY_EPSILON_EARLY`
- Current value: not set in `.env`
- Effective default: `0.05`
- What it does: tolerance for early-window maker recovery being slightly worse than waiting.
- Friendly explanation: early in the market the bot still prefers positive maker completion, but it can tolerate a very small negative if waiting is no better.

### `RECOVERY_EPSILON_MID`
- Current value: not set in `.env`
- Effective default: `0.15`
- What it does: tolerance for mid-window maker recovery being slightly worse than waiting.
- Friendly explanation: as time shrinks, the bot should accept slightly uglier maker completion to avoid a worse terminal rescue later.

### `RECOVERY_EPSILON_LATE`
- Current value: not set in `.env`
- Effective default: `0.35`
- What it does: tolerance for late-window maker recovery before terminal rescue becomes preferable.
- Friendly explanation: late in the market, waiting should become expensive, so the bot allows more negative maker completion if it still improves the floor.

### `RECOVERY_TERMINAL_COMPARE_ALL_PATHS`
- Current value: not set in `.env`
- Effective default: `true`
- What it does: allows the scorer to compare maker, exact-sell, taker-buy, and wait directly in the terminal window.
- Friendly explanation: near the end, the bot should choose the least-damaging path, not the cleanest-looking one.

### `RECOVERY_PREFER_EXACT_SELL`
- Current value: not set in `.env`
- Effective default: `true`
- What it does: tie-break bias toward heavy-side exact sell over missing-leg taker BUY.
- Friendly explanation: when two rescue actions are close, this prefers reducing the heavy side early instead of paying up later to buy the missing side.

### `RECOVERY_EXACT_SELL_MIN_DEPTH_BUFFER`
- Current value: not set in `.env`
- Effective default: `1.05`
- What it does: minimum visible bid-depth multiple required before exact heavy-side sell is considered executable.
- Friendly explanation: this protects the scorer from pretending there is enough heavy-side liquidity when the book is actually too thin.

### `RECOVERY_EXACT_SELL_MAX_SLICES`
- Current value: not set in `.env`
- Effective default: `3`
- What it does: caps how large an exact heavy-side sell candidate is allowed to be in recovery scoring, measured in clip-sized slices.
- Friendly explanation: this keeps the recovery model conservative instead of preferring a heavy-side exact sell that would effectively need too many rescue chunks to be sensible.

### `RECOVERY_SHADOW_SCORING_ENABLED`
- Current value: `true`
- What it does: emits shadow scoring logs without changing current recovery behavior.
- Friendly explanation: this is the safe rollout mode. You can see what the new policy would choose before letting it drive the bot.

### `RECOVERY_SCORING_ACTIVE`
- Current value: `true`
- What it does: turns the recovery scorer from observer into decision-maker.
- Friendly explanation: turning this on lets the scorer actually choose recovery actions. Current code still treats it as canary-grade, not a safe default, because taker selection is only partially validated.

### `RECOVERY_SCORING_TAKER_MIN_ADVANTAGE`
- Current value: not set in `.env`
- Effective default: `0.10`
- What it does: minimum score advantage required before scored recovery is allowed to choose `taker_buy_light`.
- Friendly explanation: this stops the scorer from flipping into taker rescue on tiny score differences. If the taker path is only marginally better than maker or waiting, Step 1 keeps the safer non-taker behavior.

---

## Settlement Shaper Canary Controls

These keys only matter when `EXEC_MODE=SETTLEMENT_SHAPER`.

For the keys below:

1. current value in the checked-in `.env`: not set
2. effective value when unset: the built-in Sprint 3 canary default shown below

### Role / Hysteresis

- `FAV_UNDERDOG_SWITCH_MIN_DIFF`
  Default: `0.01`
  What it does: minimum ranked-price gap required before a favorite/underdog flip is even considered.
- `FAV_UNDERDOG_SWITCH_CONFIRM_UPDATES`
  Default: `3`
  What it does: number of consecutive qualifying updates required before the role assignment actually flips.

### Phase Budget Slices

- `SETTLEMENT_SHAPER_BUDGET_SEED_MIN_FRACTION`
  Default: `0.10`
- `SETTLEMENT_SHAPER_BUDGET_SEED_MAX_FRACTION`
  Default: `0.15`
- `SETTLEMENT_SHAPER_BUDGET_EARLY_MIN_FRACTION`
  Default: `0.15`
- `SETTLEMENT_SHAPER_BUDGET_EARLY_MAX_FRACTION`
  Default: `0.20`
- `SETTLEMENT_SHAPER_BUDGET_MAIN_MIN_FRACTION`
  Default: `0.45`
- `SETTLEMENT_SHAPER_BUDGET_MAIN_MAX_FRACTION`
  Default: `0.55`
- `SETTLEMENT_SHAPER_BUDGET_FINISH_MIN_FRACTION`
  Default: `0.15`
- `SETTLEMENT_SHAPER_BUDGET_FINISH_MAX_FRACTION`
  Default: `0.20`
- `SETTLEMENT_SHAPER_BUDGET_FREEZE_MIN_FRACTION`
  Default: `0.05`
- `SETTLEMENT_SHAPER_BUDGET_FREEZE_MAX_FRACTION`
  Default: `0.10`

Friendly explanation: these ten keys split the usable market budget across `SeedBothSides`, `EarlyBuild`, `MainAccumulation`, `FinishShape`, and the late freeze / repair reserve so the mode does not spend the whole window too early.

### Shape Bands And Regime Thresholds

- `SETTLEMENT_SHAPER_PAIR_COVERAGE_SOFT_MIN`
  Default: `0.80`
  What it does: below this, the controller treats coverage as clearly unhealthy.
- `SETTLEMENT_SHAPER_PAIR_COVERAGE_GOOD`
  Default: `0.90`
  What it does: threshold for a healthy paired book and optionality eligibility.
- `SETTLEMENT_SHAPER_SHARE_SKEW_TARGET_LOW`
  Default: `1.05`
- `SETTLEMENT_SHAPER_SHARE_SKEW_TARGET_HIGH`
  Default: `1.20`
- `SETTLEMENT_SHAPER_SHARE_SKEW_SOFT_CAP`
  Default: `1.30`
- `SETTLEMENT_SHAPER_SHARE_SKEW_HARD_CAP`
  Default: `1.40`
  Friendly explanation: these define the healthy skew band and the soft/hard limits beyond it.
- `SETTLEMENT_SHAPER_FAVORITE_COST_FRACTION_LOW`
  Default: `0.60`
- `SETTLEMENT_SHAPER_FAVORITE_COST_FRACTION_HIGH`
  Default: `0.67`
- `SETTLEMENT_SHAPER_UNDERDOG_SHARE_FRACTION_LOW`
  Default: `0.51`
- `SETTLEMENT_SHAPER_UNDERDOG_SHARE_FRACTION_HIGH`
  Default: `0.60`
  Friendly explanation: these define the healthy target band for dollars on the favorite and shares on the underdog.
- `SETTLEMENT_SHAPER_VWAP_SUM_GREAT`
  Default: `0.94`
- `SETTLEMENT_SHAPER_VWAP_SUM_GOOD`
  Default: `0.97`
- `SETTLEMENT_SHAPER_VWAP_SUM_STOP_OVERLAY`
  Default: `1.00`
  Friendly explanation: these thresholds classify live/inventory cost quality into green, good, caution, and stop-overlay regimes.

### Target Centers

- `SETTLEMENT_SHAPER_TARGET_PAIR_COVERAGE`
  Default: `0.90`
- `SETTLEMENT_SHAPER_TARGET_SHARE_SKEW_RATIO`
  Default: `1.10`
- `SETTLEMENT_SHAPER_TARGET_FAVORITE_COST_FRACTION`
  Default: `0.635`
- `SETTLEMENT_SHAPER_TARGET_UNDERDOG_SHARE_FRACTION`
  Default: `0.555`

Friendly explanation: these are the neutral centers used by `ShapeRepair` and normal-path action scoring when the controller measures target gaps.

### Clip Ladder

- `SETTLEMENT_SHAPER_CLIP_LADDER_SMALL`
  Default: `5,10,20,25`
  What it does: comma-separated small-clip ladder used by entry repair, shape repair, and underdog overlay.
- `SETTLEMENT_SHAPER_CLIP_LADDER_MEDIUM`
  Default: `40`
  What it does: medium clip used in stronger healthy-book repair / size-up cases.
- `SETTLEMENT_SHAPER_CLIP_LADDER_LARGE`
  Default: `80`
  What it does: large clip reserved for main-accumulation favorite-side size-up only.

Friendly explanation: the settlement-shaper controller no longer emits arbitrary share sizes. It snaps actions onto this ladder so canary behavior is predictable and metrics are comparable by bucket.

---

## Wallet Clone Controls

These keys only matter when `EXEC_MODE=WALLET_CLONE`.

For the keys below:

1. current value in the checked-in `.env`: not set
2. effective value when unset: the built-in Sprint 4 default shown below

### Startup And Timing

- `WALLET_CLONE_PREARM_LEAD_SECONDS`
  Default: `20`
  What it does: how early the wallet-clone path starts enforcing pre-open readiness checks.
- `WALLET_CLONE_TARGET_BOTH_SIDES_BY_30S`
  Default: `0.80`
  What it does: operator target used in logs and metrics for startup completion by 30 seconds.
- `WALLET_CLONE_TARGET_BOTH_SIDES_BY_60S`
  Default: `0.95`
  What it does: operator target used in logs and metrics for startup completion by 60 seconds.
- `WALLET_CLONE_TAPER_START_SECONDS`
  Default: `240`
  What it does: point in the 300-second market when aggressive expansion gives way to taper maintenance.
- `WALLET_CLONE_FINAL_QUIET_SECONDS`
  Default: `30`
  What it does: final quiet window where almost all new expansion is suppressed.

Friendly explanation: these keys define the wallet-clone rhythm. The path should be armed before open, aggressive through most of the market, lighter after 240 seconds, and nearly silent in the last 30 seconds.

### Clip Sizing

- `WALLET_CLONE_SEED_CLIP_SMALL`
  Default: `15`
  What it does: small paired seed clip used by `OpenBoth`.
- `WALLET_CLONE_REPAIR_CLIP_SMALL`
  Default: `15`
  What it does: small missing-side repair clip used by `SeedCompletion`.
- `WALLET_CLONE_REPAIR_RESERVE_BUFFER_USD`
  Default: `1.0`
  What it does: extra wallet-clone budget cushion that `PairBuild` preserves on top of the likely lighter-side repair cost before allowing optional paired growth.
- `WALLET_CLONE_CLIP_LADDER_LARGE`
  Default: `40,80`
  What it does: comma-separated large clip ladder used by `PairBuild` for aggressive replenishment.

Friendly explanation: Sprint 4 does not use a single giant opener. It uses small startup clips, exact-gap repair sizing, and a protected repair reserve before scaling into repeated larger passive adds.

### Phase Budget Slices

- `WALLET_CLONE_BUDGET_SEED_MIN_FRACTION`
  Default: `0.10`
- `WALLET_CLONE_BUDGET_SEED_MAX_FRACTION`
  Default: `0.15`
- `WALLET_CLONE_BUDGET_EARLY_MIN_FRACTION`
  Default: `0.15`
- `WALLET_CLONE_BUDGET_EARLY_MAX_FRACTION`
  Default: `0.20`
- `WALLET_CLONE_BUDGET_MAIN_MIN_FRACTION`
  Default: `0.45`
- `WALLET_CLONE_BUDGET_MAIN_MAX_FRACTION`
  Default: `0.55`
- `WALLET_CLONE_BUDGET_LATE_MIN_FRACTION`
  Default: `0.15`
- `WALLET_CLONE_BUDGET_LATE_MAX_FRACTION`
  Default: `0.20`
- `WALLET_CLONE_BUDGET_TAPER_MIN_FRACTION`
  Default: `0.05`
- `WALLET_CLONE_BUDGET_TAPER_MAX_FRACTION`
  Default: `0.10`

Friendly explanation: these ten keys split the usable market budget across startup, early build, main build, late build, and taper reserve without reintroducing Sprint 3 shape goals.

### Behavior Guardrail

- `WALLET_CLONE_BUY_ONLY_NORMAL_FLOW`
  Default: `true`
  What it does: keeps normal wallet-clone flow on the observed `BUY`-only path instead of adding live sell-style shaping or exits.

Friendly explanation: this is not a conservative profitability gate. It is just a guardrail that keeps Sprint 4 on the observed wallet behavior instead of drifting back into controller-style cleanup logic. Setting it to `false` is currently unsupported and the wallet-clone loop will fail closed instead of pretending a non-buy path exists.

### Shared Maker Cadence

Wallet-clone reuses the existing general maker cadence controls instead of adding a second competing cadence surface:

- `ENTRY_EDGE_TICKS`
- `PAIR_BASE_REFRESH_SECONDS`
- `REPLACE_IF_PRICE_MOVES_TICKS`
- `STALE_SECONDS`

Friendly explanation: Sprint 4 gets its own timing and sizing knobs above, but quote refresh speed and reprice cadence still come from the shared maker controls already documented earlier in this file.

---

## Practical Reading Of This `.env`

This configuration is currently saying:

1. run real trading
2. use the Step 1 pair-base engine
3. keep skew / arb / stretch overlays off
4. build maker pairs inside a modest working budget
5. attempt maker recovery first
6. use aggressive terminal taker rescue near expiry
7. immediately taker-clean sub-min mismatch tails
8. run recovery shadow scoring and active scoring together, but with taker rescue guarded by a minimum advantage threshold
9. quote recovery relatively aggressively with `ENTRY_EDGE_TICKS=2`

The main tradeoff of this setup is:

1. safer terminal flattening
2. but still conservative maker recovery if quotes are repeatedly invalidated or become uneconomic

---

## Friendly Summary By Intent

If you only want the shortest operator summary:

1. `PAIR_BASE_*` keys control Step 1 pair building, recovery, and terminal cleanup.
2. `ENTRY_EDGE_TICKS`, `PAIR_BASE_REFRESH_SECONDS`, `REPLACE_IF_PRICE_MOVES_TICKS`, and `STALE_SECONDS` control how aggressively the missing light side is repriced.
3. `PAIR_BASE_NEAR_EXPIRY_*` keys control how strongly the bot prioritizes flattening near expiry.
4. `MAX_TOTAL_COST`, `PAIR_BASE_WINDOW_BUDGET_USDC`, `PAIR_BASE_MERGE_BUDGET_USDC`, and `PAIR_BASE_HARD_RESERVE_USDC` control how much money the bot is allowed to put to work.

If you want, the next useful documentation step is a second section that says:

1. which variables are safe for daily tuning
2. which variables are dangerous and should rarely change
3. which variables only matter for debugging or rescue behaviour

