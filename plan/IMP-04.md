# IMP-04 Plan: Exact Open-Time Seeding

## Summary
Implement `REQ-004`, `REQ-005`, and `REQ-006` inside the current `PreArm` and `OpenBoth` runtime, without changing the broader phase model yet. `PreArm` stays the pre-open warmup state, and `OpenBoth` stays the startup seeding owner, but the handler becomes explicitly deadline-aware.

The startup clock will use two runtime-observed anchors:
- `open_confirmed_ts`: the first runtime cycle where `t_into_s >= 0`
- `first_tradable_post_open_ts`: the first post-open moment where both sides have fresh cached quotes, both quote timestamps are `>= open_confirmed_ts`, and startup pair-quote validation passes

The seeding KPI anchor is the earlier nonzero of those two timestamps. The bot must try to get both first seed submits done within `5.0s` of that anchor, and the first YES/NO submit timestamps must be within `1.0s` of each other. Per your choice, if neither leg has been submitted by that deadline because readiness never went clean, the runtime may unlock exactly one late entry attempt once readiness becomes clean. That late entry still counts as a missed `<= 5s` KPI.

## Key Changes
### Runtime config and state
- Extend `BotRuntimeConfigSnapshot` with:
  - `open_both_seed_deadline_seconds = 5.0`
  - `open_both_submit_delta_max_seconds = 1.0`
  - `open_both_allow_single_late_seed = true`
- Add matching env keys:
  - `BOT_OPEN_BOTH_SEED_DEADLINE_SECONDS`
  - `BOT_OPEN_BOTH_SUBMIT_DELTA_MAX_SECONDS`
  - `BOT_OPEN_BOTH_ALLOW_SINGLE_LATE_SEED`
- Validate:
  - seed deadline `> 0`
  - submit delta max `> 0`
  - submit delta max `<=` seed deadline
- Extend `BotRuntimeState` with explicit startup timing fields:
  - `open_confirmed_ts`
  - `open_both_first_tradable_post_open_ts`
  - `open_both_seed_anchor_ts`
  - `open_both_seed_deadline_missed_ts`
  - `open_both_late_seed_unlock_used`
  - `open_both_late_seed_exhausted`
  - `open_both_first_yes_submit_ts`
  - `open_both_first_no_submit_ts`
  - `open_both_first_submit_delta_ms`
  - `open_both_seed_by_deadline_met`
  - `open_both_submit_delta_met`
  - `prearm_ready_before_open`

### Pre-open and open-time behavior
- Keep `PreArm` readiness requirements as they are now: market selected, asset IDs ready, market WS ready, user WS ready, fresh quotes, and valid paired quotes.
- Record `prearm_ready_before_open = true` only if `PreArm.ready` becomes true before `open_confirmed_ts`.
- At the first `OpenBoth` cycle, record `open_confirmed_ts` if it is still unset.
- Record `open_both_first_tradable_post_open_ts` only when:
  - both YES/NO cached quotes exist
  - both quote timestamps are post-open (`>= open_confirmed_ts`)
  - `_bot_runtime_startup_pair_quote_status()` passes
- Compute `open_both_seed_anchor_ts` from the earlier nonzero of `open_confirmed_ts` and `open_both_first_tradable_post_open_ts`.
- Compute the seed deadline as `anchor + open_both_seed_deadline_seconds`.

### OpenBoth submit policy
- Keep `_maker_submit_pair_orders(...)` as the paired submit primitive.
- Use the paired-submit call start timestamp as the submit timestamp for any leg that becomes newly live in that call.
- Record first successful YES and NO seed submit timestamps independently.
- `REQ-005` success is met only when both first leg-submit timestamps exist and `max(yes_ts, no_ts) <= deadline`.
- `REQ-006` success is met only when both first leg-submit timestamps exist and `abs(yes_ts - no_ts) <= 1.0s`.
- Before the deadline:
  - run the current paired submit path normally
- After the deadline:
  - if neither leg has a first submit yet, record one deadline miss and suppress fresh seeding until readiness becomes clean
  - once readiness becomes clean, allow exactly one late-entry unlock
  - the next paired seed submit may proceed under that unlock
  - if that unlocked attempt creates zero new legs, mark `open_both_late_seed_exhausted = true` and stop issuing fresh post-deadline pair-submit attempts for the market
- Important exception:
  - if one leg already has a first submit before the deadline, do not block the missing leg after `5s`
  - existing asymmetry resolution and missing-leg seeding may continue
  - the timing KPI simply records failure if the second first-submit lands after `5s` or the cross-leg delta exceeds `1s`
- Do not move fill-timing or second-side fill deadlines into this task; those remain `IMP-05`.

### Telemetry and loop reporting
- Add explicit logs for:
  - `open_confirmed`
  - `first_tradable_post_open`
  - `seed_deadline_missed`
  - `late_seed_unlock`
  - first YES seed submit
  - first NO seed submit
  - final seed submit delta
- Extend periodic and final BOT metrics with:
  - `prearm_ready_before_open`
  - `seed_anchor_t_into`
  - first YES submit `t_into`
  - first NO submit `t_into`
  - `seed_by_5s_met`
  - `late_seed_used`
  - `seed_submit_delta_ms`
  - `seed_submit_delta_met`
- No DB schema or repository changes in `IMP-04`; this is runtime-state, logging, and test coverage only.

## Important Interface Changes
- `BotRuntimeConfigSnapshot` gains the three startup-timing fields above.
- `BotRuntimeState` gains the startup timing, late-entry, and KPI fields above.
- No external API or DB interface changes are required.
- `_maker_submit_pair_orders(...)` stays in place; the runtime records timing around it rather than replacing it.

## Test Plan
- Config tests:
  - defaults are `5.0s`, `1.0s`, and `true`
  - invalid deadline or delta values fail validation
- Policy tests:
  - anchor selection uses the earlier nonzero runtime-observed timestamp
  - first tradable post-open ignores pre-open quote timestamps
  - submit delta math is correct and only evaluates once both first-submit timestamps exist
- Runtime tests:
  - `PreArm` can become ready before open and records `prearm_ready_before_open`
  - first post-open cycle records `open_confirmed_ts`
  - both first submits created in the same paired call meet `<= 5s` and `<= 1s`
  - staggered first submits within `5s` but over `1s` fail only the delta KPI
  - a second leg first-submitted after `5s` fails the deadline KPI
  - if neither leg is submitted by `5s`, one late-entry unlock is allowed once readiness becomes clean
  - if the unlocked late attempt creates zero new legs, no repeated fresh post-deadline pair submits occur
  - if one leg already submitted before the deadline, missing-leg seeding can continue after `5s` without using the late-entry unlock path
- Regression coverage:
  - existing `OpenBoth -> SeedCompletion -> PairBuild` routing is preserved
  - paired submit still happens through the existing single `_maker_submit_pair_orders(...)` path

## Assumptions and Defaults
- `start_ts` remains the official open clock source; no new exchange open-event stream is introduced here.
- “First tradable post-open event” is defined from cached post-open quote timestamps, not a separate REST lookup.
- `OpenBoth` remains the `0-30s` phase for now; exact `0-5s` startup timing is enforced inside the handler rather than by changing phase boundaries.
- An “entered pair” is any pair with at least one successful seed submit, including late-entry pairs; late-entry pairs count as failures for the `<= 5s` KPI.
- Second-side fill timing, no-scale-before-both-filled behavior, and `AwaitSecondFill` state work stay in `IMP-05`.
