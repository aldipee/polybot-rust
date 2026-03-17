# Wallet Clone Only Removal Map

## Goal

This document maps which methods and method families should be removed if the repo is reduced to a single runtime target:

```env
EXEC_MODE=WALLET_CLONE
```

This is a removal map only. It is intentionally split into:

1. safe first-pass removals
2. shared helpers that look mode-specific but are still needed by wallet clone
3. second-pass removals after `cargo check` confirms what is really dead

## Keep First

Do not delete these in the first pass. They still support `WALLET_CLONE` even if their names came from older mode work.

### Shared maker/order infrastructure in `src/bot.rs`

- `_maker_order_slot_get`
- `_maker_order_request_cancel`
- `_maker_submit_pair_orders`
- `_maker_order_upsert_gtc`
- `_maker_replace_min_interval_seconds`
- `_maker_submit_reject_cooldown_seconds`
- `_maker_order_*` family in general
- `MakerOrderKey`, `MakerOrderSlot`, `MakerExec*`, and related order-ledger state

Wallet clone is calling these directly during:

- `OPEN_BOTH`
- `SEED_COMPLETION`
- `PAIR_BUILD`
- `TAPER`

### Shared helpers that currently live under `settlement_shaper_*` names

These are misnamed shared utilities, not settlement-shaper-only logic. Keep them first, then rename later.

- `settlement_shaper_has_side_participation`
- `settlement_shaper_round_down_to_lot`
- `settlement_shaper_round_up_to_lot`
- `settlement_shaper_pair_coverage`
- `settlement_shaper_share_skew_ratio`
- `settlement_shaper_market_snapshot_price`
- `settlement_shaper_market_snapshot_vwap_sum`
- `settlement_shaper_inventory_vwap_sum`
- `settlement_shaper_origin_owns_recovery`
- `settlement_shaper_maker_slot_family_live`
- `settlement_shaper_pair_submit_leg_is_new`
- `settlement_shaper_pair_order_asymmetry`
- `_settlement_shaper_pair_order_asymmetry`
- `SettlementShaperSide` enum until it is renamed to something generic

## First-Pass Removals

These are the highest-confidence removals for a wallet-clone-only repo.

### 1. Remove non-wallet runtime dispatch

#### `src/bot.rs`

Remove or collapse:

- `runtime_loop_kind`
- `RuntimeLoopKind::SignalSniper`
- `RuntimeLoopKind::Sniper`
- `RuntimeLoopKind::SettlementShaper`
- `RuntimeLoopKind::Market`
- the non-wallet arms inside `run()`
- the legacy market-loop body that runs after the wallet-clone match arm in `run()`
- `_is_sniper_like_mode`
- `_default_entry_reason` branches for signal/sniper

If the repo is truly wallet-clone-only, `run()` should dispatch straight into `_run_wallet_clone_loop()` and stop there.

#### `src/main.rs`

Remove startup/runtime branches related to:

- signal-mode bootstrapping
- signal hub setup
- signal-file setup
- any mode checks that exist only to distinguish `SETTLEMENT_SHAPER`, `SNIPER`, `SIGNAL_*`, or legacy market mode

### 2. Remove settlement shaper runtime and reporting

#### `src/bot.rs`

Remove these explicit settlement-shaper entry points:

- `_run_settlement_shaper_loop`
- `_log_settlement_shaper_cfg`
- `settlement_shaper_final_metrics_snapshot`

Remove all settlement-shaper-exclusive helpers and config loaders. In practice this means all `settlement_shaper_*` functions except the shared helper list in the `Keep First` section.

That includes these exclusive families:

- phase control and resolution lifecycle
- settlement-shaper config readers
- settlement-shaper budget bands
- settlement-shaper target-gap logic
- settlement-shaper owner/handler selection
- settlement-shaper clip ladder logic
- settlement-shaper optionality and overlay logic
- settlement-shaper metrics summary builders

Representative exclusive method names:

- `settlement_shaper_should_seed_both_sides`
- `settlement_shaper_entry_repair_reason`
- `settlement_shaper_entry_repair_missing_side`
- `settlement_shaper_entry_repair_clip`
- `settlement_shaper_preferred_rung`
- `settlement_shaper_core_build_decision`
- `settlement_shaper_shape_repair_plan`
- `settlement_shaper_owner_decision_for_phase`
- `settlement_shaper_metrics_summary_from_runtime`
- `settlement_shaper_phase_from_t_into_s`
- `settlement_shaper_resolution_state_from_rtds_fields`
- `settlement_shaper_next_phase`
- `settlement_shaper_settlement_state_from_resolution`
- `settlement_shaper_should_stop_for_rollover`
- `settlement_shaper_default_config`
- `settlement_shaper_config_from_reader`
- `settlement_shaper_config_from_env`
- `settlement_shaper_phase_budget_snapshot`
- `settlement_shaper_vwap_regime`
- `settlement_shaper_optionality_policy`
- `settlement_shaper_favorite_size_up_candidate_clips`
- `settlement_shaper_underdog_overlay_candidate_clips`

#### `src/main.rs`

Remove settlement-shaper-only reporting helpers:

- `settlement_shaper_target_gaps_log`
- `settlement_shaper_phase_action_mix_log`
- `settlement_shaper_metrics_consistency_log`
- `settlement_shaper_trade_metadata`

Remove the settlement-shaper-only finalization block that:

- checks `bot.exec_mode.eq_ignore_ascii_case("SETTLEMENT_SHAPER")`
- builds `settlement_shaper_summary`
- emits `[SETTLEMENT_SHAPER][METRICS]`
- writes settlement-shaper trade metadata

Remove settlement-shaper-specific tests at the bottom of `src/main.rs`.

### 3. Remove sniper and signal-mode runtime

#### `src/bot.rs`

Remove:

- `_run_signal_sniper_loop`
- `_run_sniper_loop`
- all `_sniper_*` methods
- `SniperStopCertaintyConfig`
- sniper-only runtime state and helper structs

This includes the following method families:

- sniper filters
- breakout anchor logic
- sniper trade-decision logging
- sniper stop-loss handling
- sniper endgame entry logic
- sniper hedge/unwind logic
- signal-sniper entry and signal-follow logic

Representative method names:

- `_sniper_filters_load_state`
- `_sniper_filters_save_state`
- `_sniper_filters_eval_entry`
- `_sniper_build_breakout_entry_anchor`
- `_sniper_stop_loss_mode`
- `_sniper_stop_loss_fallback_mode`
- `_sniper_force_entry_diff_signal`
- `_sniper_endgame_side_from_rtds`
- `_sniper_entry_candidate_for_side`
- `_sniper_maybe_exit_hedge`
- `_sniper_maybe_exit_hedge_with_opts`

#### `src/main.rs`

Remove signal/sniper startup handling:

- `signal_mode` detection
- `SignalHub`, `SignalInbox`, `JsonlFileService` setup that only exists for signal modes
- `SIGNAL_FOLLOW_SLUG` bootstrap branch

### 4. Remove legacy maker-skew, pair-base, and taker-pair strategy logic

Important: this does **not** mean delete generic maker order infrastructure. Only delete the old strategy layer that is no longer needed when wallet clone is the only execution mode.

#### `src/bot.rs`

Remove these legacy strategy families:

- `_maker_skew_update_state`
- `_maker_recovery_*`
- `_maker_ladder_*`
- `_maker_stretch_bias_side`
- `_maker_pair_base_try_skew`
- `_maker_pair_base_recovery_phase`
- `_maker_pair_base_risk_exit_step`
- `_maker_pair_base_recovery_step`
- `_maker_pair_base_step`
- `_maker_skew_try_arb`
- `_maker_quote_only_step`
- `_maker_skew_arb_step`
- `_maker_skew_handle_base_seed_phase`
- `_maker_skew_handle_shared_gate_phase`
- `_maker_skew_handle_recovery_phase`
- `_maker_skew_handle_directional_phase`
- `_maker_quote_only_allowed`
- `pair_base_metrics_snapshot`

Remove legacy mode checks inside generic code:

- `self.exec_mode == "MAKER_SKEW_ARB"`
- `self.exec_mode == "TAKER_PAIR"`
- logic branching on pair-base enablement under maker-skew mode

Remove legacy pair-arb/taker-pair helpers and branches around:

- `PAIR_ARB_*`
- `TAKER_PAIR`

Representative hotspots:

- near-expiry market-loop logic that switches to pair-base risk exit or emergency taker hedge
- old maker-skew routing in `run()`
- pair-arb attempts in the 27k-28k line region of `src/bot.rs`

## Second-Pass Removals After Compile Confirms Dead Code

These are likely removable after the first cut, but they should be compiler-driven because some names may still be shared indirectly.

### `src/env_contract.rs`

Remove env-key families that no longer matter:

- `SETTLEMENT_SHAPER_*`
- `SIGNAL_*`
- `SNIPER_*`
- `PAIR_BASE_*`
- `PAIR_RECOVERY_*`
- `PAIR_ARB_*`
- `MAKER_SKEW_*`
- `LOOP_WAIT_SECONDS_SNIPER`

Review before removal:

- generic maker/order keys used by wallet clone
- risk-exit keys still shared by wallet clone
- fee-model keys
- market-data keys

### `ENVIRONMENT.md`

Delete or rewrite sections for:

- `EXEC_MODE` values other than `WALLET_CLONE`
- `SETTLEMENT_SHAPER_*`
- `SIGNAL_*`
- `SNIPER_*`
- `PAIR_BASE_*`
- `PAIR_RECOVERY_*`
- `PAIR_ARB_*`
- `MAKER_SKEW_*`

### `src/main.rs`

After legacy strategy code is removed, re-check whether these are still needed:

- `pair_base_metrics_snapshot` call site
- legacy trade-decision branches that only existed for non-wallet modes

### Tests

Remove test families that only validate deleted modes:

- settlement-shaper tests in `src/bot.rs`
- sniper/signal tests in `src/bot.rs`
- runtime-loop routing tests for removed modes
- pair-base / maker-skew tests that no longer cover wallet clone
- settlement-shaper helper tests in `src/main.rs`

Likely review targets:

- `src/bot_priority1_tests.rs`
- `src/bot_priority2_tests.rs`

## Practical Delete Order

Use this order to keep the deletion manageable:

1. Collapse runtime surface to wallet clone only.
2. Remove settlement-shaper reporting from `src/main.rs`.
3. Remove `_run_settlement_shaper_loop` and settlement-shaper-exclusive helpers.
4. Remove `_run_signal_sniper_loop`, `_run_sniper_loop`, and `_sniper_*`.
5. Remove legacy maker-skew / pair-base / pair-arb strategy methods.
6. Run `cargo check -q`.
7. Let compiler errors identify remaining shared helpers.
8. Rename the kept `settlement_shaper_*` generic helpers to neutral names.
9. Prune dead env keys, docs, and tests.

## Important Warning

Do not mass-delete every `settlement_shaper_*` or every `_maker_*` symbol by prefix.

Observed shared dependencies from wallet clone include:

- maker order slot access and cancel/submit helpers
- lot rounding helpers
- pair asymmetry helpers
- inventory and market-snapshot math helpers
- recovery ownership checks

Those should be renamed, not deleted in the first cut.
