# HIGH-LEVEL BEHAVIOUR

## Version 0.1.28

Audience: non-technical operator, reviewer, or stakeholder  
Scope: Sprint 4 only  
Mode: `EXEC_MODE=WALLET_CLONE`  
Date: 2026-03-15  
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
5. the latest reviewed run finished balanced and with no expiry tail
6. the current tree now blocks duplicate `OpenBoth` resubmits and taper `suppress` fall-through
7. but the mode can still overpay early, freeze through the middle of the market, and end balanced but unprofitable

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
Goal B is active, but it can still get into an expensive early basis and then freeze instead of averaging down.

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

1. YES around `0.520`
2. NO around `0.470`
3. clip `5`
4. pair sum about `0.990`

So the opener worked and got the strategy into the market.

## C. What Counts As A Bad Startup Outcome

A bad startup outcome is mainly this:

1. one side fills
2. the other side does not
3. the bot is left one-sided right after opening

In the latest run, that did happen briefly:

1. YES filled first
2. NO did not become inventory immediately
3. the reviewed run briefly deepened to about `10 YES / 0 NO` before missing-side restoration

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

1. missing side was identified correctly
2. NO was restored by about `9.7s`
3. both startup targets were met

This is one of the main improvements in `0.1.28`.

## E. PairBuild Logic

`PairBuild` is the main normal-flow inventory builder.

Its job is to:

1. keep posting paired maker buys when conditions are usable
2. add more inventory through the middle of the market
3. switch to the lighter side when one side becomes too large
4. use clip ladders and phase budgets instead of one fixed order size

In the latest reviewed canary, `PairBuild` was real and structurally active.
It is no longer a missing stub.

But it still has three important live problems:

1. startup can leave the paired core too expensive very early
2. once paired cost is already bad, `PairBuild` spends most of the market in freeze instead of averaging down
3. that means the bot can finish balanced and still be locked at a loss

That is why the mode now looks mechanically alive but still not economically safe enough.

## F. Why PairBuild Is Still The Main Problem

The latest reviewed run repeatedly showed:

1. the book first became balanced at about `10 YES / 10 NO / total_cost=10.80`
2. middle-market fill shares were effectively zero from `60s` to `240s`
3. paired-cost occupancy was almost entirely `freeze`
4. the final state was balanced at `20 YES / 20 NO`, but only at `total_cost=21.15`

Plain-language meaning:

1. the bot can pay too much very early
2. after that, the strategy becomes too conservative instead of averaging down
3. the book can stay balanced but still cost more than it can ever settle for
4. so the bot now loses more from bad basis and freezing than from expiry tails

This is why the run ended operational but economically weak.

## G. Taper Logic

`Taper` is the late-market slowdown owner.

Its job is to:

1. stop acting like a full-speed builder late
2. allow only small maintenance if needed
3. go mostly quiet in the final quiet window

In the latest reviewed run, this part mostly behaved correctly.

Observed good signs:

1. the bot moved to `Taper` at the expected late point
2. the final state still had `tail_at_expiry=0`
3. it placed no activity after final quiet
4. it rolled over correctly near expiry

Observed caveat:

1. the reviewed run exposed a bug where taper could log `suppress` and still submit anyway
2. the current tree now blocks that fall-through path

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

### 2. OpenBoth got into market, but startup was still expensive

Once live:

1. the bot entered `OpenBoth`
2. it posted a paired maker opener immediately after open
3. the opening pair was roughly:
   - `YES bid 0.520`
   - `NO bid 0.470`
   - `pair sum 0.990`
   - `clip 5`
4. YES filled first
5. in the reviewed run, YES effectively filled again before NO was restored, and the book briefly reached about `10 YES / 0 NO`

That means the mode is no longer failing at the first trade, but startup can still get too expensive too quickly.

### 3. SeedCompletion worked

The startup outcome was briefly asymmetric:

1. YES filled first
2. NO lagged
3. the bot switched to `SeedCompletion`
4. the missing NO side was restored
5. both sides were live by about `9.7s`

This is the clearest improvement versus earlier wallet-clone canaries.

### 4. PairBuild was active, but mostly frozen

The bot then built inventory through the middle of the market.

Observed good signs:

1. it kept trading through the main window
2. it did restore a balanced `10 YES / 10 NO` paired core
3. it stayed mechanically stable instead of collapsing into a tail through the middle

Observed bad signs:

1. the book first reached about `10 YES / 10 NO / total_cost=10.80`
2. from `60s` to `240s`, it recorded no fill shares at all
3. paired-cost occupancy was almost entirely `freeze`
4. so the strategy did not average down through the middle of the market
5. the book stayed balanced, but not cheap

So the builder was live, but it still creates bad economics by freezing on an already-expensive book.

### 5. Late behaviour mostly worked, and no final tail remained

After about `240s`:

1. the bot entered `Taper`
2. it still submitted some late maintenance fills
3. it went quiet in the final quiet window
4. it rolled over correctly near expiry
5. the final state still ended with no unmatched expiry tail
6. the reviewed run also exposed a taper bug where `suppress` could still fall through to submit
7. the current tree now blocks that path

So the late lifecycle is still one of the stronger parts of the mode, and it is cleaner than the previous tail-heavy canary.

---

## Final State Of The Latest Run

The final reviewed state before rollover was approximately:

1. `qYES=20`
2. `qNO=20`
3. `total_cost=21.15`
4. `paired_size=20`
5. `unmatched_size=0`
6. `pair_coverage=1.000`
7. `share_skew=1.000`
8. `combined_avg_paid=1.058`
9. `worst_case_settlement_floor=-1.15`

Plain-language meaning:

1. the bot built a real two-sided book
2. it finished balanced
3. it did not finish with an expiry tail
4. but the final paired inventory was still much too expensive

So this is no longer a tail-heavy canary.
It is now an active bot that can finish balanced and still lose money because the book is too expensive.

---

## What Is Good News In This Run

Even though the run still fell short of the objective, it tells us useful things.

Good news:

1. the mode started correctly
2. the mode opened correctly
3. the mode repaired startup asymmetry correctly
4. the mode rolled over correctly
5. the latest reviewed run finished with no expiry tail
6. the current tree now blocks two canary-discovered flow bugs

So the architecture is much stronger than before.
The remaining problem is more focused now.

---

## What Is The Main Problem In One Sentence

The bot can now build and repair inventory, but it still overpays early and then freezes instead of averaging down, so it can finish balanced and still lose money.

---

## What A Non-Technical User Should Take Away

If you are not reading code, the practical takeaway is:

1. the bot now behaves like a real strategy, not a broken startup stub
2. it can enter the market
3. it can recover one-sided startup outcomes
4. it can finish without an expiry tail
5. but it still does not build inventory cheaply enough to be profitable

That is why the correct label today is still:

- strong canary
- not yet finished production behaviour

---

## What Should Happen Next

The next desired behaviour is simple:

1. confirm in the next canary that the latest `OpenBoth` and taper suppress fixes behave as intended
2. reduce early overpay after asymmetric startup fills
3. change `PairBuild` cost gating so the middle of the market can average down instead of freezing
4. keep startup completion, final quiet, and rollover behaviour exactly as they are unless the next canary disproves them

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
2. keep the paired core below settlement value
3. finish with cheaper, cleaner paired inventory after early asymmetry

That is the current real-world behaviour in one page.
