# CURRENT BEHAVIOUR

## Version 0.1.28

Scope: Sprint 4 only  
Mode: `EXEC_MODE=WALLET_CLONE`  
Date: 2026-03-11

This file is not a design target.
It is a concrete runtime note for the current Sprint 4 wallet-clone path in the working tree.

---

## Update Note

`0.1.28` records the current runnable Sprint 4 wallet-clone implementation.

The main current-tree changes are:

1. `WALLET_CLONE` now has its own runtime loop boundary and is no longer forced through the older market / settlement-shaper routing.
2. The wallet-clone controller now runs explicit `PreArm`, `OpenBoth`, `SeedCompletion`, `PairBuild`, `Taper`, and rollover ownership.
3. Startup seeding now uses wallet-clone-specific quote checks instead of the stricter generic parity / spread gate.
4. Missing-side startup repair stays on the intended Sprint 4 path:
   - missing-side quote health is required
   - hard skew is bypassed for startup completion
   - shape-target gating is bypassed for startup completion
   - CPP is not a hard normal-flow veto during startup completion
5. Wallet-clone phase budget fractions now affect live runtime behavior in `OpenBoth`, `PairBuild`, and `Taper`.
6. Wallet-clone metrics now count actual fill events instead of inferring fills from raw inventory deltas.
7. A flat market that reaches `PairBuild` with `qYES=0` and `qNO=0` now keeps `OpenBoth` live instead of falling through to an inactive owner branch.

---

## Executive Summary

The Sprint 4 runtime is now structurally runnable.

The current code path can:

1. pre-arm before open
2. seed both sides
3. treat one-sided startup fills as normal startup completion
4. replenish through `PairBuild`
5. taper late
6. emit wallet-clone-specific metrics and config logs

The current code path does **not** yet prove:

1. that live canary behavior matches the target wallet closely enough
2. that observed fill cadence and participation are stable across real markets
3. that the wallet-clone path is production-ready

---

## Current Practical Reading

Relative to Sprint 4 requirements, the current tree is approximately:

1. runnable as an isolated wallet-clone mode
2. mechanically aligned on startup ownership and missing-side repair
3. materially closer to an aggressive inventory builder than the older settlement-shaper path
4. still awaiting real canary evidence before claiming behavioral match

---

## Known Remaining Gap

The dominant remaining gap is no longer hidden config wiring or missing controller ownership.

The dominant remaining gap is empirical:

1. run the wallet-clone canary
2. inspect live logs and end-of-market metrics
3. verify that participation, startup timing, paired growth, and late taper resemble the Sprint 4 target

Until that canary is reviewed, `0.1.28` should be read as:

1. Sprint 4 runtime implemented
2. config surface implemented
3. metrics surface implemented
4. live wallet-clone validation pending
