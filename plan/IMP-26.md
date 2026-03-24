# IMP-26: Vidardx-Style Continuous Accumulation (Remove Hard Pause at Pair Sum >= 1.00)

## Problem

The bot currently hard-pauses when pair sum >= 1.00, leaving a naked one-sided position.
Vidardx trade data shows that **76.8% of markets temporarily cross above 1.00** but 60.3%
come back below through continued cheap accumulation on the lagging side.

The current bot treats pair_sum >= 1.00 as terminal. Vidardx treats it as temporary.

## Evidence (from `dataset/analyze_pair_sum_timeline.py`)

- 149/194 markets (76.8%) temporarily cross above 1.00
- 117/194 markets (60.3%) cross above then come back below
- Profitable markets average 2.1 crossings above 1.00, spending 20.8% of trades above
- Top profitable market: pair sum peaked at 1.1180, finished at 0.9290 (+$901)
- Markets cross above/below an average of ~2x each — it's oscillatory, not one-way

## Strategy: Keep Trading Through the 1.00 Boundary

Instead of hard-pausing, continue placing **maker limit orders on the lagging side** at
cheap prices. The running VWAP will naturally decrease as cheap fills accumulate.

## Changes Required

### 1. Remove `rescue_pair_sum_too_high` Hard Pause (startup.rs ~L1870)

**Current:** At 30s deadline, if `marginal_pair_sum >= 1.0`, hard pause permanently.
**New:** Skip the hard pause. Instead, fall through to continued maker order flow.
The rescue FAK taker is still gated at 1.0 (don't overpay with takers), but
the bot does NOT hard pause — it continues posting maker limit orders.

### 2. Raise Edge Cap from 1.00 to 1.05 (startup.rs ~L2112)

**Current:** `max_missing_price = (1.00 - filled_vwap).max(0.01)` — caps maker bid so
pair sum stays <= 1.00.
**New:** `max_missing_price = (1.05 - filled_vwap).max(0.01)` — allows the bot to post
maker bids that would result in pair sums up to 1.05. This gives breathing room to get
fills while still preventing absurd overpayment. The 0.05 tolerance matches Vidardx's
avg max pair sum of ~1.03.

### 3. Allow AwaitSecondFill to Continue Past Deadline (startup.rs ~L1661)

**Current:** At 30s deadline: cancel maker, attempt rescue FAK, hard pause if rescue fails.
**New:** At 30s deadline: attempt rescue FAK if pair_sum < 1.0 (existing logic). If rescue
is blocked (pair_sum >= 1.0), **don't hard pause** — instead continue the pre-deadline
maker order flow. The bot keeps posting maker limit orders on the missing side.

### 4. PairBuild StopAdd Bypass for Lagging Side (handler.rs ~L60-82)

**Current:** StopAdd blocks all orders except HardDisable/Warning imbalance repairs.
**New:** Also allow LighterSideFirst repairs in `Throttle` imbalance state when pair sum
is in StopAdd zone. This lets pair_build continue accumulating the lagging side even when
the running pair sum is temporarily above 1.00.

## What NOT to Change

- **Taker rescue gate at 1.0:** Keep blocking FAK taker rescue when pair_sum >= 1.0.
  Takers pay the ask price (expensive). Only maker limit orders should be used above 1.0.
- **PairBuild PairedGrowth StopAdd:** Keep blocking balanced growth at pair_sum >= 1.0.
  Only lagging-side repair should be allowed above 1.0.
- **Danger zone at 1.03:** Keep as a hard ceiling for pair_build. Don't post paired-growth
  bids that would push pair sum above 1.03.

## Implementation Details

### Change 1: startup.rs — rescue_pair_sum_too_high → skip rescue, keep maker

```rust
// BEFORE (L1870-1891):
if marginal_pair_sum >= 1.0 - 1e-9 {
    self._bot_runtime_mark_await_second_fill_hard_paused(...);
    return;
}

// AFTER:
if marginal_pair_sum >= 1.0 - 1e-9 {
    // Pair sum too high for taker rescue — skip rescue but DON'T hard pause.
    // Continue posting maker limit orders (fall through to maker flow below).
    self.logger.info(&format!(
        "[BOT][AWAIT_SECOND_FILL] pair_id={} rescue_skipped_pair_sum_high:{:.3} — continuing maker flow",
        pair_id, marginal_pair_sum,
    ));
    // Mark rescue as "used" to prevent re-attempting taker rescue,
    // but do NOT set hard_paused.
    if let Ok(mut st) = self.bot_runtime_state.lock() {
        st.await_second_fill_rescue_used = true;
    }
    // Fall through to maker order placement below
}
```

### Change 2: startup.rs — Raise edge cap

```rust
// BEFORE (L2112):
let max_missing_price = (1.00 - filled_vwap).max(0.01);

// AFTER:
let max_missing_price = (1.05 - filled_vwap).max(0.01);
```

### Change 3: startup.rs — Post-deadline flow restructure

The deadline block (L1661+) currently forces: cancel maker → rescue → hard pause.
Restructure to: attempt rescue if viable → if rescue blocked, continue maker flow.

The key insight: after deadline, if rescue fails/blocked, we should NOT hard pause.
Instead, we should continue to the maker order upsert code (L2124+) that posts a
GTC limit order on the missing side.

### Change 4: handler.rs — Extend StopAdd bypass

```rust
// BEFORE (L75-80):
let imbalance_repair = decision.mode == BotRuntimePairBuildMode::LighterSideFirst
    && matches!(
        runtime_imbalance_state,
        BotRuntimeImbalanceState::HardDisable | BotRuntimeImbalanceState::Warning
    );

// AFTER:
let imbalance_repair = decision.mode == BotRuntimePairBuildMode::LighterSideFirst
    && matches!(
        runtime_imbalance_state,
        BotRuntimeImbalanceState::HardDisable
            | BotRuntimeImbalanceState::Warning
            | BotRuntimeImbalanceState::Throttle
    );
```

## Risk Mitigation

- The 1.05 edge cap prevents runaway overpayment on maker orders
- Maker-only above 1.0 (no taker rescue) limits execution risk
- Existing budget caps still apply
- The bot still stops new orders in Taper phase (t > 180s)
- PairedGrowth remains blocked at StopAdd — only lagging-side repair allowed
