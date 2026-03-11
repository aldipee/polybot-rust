# HIGH-LEVEL BEHAVIOUR

## Version 0.1.28

Audience: non-technical operator, reviewer, or stakeholder  
Scope: Sprint 4 only  
Mode: `EXEC_MODE=WALLET_CLONE`  
Date: 2026-03-11  
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

Current status: `PARTIAL`

That means:

1. the mode is live
2. the startup path now works
3. the bot can seed both sides
4. the bot can repair one-sided startup fills
5. the bot can taper and roll over correctly
6. but the normal `PairBuild` phase still churns too much and still overpays too often

So the mode is real and much stronger than before, but it is still a canary, not a finished wallet clone.

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

### Goal C: avoid late chaos

This means:

1. stop acting like a full-speed builder late in the market
2. reduce activity after taper starts
3. become nearly silent in the final quiet window

In the current canary, Goal A and Goal C are working much better than before.
Goal B is active, but still not economically clean enough.

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

Compared with older behaviour, the important improvement is:

1. startup asymmetry now has its own owner
2. the bot no longer gets stuck because startup repair is treated like ordinary shaping

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

1. YES around `0.491`
2. NO around `0.500`
3. clip `5`
4. pair sum about `0.991`

So the opener worked and got the strategy into the market.

## C. What Counts As A Bad Startup Outcome

A bad startup outcome is mainly this:

1. one side fills
2. the other side does not
3. the bot is left one-sided right after opening

In the latest run, that did happen briefly:

1. YES filled first
2. NO did not become inventory immediately
3. the bot was left at about `5 YES / 0 NO`

The important difference from older behaviour is:

1. the bot did not freeze in that state
2. it handed control to the startup repair owner

## D. SeedCompletion Logic

`SeedCompletion` is the owner that fixes one-sided startup fills.

Its job in plain language is:

1. detect which side is missing
2. place a maker buy on the missing side
3. ignore the older shape-style vetoes
4. restore both sides first
5. then hand control to the normal builder

In the reviewed run, this worked:

1. first fill was around `3.4s`
2. missing side was identified correctly
3. NO was restored by about `6.6s`
4. both startup targets were met

This is one of the main improvements in `0.1.28`.

## E. PairBuild Logic

`PairBuild` is the main normal-flow inventory builder.

Its job is to:

1. keep posting paired maker buys when conditions are usable
2. add more inventory through the middle of the market
3. switch to the lighter side when one side becomes too large
4. use clip ladders and phase budgets instead of one fixed order size

In the current canary, `PairBuild` is real and active.
It is no longer a missing stub.

But it still has two important live problems:

1. it cancels resting maker orders too aggressively
2. it still lets some expensive additions through

That is why the mode now looks mechanically alive but not yet economically clean.

## F. Why PairBuild Is Still The Main Problem

The reviewed run repeatedly showed:

1. `lighter_side_live_order_stale_cancel`
2. `asymmetric_submit_stale_cancel`
3. repeated re-submission of the lighter side
4. expensive fills while trying to rebalance

Plain-language meaning:

1. the bot is still giving up on resting maker orders too early
2. it then reposts again
3. that creates churn
4. churn makes it easier to overpay

This is why the run ended active but not clean.

## G. Taper Logic

`Taper` is the late-market slowdown owner.

Its job is to:

1. stop acting like a full-speed builder late
2. allow only small maintenance if needed
3. go mostly quiet in the final quiet window

In the reviewed run, this part behaved correctly.

Observed good signs:

1. the bot moved to `Taper` at the expected late point
2. it emitted `final_quiet_rest`
3. it placed no meaningful new activity after final quiet

## H. Rollover Logic

Near expiry, the bot now behaves correctly.

Current rollover behaviour:

1. it watches the stop buffer near market end
2. it stops foreground trading near expiry
3. it exits with rollover instead of hanging on the old market
4. it cancels outstanding orders and waits for resolution cleanly

This part is now one of the more reliable pieces of the mode.

---

## Step-By-Step Of The Latest Live Run

Below is the latest reviewed run in business terms.

### 1. Startup gating worked

The bot:

1. found the correct market
2. loaded the YES and NO assets
3. warmed trading metadata
4. waited for fresh enough data
5. waited for websocket readiness

So the run did not fail at discovery or readiness.

### 2. OpenBoth worked

Once live:

1. the bot entered `OpenBoth`
2. it posted a paired maker opener
3. the first fill arrived around `3.4s`

That means the mode is no longer failing at the first trade.

### 3. SeedCompletion worked

The startup outcome was briefly asymmetric:

1. YES filled first
2. NO lagged
3. the bot switched to `SeedCompletion`
4. the missing NO side was restored by about `6.6s`

This is the clearest improvement versus earlier wallet-clone canaries.

### 4. PairBuild was active but noisy

The bot then built inventory through the middle of the market.

Observed good signs:

1. it kept trading through the main window
2. it used both paired-growth and lighter-side-first actions
3. it accumulated meaningful size

Observed bad signs:

1. frequent stale cancels
2. repeated lighter-side reposts
3. some very expensive NO fills later in the run

So the builder was live, but still rough.

### 5. Late behaviour worked

After about `240s`:

1. the bot entered `Taper`
2. it reduced activity
3. it went quiet in the final quiet window
4. it rolled over correctly near expiry

So the late lifecycle is currently one of the stronger parts of the mode.

---

## Final State Of The Latest Run

The final reviewed state before rollover was approximately:

1. `qYES=70`
2. `qNO=65`
3. `total_cost=67.50`
4. `paired_size=65`
5. `unmatched_size=5`
6. `combined_avg_paid=1.019`

Plain-language meaning:

1. the bot built a real two-sided book
2. the final book was not catastrophically skewed
3. but the paired inventory was still slightly too expensive
4. and there was still a small unmatched tail

So this is no longer a dead or frozen bot.
It is an active bot that still needs cleaner `PairBuild` economics.

---

## What Is Good News In This Run

Even though the run still fell short of the objective, it tells us useful things.

Good news:

1. the mode started correctly
2. the mode opened correctly
3. the mode repaired startup asymmetry correctly
4. the mode stayed active through the market
5. the mode tapered correctly
6. the mode rolled over correctly

So the architecture is much stronger than before.
The remaining problem is more focused now.

---

## What Is The Main Problem In One Sentence

The bot can now build and repair inventory, but `PairBuild` still churns too much and still pays too much to be a close wallet-clone match.

---

## What A Non-Technical User Should Take Away

If you are not reading code, the practical takeaway is:

1. the bot now behaves like a real strategy, not a broken startup stub
2. it can enter the market
3. it can recover one-sided startup outcomes
4. it can stay active for most of the market
5. but it still does not build inventory cleanly enough

That is why the correct label today is still:

- strong canary
- not yet finished production behaviour

---

## What Should Happen Next

The next desired behaviour is simple:

1. let good resting maker orders live longer before canceling them
2. stop leaking sub-minimum exchange-invalid orders
3. make normal `PairBuild` adds more selective once paired cost quality becomes weak
4. keep startup completion and taper behaviour exactly as they are unless the next canary disproves them

If those things improve, the next canary has a realistic chance of looking much closer to the target wallet.

---

## Short Plain-English Conclusion

Today, wallet clone is operational and substantially improved.

It can:

1. pre-arm
2. open
3. repair startup asymmetry
4. build inventory
5. taper
6. roll over

But it still cannot reliably do the most important economic next step well enough:

1. keep `PairBuild` calm
2. avoid unnecessary cancel/repost churn
3. finish with cheaper, cleaner paired inventory

That is the current real-world behaviour in one page.
