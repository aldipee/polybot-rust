# CURRENT BEHAVIOUR

## Version 0.1.27

Scope: Sprint 3 only  
Mode: `EXEC_MODE=SETTLEMENT_SHAPER`  
Date: 2026-03-11

This file is not a design target.
It is a concrete runtime behaviour note for the current `SETTLEMENT_SHAPER` canary.

---

## Update Note

This note now reflects the latest live canary on 2026-03-11 after the recent controller fixes for:

1. maker BUY precision quantization
2. exact-to-lot repair sizing
3. sub-lot / no-legal-repair rest fallback into `PairResting`
4. paired-growth mild rebuild allowance
5. paired-growth family live-order waiting and stale-cancel handling
6. maker-style near-expiry rollover

Those fixes materially improved runtime mechanics.

The latest run proves that:

1. routing into `SETTLEMENT_SHAPER` still works
2. seed submission still works
3. startup asymmetry ownership still works
4. exact repair sizing is still in place
5. maker-style rollover still works

But the current live behaviour is still not a Sprint 3 wallet match.
The newest dominant blocker is now:

1. a one-leg seed outcome leaves the bot in true `EntryRepair`
2. `EntryRepair` correctly detects the missing side as startup asymmetry
3. the actual missing-side repair is then rejected every tick as `hard_skew_breach`
4. the bot remains one-sided for the full market and rolls over without ever restoring two-sided participation

### Code Update After That Canary

That specific startup blocker is now patched in code:

1. true missing-side `EntryRepair` can now bypass the normal hard-skew reject path when the projected action restores both-side participation
2. settlement-shaper builder orders now bypass the generic maker recovery gate, so paired growth and directional-step no longer inherit the old `skip heavy-side BUY during recovery` / `skip light-side BUY stacking during recovery` rules
3. directional-step now uses a settlement-shaper core-build gate instead of repair-style target-pressure rejection, including a blocked-rebuild `inventory_vwap_sum` allowance when the projected book improves the current held book
4. late-phase near-target books now treat near-exact partial-fill one-lot surplus states as already reached, so a book like `49.99 / 45.00` should rest instead of trying to buy another full underdog lot just because the surplus is short by `0.01`
5. that late-phase directional-step gate is now patched in code: near-target blocked-rest states can tolerate a small additional `inventory_vwap_sum` drift when the projected step reaches the good coverage / target-skew envelope
6. healthy two-sided books with poor held `inventory_vwap_sum` now stay in `PairResting` instead of falling into `ShapeRepair -> inventory_quality_poor`; that keeps late books like `45.00 / 44.99` in the builder lane so paired growth, directional-step, or hold can decide the next move
7. a fresh live canary is still required to confirm that the latest dominant blocker has moved from `inventory_quality_poor -> ShapeRepair` to the next true runtime issue

---

## Executive Summary

`SETTLEMENT_SHAPER` is still canary-stable mechanically.

What the latest run proved:

1. routing into `SETTLEMENT_SHAPER` works
2. seed both sides works
3. startup asymmetry ownership works
4. exact repair sizing is preserved
5. rollover near expiry works like `MAKER_SKEW_ARB`

What is still wrong:

1. startup asymmetry is no longer the dominant blocker; the bot can now recover into a real two-sided base and keep building for several rungs
2. the newest dominant blocker was a late healthy book around `45.00 / 44.99` being misclassified as `ShapeRepair -> inventory_quality_poor`
3. once it entered that lane, the repair planner looped on `hard_skew_breach` and `shape_worsens` instead of staying in the normal settlement-shaper builder
4. the final live book can still collapse back to a balanced hold instead of the Sprint 3 fingerprint:
   - more dollars on the favorite
   - more shares on the underdog
   - high coverage
   - mild skew
   - held to settlement

The current code behaves more like:

1. seed both sides
2. recover any startup asymmetry through `EntryRepair`
3. build a real two-sided base through paired growth and bounded repair
4. reach a late balanced or near-balanced book such as `45.00 / 44.99`
5. risk dropping that late book into `ShapeRepair -> inventory_quality_poor` instead of keeping it in `PairResting`
6. stall on repair-style `hard_skew_breach` / `shape_worsens` holds until rollover

That is still useful progress.
It is not yet the intended settlement-shaping controller.

---

## Latest Live Canary

Market:

1. `btc-updown-5m-1773186600`
2. start `2026-03-11 06:49 WIB`
3. stop reason `ROLLOVER`

Final live book before rollover:

1. `qYES=14.98`
2. `qNO=0.00`
3. `total_cost=7.34`
4. `pair_coverage=0.000`
5. `skew=inf`
6. `inventory_vwap_sum=inf`

Final owner state near rollover:

1. `owner=EntryRepair`
2. `owner_reason=startup_asymmetry`
3. repeated holds with:
   - `hard_skew_breach`
   - occasional `spread_too_wide`

That final state is a one-sided stranded book.
It is not the Sprint 3 target shape.

---

## What Worked In The Latest Run

### 1. Seed submission still behaved correctly

Observed sequence:

1. `SeedBothSides` submitted a paired maker entry with:
   - `clip=15.00`
   - `y_bid=0.490`
   - `n_bid=0.480`
2. the YES seed leg filled in three chunks
3. the NO seed leg did not become inventory

Interpretation:

1. paired seed submission is still live
2. exact clip sizing is still present
3. the one-leg seed case remains the most important live startup failure mode

### 2. Ownership transfer into `EntryRepair` still behaved correctly

Observed sequence:

1. once the book was `qYES~=15`, `qNO=0`, owner switched:
   - `PairResting -> EntryRepair`
2. owner reason was:
   - `startup_asymmetry`
3. the hold logs consistently named:
   - `missing_side=NO`

Interpretation:

1. the owner split is still correct
2. the controller knows this is a true missing-side recovery case
3. the failure is not in ownership detection

### 3. Foreground rollover is still correct

Observed sequence:

1. near expiry the log shows:
   - `Expiring in 15s -> stopping for rollover.`
2. run exits with:
   - `reason=ROLLOVER`

Interpretation:

1. settlement shaper now stops in the same near-expiry window as the old maker flow
2. that part is still working as intended

---

## Current Dominant Failure Modes

### Failure Mode 1: missing-side `EntryRepair` is still blocked by `hard_skew_breach`

Observed evidence during the latest run:

1. after ownership moved to `EntryRepair`, every repair tick logged:
   - `missing_side=NO`
   - `trigger=startup_asymmetry`
   - `reason=hard_skew_breach`
2. there was no real `SETTLEMENT_SHAPER_ENTRY_REPAIR` submit after the one-leg seed outcome
3. this persisted through:
   - `SeedBothSides`
   - `EarlyBuild`
   - `MainAccumulation`
   - `FinishShape`
   - `FreezeRepairOnly`

Interpretation:

1. the controller correctly recognizes a true missing-side startup recovery state
2. the actual repair admission logic is still applying the normal hard-skew gate
3. that gate should not own a true missing-side recovery for the entire market

Practical effect:

1. the missing side is never restored
2. the bot remains one-sided until rollover

### Failure Mode 2: the bot can stay one-sided through every phase

Observed evidence:

1. at `06:50:04` the live book was already:
   - `qYES=14.98`
   - `qNO=0.00`
2. the same stranded state was still present after:
   - the `EarlyBuild` phase transition at `06:50:30`
   - the `MainAccumulation` phase transition at `06:51:00`
   - `FinishShape`
   - `FreezeRepairOnly`

Interpretation:

1. phase progression is working
2. but phase progression alone does not help if the core missing-side repair is gated off
3. the mode can therefore spend an entire market in a stranded startup state

Practical effect:

1. the bot never reaches a real two-sided book
2. no later Sprint 3 shaping behavior can even begin

### Failure Mode 3: favorite/underdog changes do not unblock repair

Observed evidence:

1. favorite side flipped several times during the run:
   - `YES -> NO`
   - `NO -> YES`
   - `YES -> NO`
2. even when projected missing-side repair quality improved materially:
   - projected `inventory_vwap_sum` dropped as low as about `0.800`
3. the planner still rejected the repair on the same `hard_skew_breach` reason

Interpretation:

1. this is not just a stale favorite-role issue
2. the repair gate itself is too strict for a true missing-side recovery

---

## Detailed Runtime Interpretation

### Seed And One-Leg Fill

This part is behaving partly correctly.

The run started with a one-leg seed fill, then:

1. `SeedBothSides` submitted a `15 / 15` paired entry
2. the YES leg filled almost completely
3. the NO leg never established inventory
4. owner switched to `EntryRepair`

This is still strong evidence that the startup controller split is correct:

1. startup asymmetry is not being blurred into generic `ShapeRepair`
2. the missing side is being identified explicitly
3. the failure starts after that point

### EntryRepair Admission Logic

This is the real blocker in the latest canary.

Once `EntryRepair` owned the state, the controller repeatedly evaluated the missing-side NO buy and then refused it with:

1. `reason=hard_skew_breach`
2. `missing_side=NO`
3. `trigger=startup_asymmetry`

That means the bug is no longer in:

1. seed submission
2. owner routing
3. basic missing-side detection

It is now inside the missing-side repair admission logic itself.

### Why This Matters

Sprint 3 cannot behave like the wallet if the mode can spend a full market in a one-sided startup state.

Before any of the normal shaping goals matter, the bot must be able to restore:

1. two-sided participation
2. non-zero `pair_coverage`
3. a live base book that later phases can shape

The latest run shows that this still is not guaranteed.

---

## Current Behaviour Conclusion

`SETTLEMENT_SHAPER` is still improving, but the current live bottleneck has moved again.

The latest dominant blocker is now:

1. one-leg seed fill
2. correct transfer to `EntryRepair`
3. missing-side repair rejected as `hard_skew_breach`
4. one-sided hold until rollover

So the next controller fix should be:

1. let true missing-side startup recovery bypass or relax the normal hard-skew gate
2. preserve maker-first behavior
3. restore two-sided participation before directional shape logic is allowed to dominate

That is the most important live blocker now.
