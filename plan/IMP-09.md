# IMP-09 Plan: Exact Clip Ladder and Green-Gated Large Orders

## Summary
Implement `REQ-016` and `REQ-017` by making clip sizing use one authoritative, env-configurable ladder with requirement defaults `12 / 20 / 40 / 80`.

Chosen decisions:
- Authoritative env is `BOT_CLIP_LADDER=12,20,40,80`
- This is a clean switch: the old split clip envs stop controlling runtime clip policy
- Escalation is progressive:
  - seeds use `12`
  - normal adds use `20`
  - `40` is only eligible once matched pair base is at least `20`
  - `80` is only eligible once matched pair base is at least `40`
- Any clip above `20` requires all green conditions
- `80` remains the hard single-order cap

## Key Changes
### Config and clip policy
- Replace the current split runtime clip knobs with one authoritative `clip_ladder: [f64; 4]`.
- Parse `BOT_CLIP_LADDER` as exactly 4 ascending positive values and validate:
  - finite and `> 0`
  - ascending
  - final rung `<= 80`
- Default remains `12,20,40,80`.
- Remove runtime dependence on `BOT_SEED_CLIP_SMALL`, `BOT_REPAIR_CLIP_SMALL`, and `BOT_CLIP_LADDER_LARGE` for clip selection.
- Update startup/config logging to print the exact four-rung ladder instead of seed/repair/large split fields.

### Decision model and runtime behavior
- Add an exact rung model for decisions, for example `Seed12`, `Normal20`, `Large40`, `Large80`, and `ExactGapRepair`.
- Extend pair-build decision payloads with:
  - selected rung
  - requested rung
  - whether the order is a large-clip request
  - green-condition booleans
  - final `green_conditions_met`
- Apply one clip-selection policy across runtime:
  - `OpenBoth` seeds always target rung 1 (`12`)
  - normal paired growth targets rung 2 (`20`)
  - larger paired growth may escalate only progressively to `40` then `80`
  - lighter-side repair uses the largest legal rung `<= qty_gap`, with `40/80` still requiring green conditions
  - if no rung fits a repair gap, exact-gap repair below `12` remains allowed as the only off-ladder exception
- De-escalation rules:
  - if a requested rung fails budget, downgrade to the next smaller legal rung
  - if a requested rung `> 20` fails green conditions, downgrade to `20`
  - paired growth must never emit off-ladder sizes
  - repair may emit only a legal rung or an exact-gap residual clip
- Keep `cpp_hint`, repair reserve, and other pacing heuristics only as de-escalators between legal rungs; they must not invent arbitrary sizes like `27`, `33`, or `57`.

### Green-condition gate for 40/80
- Large clips are allowed only when all are true:
  - both sides already filled
  - current decision’s `effective_marginal_pair_cost < 0.94`
  - `projected_unmatched_fraction < 0.07`
  - `t_into_s < 180`
  - remaining budget can fund the full rung at the current decision cost
- Use the current decision cost mode from `IMP-07`:
  - balanced growth uses balanced-add cost
  - repair uses rebalance-add cost
- If a large repair or large paired add becomes ineligible, downgrade rather than silently blocking all sizing unless even `20` cannot be legally emitted.

## Public Interfaces and Telemetry
- `BotRuntimeConfigSnapshot` gains one authoritative four-rung ladder and drops clip-policy authority from the old split fields.
- `BotRuntimePairBuildDecision` gains exact rung and green-condition telemetry.
- Logs and metrics for any `40` or `80` order must include:
  - rung
  - matched pair base
  - effective marginal cost
  - projected unmatched fraction
  - `t_into_s`
  - budget-ok flag
  - `green_conditions_met`
- Existing coarse fields like `clip_bucket` may remain for compatibility, but policy and tests should assert on exact rung, not the coarse bucket.

## Test Plan
- Config tests:
  - default ladder is `12,20,40,80`
  - malformed, non-ascending, or `>80` ladders fail validation
  - old split clip envs no longer control clip sizing
- Seed/startup tests:
  - `OpenBoth` uses `12`
  - `AwaitSecondFill` does not emit `40` or `80`
- Paired-growth tests:
  - non-green growth uses `20`
  - `40` requires matched base `>= 20` plus all green conditions
  - `80` requires matched base `>= 40` plus all green conditions
  - if `80` is green but budget only supports `40`, final clip is `40`
  - no paired-growth order emits any size outside `12/20/40/80`
- Repair tests:
  - repair chooses the largest legal rung `<= qty_gap`
  - large repair (`40/80`) requires all green conditions
  - exact-gap repair below `12` remains allowed only to avoid overshoot
  - rebalance green checks use rebalance cost, not balanced-add cost
- Regression tests:
  - no emitted order exceeds `80`
  - `cpp_hint` can downgrade `40/80` to `20` or `12`, but never produce arbitrary clips
  - logs for every `40` or `80` decision prove all green conditions were true

## Assumptions and Defaults
- `BOT_CLIP_LADDER` is the only authoritative clip-sizing env after `IMP-09`.
- Seeds always use the first rung.
- Standard accumulation defaults to the second rung.
- Exact-gap repair below the first rung remains allowed as the only intentional off-ladder exception.
- Large-clip gating uses the current decision’s `effective_marginal_pair_cost`, so repair mode uses rebalance pricing automatically.
