# IMP-19 Plan: Configurable Pair and Wallet-Global Gross Deployed Cost Caps

## Summary
Implement `REQ-032` by adding explicit gross deployed cost caps that are enforced before any maker or taker order approval.

Chosen decisions:
- use explicit new envs, not `MAX_TOTAL_COST`, as the authoritative gross-cap surface
- keep `MAX_TOTAL_COST` and the existing budget fractions as separate spend/allocation controls
- enforce portfolio caps wallet-globally across same-wallet multi-instance deployments
- scope `IMP-19` to gross deployed cost only; do not add unmatched-gross enforcement in this task

Gross deployed cost for this task means:
- filled pair cost already on the books
- plus pending BOT order reservations that can still turn into additional spend
- plus the candidate order being approved right now

## Config and Interfaces
- Add these first-class config fields to the versioned config bundle:
  - `BOT_PAIR_GROSS_DEPLOYED_COST_CAP_USD`
  - `BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD`
  - `BOT_PAIR_GROSS_DEPLOYED_COST_BUFFER_USD`
  - `BOT_PORTFOLIO_GROSS_DEPLOYED_COST_BUFFER_USD`
  - `BOT_GROSS_CAP_INCLUDE_PENDING_MAKER`
  - `BOT_GROSS_CAP_INCLUDE_PENDING_TAKER`
  - `BOT_GROSS_CAP_SHARED_STATE_TTL_SECONDS`
- Defaults:
  - pair cap defaults to current `MAX_TOTAL_COST`
  - portfolio cap defaults to `4 * pair cap` to match the requirement appendix’s recommended first-live `max_concurrent_pairs = 4`
  - both buffers default to `0`
  - both pending-inclusion booleans default to `true`
  - shared-state TTL defaults to `30s`
- Validation:
  - both caps must be finite and `> 0`
  - buffers must be finite, `>= 0`, and strictly below their cap
  - TTL must be finite and `> 0`
- Keep `MAX_TOTAL_COST` untouched as the current pair-budget control; gross caps are an outer hard guard, not a rename of budget logic.
- Extend the versioned snapshot, env contract, startup effective-config log, and audit payloads with the new gross-cap fields.

## Implementation Changes
### 1. Gross exposure model
- Add one shared gross snapshot helper used by all approval paths:
  - current pair filled gross
  - current pair pending maker gross
  - current pair pending taker gross
  - requested order gross
  - projected pair gross
  - portfolio filled gross
  - portfolio pending gross
  - projected portfolio gross
- Pair cap uses:
  - current pair filled gross from in-memory state
  - plus current pair pending reservations
  - plus the candidate request
- Portfolio cap uses wallet-global shared state:
  - per-trade filled gross snapshots from all live trades on the wallet
  - plus pending order reservations from all instances on the wallet
  - plus the candidate request
- The current trade’s in-memory filled gross must replace any stale shared copy for that same trade during the check so the local bot never double-counts itself.
- Pair submits must be gated on the combined YES+NO request cost once, before either leg is posted.
- Replacement refreshes must subtract the reservations being intentionally replaced before adding the new request, so cancel-replace activity is not double-counted as additive exposure.
- Protective cancels and terminal drains bypass gross-cap blocking because they reduce exposure.

### 2. Shared wallet-global state
- Add a new shared companion file, one per wallet, for gross-cap state.
- Store two append-safe maps:
  - live trade gross snapshots keyed by `trade_id` with `pair_id`, `gross_filled_cost`, `updated_ts`
  - pending order reservations keyed by `order_id` with `trade_id`, `pair_id`, `asset_id`, `origin`, `side`, `price`, `size`, `applied_size`, `updated_ts`, and maker/taker kind
- Use the existing companion-lock pattern for all read-modify-write operations.
- Trim stale entries by TTL, but keep them alive by refreshing:
  - current trade filled-gross snapshot on startup, on fills, and periodically during the runtime loop
  - pending reservations on submit success, user-order updates, fills, cancels, and reconciliation
- On startup/reconnect reconciliation, republish the current market’s live BOT orders into the shared reservation state so restart or context pruning does not drop pending exposure.
- If shared gross state cannot be read or written, enter the existing dependency-pause path and block new risk until it recovers; use a dedicated stable reason such as `dependency_pause:database:gross_cap_state`.

### 3. Enforcement points
- Enforce gross caps in the order-approval layer, not just in pair-build decisions:
  - maker pair submit
  - maker single-side submit
  - await-second-fill/rescue submit
  - taper repair or maintenance submit
  - taker exception submits
- Stable block reasons:
  - `gross_cap_market`
  - `gross_cap_portfolio`
- Add gross-cap snapshot fields to the existing risk-block and decision audit payloads so every block or allowed submit records:
  - pair cap
  - portfolio cap
  - current pair gross
  - projected pair gross
  - current portfolio gross
  - projected portfolio gross
  - requested gross
  - whether pending maker/taker were included

## Test Plan
- Config tests:
  - default pair cap derives from `MAX_TOTAL_COST`
  - default portfolio cap derives from `4 * pair cap`
  - invalid caps, invalid buffers, and invalid TTL fail validation
  - old config snapshots load with additive defaults
- Pair-cap behavior:
  - startup paired seed is blocked when the combined YES+NO request would exceed the pair cap
  - pair-build or taper single-side repair is blocked when projected pair gross exceeds the pair cap
  - taker submit is blocked by projected pair gross before approval
- Portfolio-cap behavior:
  - same-wallet second bot is blocked when another market has already consumed enough filled or pending gross
  - current bot uses its in-memory trade gross instead of stale shared data for self-calculation
  - replacement refreshes do not double-count old reservation plus new request
- Shared-state behavior:
  - maker and taker pending reservations count before fills
  - partial fills reduce remaining reservation cost
  - cancel/full fill removes the reservation
  - restart/reconciliation repopulates reservations for live BOT orders
  - stale shared entries age out by TTL after crash
- Failure-path behavior:
  - unreadable or unwritable gross-cap shared state triggers dependency pause
  - trading does not resume until shared state is healthy again
- Audit/observability:
  - `gross_cap_market` and `gross_cap_portfolio` blocks emit the expected payload fields
  - allowed submits record the gross snapshot used for approval

## Assumptions and Defaults
- `IMP-19` enforces gross deployed cost only; unmatched-gross limits remain later work.
- Portfolio gross caps are wallet-global, not per-process.
- Paper and replay modes are still future tasks; this task should put the gross-cap math in shared reusable helpers so those modes can call the same logic later.
- `MAX_TOTAL_COST` remains the existing pair-budget control and is not renamed; gross caps are separate, harder ceilings layered above current budget logic.
- If `BOT_PORTFOLIO_GROSS_DEPLOYED_COST_CAP_USD` is set below the pair cap, that is allowed and simply means the portfolio cap dominates.
