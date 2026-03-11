# HIGH-LEVEL BEHAVIOUR

## Version 0.1.27

Audience: non-technical operator, reviewer, or stakeholder  
Scope: Sprint 3 only  
Mode: `EXEC_MODE=SETTLEMENT_SHAPER`  
Date: 2026-03-11
Reference: based on the latest technical runtime note in `behaviour-0.1.27.md`

This document explains the current settlement-shaper behaviour in plain language.
It is meant to describe what the bot is actually doing today, not what we eventually want it to do.

---

## Purpose

Settlement shaper is meant to do five things:

1. enter both sides of a market
2. keep the wallet safe enough while the market is live
3. gradually shape the wallet toward the preferred final pattern
4. stay in control as the market moves through its time phases
5. roll over cleanly when the market is ending

The important business idea is:

- this mode is supposed to manage a two-sided wallet
- it is not supposed to get stranded on only one side

---

## Current Status

Current status: `PARTIAL`

That means:

1. the mode is live
2. the basic startup flow works
3. the bot can place opening orders
4. the bot can recognize a bad opening outcome
5. the bot can roll over correctly near expiry
6. but it still does not reliably recover from every one-sided opening fill

So the mode is real, but it is still a canary, not a finished Sprint 3 strategy.

---

## What The Bot Is Trying To Optimize

At a high level, the bot is trying to balance two goals at the same time.

### Goal A: keep the book healthy

This means:

1. both sides should be present
2. one side should not become wildly larger than the other
3. the average held price should not become too expensive
4. the book should remain manageable all the way through the market

### Goal B: shape the wallet in a preferred direction

This means:

1. spend a bit more money on the favorite side
2. end with a bit more shares on the underdog side
3. keep that bias mild, not extreme

In practice, Goal A currently dominates Goal B.
That is the right safety instinct, but today it is still too strong in some bad startup cases.

---

## Current Runtime Lifecycle

The mode currently moves through these broad stages.

### 1. Discovery

At startup, the bot:

1. loads the market
2. loads asset metadata
3. waits for market data to become fresh
4. waits for trading warmup to finish
5. identifies which side currently looks like the favorite and which looks like the underdog

If market data is stale, it will not trade yet.

### 2. Time Phases

Once discovery is complete, the market is divided into five time windows:

1. `SeedBothSides`
   - first 30 seconds
   - budget slice: about 10% to 15%
2. `EarlyBuild`
   - 30 to 60 seconds
   - budget slice: about 15% to 20%
3. `MainAccumulation`
   - 60 to 180 seconds
   - budget slice: about 45% to 55%
4. `FinishShape`
   - 180 to 240 seconds
   - budget slice: about 15% to 20%
5. `FreezeRepairOnly`
   - 240 seconds onward
   - budget slice: about 5% to 10%

These phases do not just label time.
They decide how aggressive the bot is allowed to be and what kind of action it is allowed to take.

### 3. Control Owners

Within those phases, the bot gives control to one main owner at a time.

The important owners today are:

1. `DiscoveryArm`
   - startup waiting state
2. `EntryRepair`
   - handles missing-side situations
   - for example: one opening leg filled and the other did not
3. `ShapeRepair`
   - handles unhealthy two-sided books
   - for example: weak coverage, excessive imbalance, or expensive held inventory
4. `PairResting`
   - normal building state once the wallet is healthy enough
5. `SettlementRedeem`
   - resolution / settlement ownership later in the lifecycle

In the current canary, not every owner is fully active all the time.
The logs explicitly say this is a limited canary with maker-side repair and builder slices enabled.

---

## Core Logic The Bot Uses Today

## A. Favorite / Underdog Identification

The bot continuously decides which side is the favorite and which side is the underdog.

It does not instantly flip on one noisy price tick.
Instead it uses a small confirmation rule:

1. minimum price difference: about `0.01`
2. confirmation updates required: `3`

So the role only switches after several confirming observations.
This is meant to reduce flip-flopping.

## B. Opening Logic

When it enters `SeedBothSides`, the bot tries to open both sides at once.

Current opening rules include:

1. both sides must have usable quotes
2. the combined bid price must be below `1.00`
3. the opening must fit inside the current phase budget
4. the opening size is chosen from the current lot / clip rules
5. the opening is maker-first

In the latest live run, the opening pair was:

1. YES bid `0.490`
2. NO bid `0.480`
3. pair sum `0.970`
4. clip `15`

So the bot did attempt a real paired opening.

## C. What Counts As A Bad Opening Outcome

A bad opening outcome is mainly this:

1. one side fills
2. the other side does not
3. the bot is left with inventory on only one side

In the current run, that is exactly what happened:

1. YES filled almost completely
2. NO did not become inventory
3. the bot ended up around `14.98 YES / 0 NO`

At that point, the bot correctly recognized the problem.

## D. EntryRepair Logic

`EntryRepair` is the controller that is supposed to fix a missing side.

Its job is simple in plain language:

1. detect which side is missing
2. place a maker buy on the missing side
3. restore two-sided participation
4. only then hand the wallet back to the normal builder

The current logic still uses several gates before it will actually place that repair:

1. budget must still be available
2. both assets must still be valid
3. quotes must still be usable
4. the missing-side bid must be positive
5. the repair must be large enough to satisfy the lot rules
6. the projected repaired book must pass the internal safety rules

The current live problem is in that last step.

## E. The Safety Rules That Currently Block Recovery

The bot simulates what the wallet would look like after the repair before sending the order.

Then it asks questions like:

1. would the wallet still be too imbalanced?
2. would the repair make the shape worse?
3. would the held inventory become too expensive?
4. is the market too wide right now?

In the latest live run, the recovery kept failing on this one reason:

- `hard_skew_breach`

Plain-language meaning:

- the bot believed the temporary repaired wallet would still be too uneven
- so it refused to place the missing-side repair order

This is the key current defect.
The bot is being too strict during a true missing-side recovery.

## F. ShapeRepair Logic

`ShapeRepair` is meant for a different problem.
It is supposed to work only after both sides already exist.

Its job is to improve a two-sided wallet that is not healthy enough.
Examples:

1. one side has become too large
2. coverage is too weak
3. the held book is too expensive
4. the wallet has drifted too far from the desired shape

This is not the main failure in the latest live run.
The bot never got far enough to use two-sided shaping properly because it never repaired the missing side.

## G. PairResting Logic

`PairResting` is the normal builder state.
That is where the bot should spend most of its healthy market time.

Its job is to:

1. keep the book alive on both sides
2. add more paired inventory when conditions are good
3. size up carefully within the phase budget
4. keep the wallet inside its health limits

Again, this was not the latest blocker.
The run never returned to a healthy two-sided state, so the normal builder never really got a chance.

## H. Rollover Logic

Near expiry, the bot now behaves correctly.

Current rollover behaviour:

1. it watches the stop buffer near market end
2. it stops foreground trading when the market is close to expiry
3. it exits with rollover instead of hanging on the expired market
4. it prepares for the next market correctly

This part is now one of the more reliable pieces of the mode.

---

## Step-By-Step Of The Latest Live Run

Below is the latest run in business terms.

### 1. Startup worked

The bot:

1. found the correct market
2. loaded the YES and NO assets
3. warmed trading metadata
4. connected market data
5. entered settlement-shaper mode correctly

### 2. Discovery completed

After data freshness and warmup:

1. the bot identified a favorite side
2. it moved from `DiscoveryArm` into `SeedBothSides`
3. it started the opening process

### 3. SeedBothSides submitted correctly

The bot placed a paired opening order:

1. YES at `0.490`
2. NO at `0.480`
3. size `15`

So the opening logic itself was not broken.

### 4. Only one side filled

What happened next:

1. the YES side filled in several chunks
2. the NO side did not become live inventory
3. the bot was left with almost `15 YES` and `0 NO`

This is the exact kind of startup asymmetry `EntryRepair` is supposed to solve.

### 5. Ownership moved correctly to EntryRepair

The bot then did the right ownership move:

1. it left `PairResting`
2. it moved to `EntryRepair`
3. it tagged the reason as `startup_asymmetry`
4. it correctly identified the missing side as `NO`

So the diagnosis was right.

### 6. The actual repair never happened

From that point onward, every repair check repeated the same pattern:

1. the bot evaluated a missing-side NO repair
2. it judged that repair as violating the hard skew rule
3. it refused to place the repair
4. it tried again on the next cycle
5. it refused again

This happened again and again through:

1. `SeedBothSides`
2. `EarlyBuild`
3. `MainAccumulation`
4. `FinishShape`
5. `FreezeRepairOnly`

So the bot spent almost the whole market recognizing the problem but not acting on it.

### 7. The market rolled over correctly

At the end:

1. the bot hit the expiry buffer
2. it stopped trading correctly
3. it exited with `ROLLOVER`

So the end-of-market handling worked.

---

## Final State Of The Latest Run

The final live state before rollover was effectively:

1. only YES inventory
2. no NO inventory
3. no real pair coverage
4. no useful shaping yet

That means the mode did not reach the real Sprint 3 behavior.
It never reached a healthy two-sided wallet, so it never got to do meaningful shaping.

---

## What Is Good News In This Run

Even though the run failed in an important way, it still tells us useful things.

Good news:

1. the mode did not fail at startup
2. the mode did not fail to place the initial orders
3. the mode did not lose track of the problem
4. the mode kept the correct owner on the problem
5. the mode rolled over correctly at the end

So the architecture is stronger than before.
The remaining issue is more focused now.

---

## What Is The Main Problem In One Sentence

The bot now understands when it has a one-sided startup problem, but it is still too strict about allowing the repair that would fix it.

---

## What A Non-Technical User Should Take Away

If you are not reading code, the practical takeaway is:

1. the bot can start correctly
2. the bot can place the opening trade correctly
3. the bot can identify a failed one-sided opening
4. but it can still freeze in that state instead of repairing it
5. so the strategy is not yet reliably doing its full job

That is why the correct label today is still:

- promising canary
- not finished production behaviour

---

## What Should Happen Next

The next desired behaviour is simple:

1. when only one side of the opening fills
2. the bot should still be willing to buy the missing side
3. even if the wallet looks temporarily uneven during that repair
4. because restoring both sides is more important than rejecting that repair for being temporarily imperfect

Once that works, the bot has a much better chance of reaching the real Sprint 3 shaping behaviour afterward.

---

## Short Plain-English Conclusion

Today, settlement shaper is operational but still not self-recovering enough.

It can:

1. start
2. open
3. detect a bad opening
4. keep control
5. roll over

But it still cannot reliably do the most important next step:

1. fix the missing side
2. restore a healthy two-sided wallet
3. continue shaping from there

That is the current real-world behaviour in one page.
