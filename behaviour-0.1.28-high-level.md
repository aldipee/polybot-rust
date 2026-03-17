# HIGH-LEVEL BEHAVIOUR

## Version 0.1.28

Audience: non-technical operator, reviewer, or stakeholder
Scope: Sprint 4 only
Mode: `EXEC_MODE=WALLET_CLONE`
Date: 2026-03-16
Reference: based on the latest technical runtime note in `behaviour-0.1.28.md`

This document explains the current wallet-clone behaviour in plain language.
It is meant to describe what the bot is actually doing today, not what we eventually want it to do.

---

## Purpose

Wallet clone is meant to do five things:

1. get into both sides of a market quickly
2. stay active across the market as a maker-first inventory builder
3. repair missing-side situations without freezing
4. slow down late instead of forcing ugly last-minute trading
5. roll over cleanly near expiry

The important business idea is:

- this mode is supposed to behave like an aggressive two-sided accumulator
- it is not supposed to behave like the older settlement-shaper controller

---

## Current Status

Current status: `MIXED_CANARY_RESULTS`

That means:

1. the mode is live
2. the startup path works
3. the bot can seed both sides
4. the bot can repair one-sided startup fills
5. the mid-market builder no longer freezes - it accumulates through the whole window
6. lighter-side repairs are now capped to prevent overpaying after market moves
7. profitable books have now been seen in canaries 1 and 2
8. later completed canaries still lost for two different reasons:
   - directional tail
   - expensive paired core
9. the mode is still a canary, not a confirmed production strategy

So the mode is real and materially improved, but it is not yet consistently profitable.

---

## What The Bot Is Trying To Optimize

At a high level, the bot is trying to do three things at once.

### Goal A: get both sides live quickly

This means:

1. the bot should not miss the opening
2. it should open both sides early
3. if only one side fills, it should restore the missing side quickly

### Goal B: keep building paired inventory

This means:

1. keep participating across the market
2. keep posting maker buys on both sides when conditions are usable
3. lean toward the lighter side when one side grows too large
4. cap lighter-side repair bids to avoid chasing bad prices
5. average down the book when the current pair_sum is cheaper than the existing book

### Goal C: avoid late chaos

This means:

1. stop acting like a full-speed builder late in the market
2. reduce activity after taper starts
3. become nearly silent in the final quiet window

Across the reviewed canary set, Goal A and Goal C are mechanically working. Goal B is substantially improved because the bot no longer freezes mid-market, but it still fails economically in some market regimes.

---

## Current Runtime Lifecycle

The mode currently moves through these broad stages.

### 1. PreArm

Before open, the bot:

1. loads the market
2. loads the YES and NO assets
3. warms trading metadata
4. waits for market and user websocket readiness
5. waits for usable quote inputs

If data is stale or connections are missing, it does not trade yet.

### 2. Time Phases

Once the market is live, the bot moves through these broad windows:

1. `OpenBoth`
   - startup opener
   - current canary profile uses about 10% to 15% of usable budget here
2. `PairBuild`
   - the main normal-flow builder
   - includes early build, main build, and late build behavior
   - now includes lighter-side repair bid cap and averaging-down logic
3. `Taper`
   - begins around `240s`
   - late activity becomes maintenance-only
4. `HoldSettleRollover`
   - near-expiry stop and rollover ownership

These phases do not just label time.
They decide how aggressive the bot is allowed to be and what kind of action it is allowed to take.

### 3. Control Owners

Within those phases, the bot gives control to one main owner at a time.

The important owners today are:

1. `PreArm`
   - startup readiness state
2. `OpenBoth`
   - paired opener
3. `SeedCompletion`
   - missing-side startup repair owner
4. `PairBuild`
   - normal inventory-building owner
5. `Taper`
   - late maintenance owner
6. `HoldSettleRollover`
   - stop / rollover owner

Compared with older behaviour, the important improvements are:

1. startup asymmetry now has its own owner
2. the bot no longer gets stuck because startup repair is treated like ordinary shaping
3. `PairBuild` no longer freezes when the book is expensive — it can average down
4. lighter-side repairs no longer chase the market — they are capped to the original pair economics

---

## Core Logic The Bot Uses Today

## A. PreArm Logic

Before open, the bot does not blindly start trading.

It waits for:

1. market websocket connection
2. user websocket connection
3. asset IDs to be ready
4. quote inputs to be usable
5. market data to be fresh enough

In the reviewed run, this part behaved correctly.
The bot held when feeds were not ready and then became active once the inputs were healthy.

## B. Opening Logic

When the market opens, the bot tries to seed both sides.

Current opening rules include:

1. use maker-first paired buys
2. use small startup clips
3. avoid favorite / underdog shaping logic
4. require usable quote inputs
5. stay within the startup budget slice

In the reviewed run, the opening pair was roughly:

1. YES around `0.470`
2. NO around `0.510`
3. clip `5`
4. pair sum about `0.980`

So the opener worked and got the strategy into the market.

## C. What Counts As A Bad Startup Outcome

A bad startup outcome is mainly this:

1. one side fills
2. the other side does not
3. the bot is left one-sided right after opening

In the latest run, that did happen:

1. NO filled first at `0.510`
2. YES did not fill immediately
3. `SeedCompletion` posted YES, first at `0.480`, then `0.520`, finally filling at `0.580`

The important difference from older behaviour is:

1. the bot did not freeze in that state
2. it handed control to the startup repair owner
3. the expensive startup fill was recovered through mid-market volume

## D. SeedCompletion Logic

`SeedCompletion` is the owner that fixes one-sided startup fills.

Its job in plain language is:

1. detect which side is missing
2. place a maker buy on the missing side
3. ignore the older shape-style vetoes
4. restore both sides first
5. then hand control to the normal builder

In the reviewed run, this worked:

1. missing side was identified correctly
2. YES was restored by about `22.5s`
3. both startup targets were met

Note: `SeedCompletion` bypasses cost guards by design. In this run it paid `0.580` for YES, which is expensive. The strategy recovered this cost through subsequent mid-market accumulation at cheaper pair_sums.

## E. PairBuild Logic

`PairBuild` is the main normal-flow inventory builder.

Its job is to:

1. keep posting paired maker buys when conditions are usable
2. add more inventory through the middle of the market
3. switch to the lighter side when one side becomes too large
4. cap lighter-side repair bids to preserve pair economics
5. average down the book when pair_sum is cheaper than the current blended cost
6. use clip ladders and phase budgets instead of one fixed order size

In the reviewed canary set, `PairBuild` is mechanically live but not yet economically safe.

The two new fixes addressed the previous problems:

1. **Lighter-side repair bid cap**: when paired growth fills one side and the other needs repair, the repair bid is capped at `original_pair_sum - filled_side_price`. This prevents the bot from chasing the current market price after a move. The log shows `bid_cap_applied=true bid=0.420 original_bid=0.590` — a saving of `0.17` per share on that repair.

2. **Averaging-down exception**: when the current book is expensive (RepairOnly or Freeze band) but the current pair_sum is cheaper than the existing inventory_vwap_sum, the bot is now allowed to add a small clip to improve the blended cost. This eliminated the mid-market freeze entirely in the profitable canaries.

But the later completed canaries exposed two new problems:

1. **Directional tail**: in canary 5, a late lighter-side YES repair filled `20` shares even though the exact live gap was only about `10`. That turned a nearly balanced book into a `10.55` share YES tail.
2. **Expensive core**: in canary 6, the bot finished almost perfectly balanced but still lost because the combined paired cost finished at `1.026`.

## F. Why PairBuild Is Better But Still Risky

The reviewed canary set showed:

1. the bot now accumulates actively through the middle of the market
2. the earlier freeze problem is gone
3. profitable books are possible
4. but the bot can still lose in two ways:
   - end skewed with a tail
   - end balanced but too expensive

Plain-language meaning:

1. the bot now accumulates actively through the middle of the market
2. the freeze that prevented mid-market building is gone
3. the builder is no longer broken
4. the remaining problem is economic judgement, not lack of activity

## G. Taper Logic

`Taper` is the late-market slowdown owner.

Its job is to:

1. stop acting like a full-speed builder late
2. allow only small maintenance if needed
3. go mostly quiet in the final quiet window

In the reviewed completed runs, this part behaved mechanically correctly.

Observed good signs:

1. the bot moved to `Taper` at the expected late point
2. quiet late behaviour still exists in profitable runs
3. the bot rolled over correctly near expiry

But taper is not yet solving late risk by itself:

1. in canary 5, taper started with `budget_too_small`
2. the lifecycle handoff worked, but there was no budget left to clean up the tail

## H. Rollover Logic

Near expiry, the bot now behaves correctly.

Current rollover behaviour:

1. it watches the stop buffer near market end
2. it stops foreground trading near expiry
3. it exits with rollover instead of hanging on the old market
4. it cancels outstanding orders and waits for resolution cleanly

This part is now one of the more reliable pieces of the mode.

---

## Final State Of The Latest Completed Runs

The most important completed runs to read together are:

1. profitable reference run (`btc-updown-5m-1773596100`)
   - `paired_size=64.98`
   - `tail_at_expiry=0.01`
   - `combined_avg_paid=0.982`
   - `worst_case_settlement_floor=+1.19`
2. directional-tail loss (`btc-updown-5m-1773598200`)
   - `paired_size=65.00`
   - `tail_at_expiry=10.55`
   - `combined_avg_paid=0.960`
   - `worst_case_settlement_floor=-0.92`
3. expensive-core loss (`btc-updown-5m-1773598500`)
   - `paired_size=59.99`
   - `tail_at_expiry=0.01`
   - `combined_avg_paid=1.026`
   - `worst_case_settlement_floor=-1.55`

Plain-language meaning:

1. the bot can build a large profitable two-sided book
2. the bot can also lose even when the paired core is cheap if it ends with the wrong tail
3. the bot can also lose even when it finishes balanced if the whole paired book was bought too expensively

---

## What Is Good News In The Reviewed Set

Good news:

1. the mode started correctly
2. the mode opened correctly
3. the mode repaired startup asymmetry correctly
4. the mid-market builder stayed active instead of freezing
5. the lockout failure from earlier canaries is materially improved
6. the mode rolled over correctly
7. profitable runs now exist, so the edge is not purely hypothetical

This is still a substantial improvement over the earlier lockout state.

---

## What Is The Main Problem In One Sentence

The bot is now active and mechanically correct, but it can still lose either by ending skewed or by building a balanced book above `1.00` total cost.

---

## What A Non-Technical User Should Take Away

If you are not reading code, the practical takeaway is:

1. the bot now behaves like a real strategy, not a broken stub
2. it can enter the market
3. it can recover one-sided startup outcomes
4. it can accumulate through the middle of the market without freezing
5. it has shown real profits in some runs
6. it still loses in other runs for understandable reasons
7. more fixes and more canary data are still needed

That is why the correct label today is:

- mixed canary results
- mechanically live, economically not yet stable

---

## What Should Happen Next

The next desired behaviour is simple:

1. run 3-5 more canaries across different market conditions
2. clamp lighter-side repairs so they do not overshoot the exact live gap into a new tail
3. add a stronger stop for paired growth when projected paired cost stays above `1.00`
4. confirm that `worst_case_settlement_floor` stays near zero or positive more consistently
5. watch for runs where the bid cap creates persistent unfilled repairs and the bot ends heavily skewed
6. keep startup completion, final quiet, and rollover behaviour exactly as they are unless a canary disproves them

If those canaries confirm consistent profitability, the mode can be considered for production readiness.

---

## Short Plain-English Conclusion

Today, wallet clone is operational, active, and capable of profit, but it is not yet consistently safe.

It can:

1. pre-arm
2. open
3. repair startup asymmetry
4. build inventory through the middle of the market
5. cap lighter-side repairs to avoid chasing
6. average down an expensive book
7. taper
8. roll over
9. sometimes finish profitably, but sometimes lose through tail or expensive-core behaviour

The next step is tightening those two failure modes and then confirming the result across more market conditions.
