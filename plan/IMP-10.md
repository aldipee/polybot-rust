# IMP-10 Plan: Residual Directional Hard Blocks

## Summary
Implement `REQ-018` and `REQ-019` by making residual-direction control a first-class runtime gate.

Chosen decisions:
- `favorite_side` / `underdog_side` use current buy-side `bid` quotes
- if bids are equal within one tick, both are `None`
- when the runtime detects an illegal underdog-residual increase, it cancels BOT-owned working orders on that side only
- allowed one-sided actions are limited to:
  - `AwaitSecondFill` completion and its one-shot rescue
  - lagging-side repair that reduces residual and does not increase underdog residual
- all other intentional one-sided adds are blocked as `single_side_speculative_add`

## Key Changes
### Canonical residual metrics
- Add shared helpers that compute from current runtime state:
  - `favorite_side` = side with higher current bid
  - `underdog_side` = side with lower current bid
  - tie within one tick => both `None`
  - `residual_side` = side with larger filled qty, else `None`
  - `residual_kind` = `favorite`, `underdog`, or `none`
  - projected residual side and projected residual magnitude after a candidate order
  - `would_increase_underdog_residual`
- Use current filled qty only for residual classification; this task does not introduce FIFO residual lots yet.

### Runtime decision policy
- Keep `PairedGrowth` legal under `IMP-10`; it does not intentionally create one-sided exposure.
- Restrict `LighterSideFirst` to two explicit categories:
  - `SecondSideCompletion`
  - `LaggingSideRepair`
- `AwaitSecondFill` remains the only startup completion path and is always tagged `SecondSideCompletion`.
- Pair-build and taper one-sided decisions are always tagged `LaggingSideRepair`; they must:
  - reduce unmatched fraction
  - target the lagging side only
  - not increase residual magnitude on the underdog side
- Add a hard block reason for any non-completion, non-repair one-sided intent:
  - `single_side_speculative_add:<side>`
- Add a hard block reason when an order would increase underdog residual:
  - `underdog_residual_increase_block:<side>:<residual_side>:<underdog_side>`

### Order cleanup and handler behavior
- On an underdog-residual block, cancel BOT-owned working orders on the blocked side across:
  - `BOT_OPEN_BOTH`
  - `BOT_AWAIT_SECOND_FILL`
  - `BOT_PAIR_BUILD`
  - `BOT_TAPER`
- Cancel only that side, not the opposite side and not manual/recovery orders.
- Wire the same residual-direction gate through:
  - pair-build planning
  - tail-repair rewrite / taper planning
  - startup completion decision logging
- Preserve the current `AwaitSecondFill` rescue exception, but still require that it reduces imbalance and does not create an underdog-residual increase.

### Decision payloads and telemetry
- Extend `BotRuntimePairBuildDecision` with explicit residual-direction fields:
  - `favorite_side`
  - `underdog_side`
  - `residual_side`
  - `projected_residual_side`
  - `residual_kind`
  - `increases_underdog_residual`
  - `one_side_exception_kind` (`none`, `second_side_completion`, `lagging_side_repair`)
- Add stable logging fields in pair-build and taper:
  - `favorite_side`
  - `underdog_side`
  - `residual_side`
  - `projected_residual_side`
  - `residual_kind`
  - `increases_underdog_residual`
  - `one_side_exception_kind`
- Keep persistence changes out of `IMP-10`; append-only decision storage stays in `IMP-13`.

## Important Interface Changes
- `BotRuntimePairBuildDecision` gains the residual-direction and one-sided-exception fields above.
- Add a small enum for residual classification, for example:
  - `BotRuntimeResidualKind { None, Favorite, Underdog }`
- Add a small enum for allowed one-sided intent type, for example:
  - `BotRuntimeOneSideExceptionKind { None, SecondSideCompletion, LaggingSideRepair }`

## Test Plan
- Favorite/underdog classification:
  - higher bid => favorite / lower bid => underdog
  - equal within one tick => both `None`
- Residual classification:
  - larger filled qty determines `residual_side`
  - equal qty => no residual side
- Pair-build / taper policy:
  - paired growth is not blocked solely because an underdog residual already exists if projected residual magnitude does not increase
  - lagging-side repair that reduces residual and does not increase underdog residual stays allowed
  - one-sided repair on the underdog residual side is blocked when it would increase residual magnitude
  - speculative one-sided add path returns `single_side_speculative_add`
- Cleanup behavior:
  - underdog-residual block cancels BOT-owned working orders on the blocked side only
  - opposite-side BOT orders remain untouched if they are not part of the illegal increase
- Startup behavior:
  - `AwaitSecondFill` maker completion remains allowed
  - one-shot rescue remains allowed only when it reduces imbalance and does not create an underdog-residual increase
- Telemetry:
  - pair-build and taper logs expose favorite/underdog/residual fields and exception kind
  - blocked decisions emit the new stable reason codes

## Assumptions and Defaults
- Use current bid as the MVP reference price basis.
- Tie within one tick means no favorite/underdog classification for that moment.
- Cancel scope is BOT-owned working orders on the blocked side only.
- Recovery/manual one-sided logic is unchanged in this task.
- `IMP-16` remains the future place for model-edge directional exceptions; `IMP-10` assumes the overlay is off.
