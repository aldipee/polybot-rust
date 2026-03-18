use super::*;
/// Implements prearm status from snapshot for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_prearm_status_from_snapshot(
    t_into_s: f64,
    market_selected: bool,
    asset_ids_ready: bool,
    market_ws_ready: bool,
    user_ws_ready: bool,
    quotes_ready: bool,
    quote_input_reason: &str,
    paired_quotes_ready: bool,
    paired_quote_reason: &str,
) -> BotRuntimePreArmStatus {
    let hold_reason = if t_into_s >= 0.0 {
        "market_open".to_string()
    } else if !market_selected {
        "market_not_selected".to_string()
    } else if !asset_ids_ready {
        "asset_ids_missing".to_string()
    } else if !market_ws_ready {
        "market_ws_disconnected".to_string()
    } else if !user_ws_ready {
        "user_ws_disconnected".to_string()
    } else if !quotes_ready {
        format!("quote_inputs_unready:{quote_input_reason}")
    } else if !paired_quotes_ready {
        format!("paired_quotes_unready:{paired_quote_reason}")
    } else {
        "ready".to_string()
    };
    BotRuntimePreArmStatus {
        market_selected,
        asset_ids_ready,
        market_ws_ready,
        user_ws_ready,
        quotes_ready,
        quote_input_reason: quote_input_reason.to_string(),
        paired_quotes_ready,
        paired_quote_reason: paired_quote_reason.to_string(),
        ready: t_into_s < 0.0
            && market_selected
            && asset_ids_ready
            && market_ws_ready
            && user_ws_ready
            && quotes_ready
            && paired_quotes_ready,
        hold_reason,
    }
}
/// Implements open both seed size for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_open_both_seed_size(
    configured_clip: f64,
    min_shares: f64,
    pair_sum: f64,
    total_usable_budget: f64,
    total_cost: f64,
) -> Option<i64> {
    if pair_sum <= 0.0 || !pair_sum.is_finite() {
        return None;
    }
    let remaining_budget = (total_usable_budget.max(0.0) - total_cost.max(0.0)).max(0.0);
    if remaining_budget <= 0.0 {
        return None;
    }
    let min_lot = min_shares.max(1.0).floor();
    let preferred_clip = configured_clip.max(min_lot).floor();
    let budget_clip_cap = (remaining_budget / pair_sum).floor();
    let clip = preferred_clip.min(budget_clip_cap).floor();
    if clip < min_lot || clip <= 0.0 {
        None
    } else {
        Some(clip as i64)
    }
}

/// Returns the canonical startup seed anchor for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_open_both_seed_anchor_ts(
    open_confirmed_ts: f64,
    first_tradable_post_open_ts: f64,
) -> f64 {
    match (open_confirmed_ts > 0.0, first_tradable_post_open_ts > 0.0) {
        (true, true) => open_confirmed_ts.min(first_tradable_post_open_ts),
        (true, false) => open_confirmed_ts,
        (false, true) => first_tradable_post_open_ts,
        (false, false) => 0.0,
    }
}

/// Returns the startup seed deadline timestamp for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_open_both_seed_deadline_ts(
    anchor_ts: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> f64 {
    if anchor_ts > 0.0 {
        anchor_ts + cfg.open_both_seed_deadline_seconds.max(0.0)
    } else {
        0.0
    }
}

/// Returns the first-submit delta between the YES and NO startup legs.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_open_both_submit_delta_ms(
    yes_submit_ts: f64,
    no_submit_ts: f64,
) -> Option<f64> {
    if yes_submit_ts > 0.0 && no_submit_ts > 0.0 {
        Some(((yes_submit_ts - no_submit_ts).abs()).max(0.0) * 1000.0)
    } else {
        None
    }
}
/// Returns the target completion threshold for AwaitSecondFill.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_await_second_fill_target_seconds() -> f64 {
    15.0
}

/// Returns the hard completion deadline for AwaitSecondFill.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_await_second_fill_deadline_seconds() -> f64 {
    30.0
}

/// Implements await second fill missing side for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_await_second_fill_missing_side(
    q_yes: f64,
    cost_yes: f64,
    q_no: f64,
    cost_no: f64,
) -> Option<OutcomeSide> {
    let yes_live = has_side_participation(q_yes, cost_yes);
    let no_live = has_side_participation(q_no, cost_no);
    match (yes_live, no_live) {
        (false, true) => Some(OutcomeSide::Yes),
        (true, false) => Some(OutcomeSide::No),
        _ => None,
    }
}
/// Implements await second fill repair size for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_await_second_fill_repair_size(
    configured_clip: f64,
    min_shares: f64,
    missing_price: f64,
    total_usable_budget: f64,
    total_cost: f64,
) -> Option<i64> {
    if missing_price <= 0.0 || !missing_price.is_finite() {
        return None;
    }
    let remaining_budget = (total_usable_budget.max(0.0) - total_cost.max(0.0)).max(0.0);
    if remaining_budget <= 0.0 {
        return None;
    }
    let min_lot = min_shares.max(1.0).floor();
    let preferred_clip = configured_clip.max(min_lot).floor();
    let budget_clip_cap = (remaining_budget / missing_price).floor();
    let clip = preferred_clip.min(budget_clip_cap).floor();
    if clip < min_lot || clip <= 0.0 {
        None
    } else {
        Some(clip as i64)
    }
}

/// Returns the unmatched filled size available for a missing-side rescue.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_await_second_fill_unmatched_size(q_yes: f64, q_no: f64) -> f64 {
    (q_yes.max(0.0) - q_no.max(0.0)).abs()
}

/// Returns the marginal pair sum for a missing-side rescue decision.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_await_second_fill_marginal_pair_sum(
    missing_side: OutcomeSide,
    q_yes: f64,
    q_no: f64,
    cost_yes: f64,
    cost_no: f64,
    missing_ask: f64,
) -> Option<f64> {
    if missing_ask <= 0.0 || !missing_ask.is_finite() {
        return None;
    }
    let (filled_qty, filled_cost) = match missing_side {
        OutcomeSide::Yes => (q_no.max(0.0), cost_no.max(0.0)),
        OutcomeSide::No => (q_yes.max(0.0), cost_yes.max(0.0)),
    };
    if filled_qty <= 1e-9 {
        return None;
    }
    Some((filled_cost / filled_qty.max(1e-9)).max(0.0) + missing_ask.max(0.0))
}

/// Returns the one-shot taker rescue clip for AwaitSecondFill.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_await_second_fill_rescue_size(
    repair_size: i64,
    unmatched_size: f64,
    visible_ask_size: f64,
    min_shares: f64,
) -> Option<i64> {
    if repair_size <= 0 {
        return None;
    }
    let min_lot = min_shares.max(1.0).floor();
    let capped = (repair_size as f64)
        .min(unmatched_size.max(0.0).floor())
        .min(visible_ask_size.max(0.0).floor())
        .floor();
    if capped < min_lot || capped <= 0.0 {
        None
    } else {
        Some(capped as i64)
    }
}
/// Implements quote snapshot status for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_quote_snapshot_status(
    label: &str,
    quote: Option<(f64, f64, f64)>,
    now: f64,
    stale_s: f64,
) -> (bool, String) {
    let Some((bid, ask, ts)) = quote else {
        return (false, format!("missing_quotes_{label}"));
    };
    if bid <= 0.0 || ask <= 0.0 {
        return (false, format!("zero_bid_ask_{label}"));
    }
    if ts <= 0.0 {
        return (false, format!("quote_ts_missing_{label}"));
    }
    if (now - ts) > stale_s {
        return (false, format!("quote_ts_stale_{label}"));
    }
    (true, "ok".to_string())
}

/// Returns whether the ask side of a quote is fresh enough for an ask-driven taker rescue.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_ask_snapshot_status(
    label: &str,
    quote: Option<(f64, f64, f64)>,
    now: f64,
    stale_s: f64,
) -> (bool, String) {
    let Some((_bid, ask, ts)) = quote else {
        return (false, format!("missing_quotes_{label}"));
    };
    if ask <= 0.0 {
        return (false, format!("zero_ask_{label}"));
    }
    if ts <= 0.0 {
        return (false, format!("quote_ts_missing_{label}"));
    }
    if (now - ts) > stale_s {
        return (false, format!("quote_ts_stale_{label}"));
    }
    (true, "ok".to_string())
}
/// Implements startup pair quote status for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_startup_pair_quote_status(
    yes_quote: Option<(f64, f64, f64)>,
    no_quote: Option<(f64, f64, f64)>,
    now: f64,
    stale_s: f64,
) -> (bool, String) {
    let (yes_ready, yes_reason) = bot_runtime_quote_snapshot_status("YES", yes_quote, now, stale_s);
    if !yes_ready {
        return (false, yes_reason);
    }
    let (no_ready, no_reason) = bot_runtime_quote_snapshot_status("NO", no_quote, now, stale_s);
    if !no_ready {
        return (false, no_reason);
    }
    let Some((y_bid, _y_ask, _)) = yes_quote else {
        return (false, "missing_quotes_YES".to_string());
    };
    let Some((n_bid, _n_ask, _)) = no_quote else {
        return (false, "missing_quotes_NO".to_string());
    };
    if y_bid <= 0.0 || n_bid <= 0.0 {
        return (false, "zero_bid_pair".to_string());
    }
    let pair_sum = y_bid + n_bid;
    if !pair_sum.is_finite() || pair_sum <= 0.0 {
        return (false, "pair_sum_unusable".to_string());
    }
    if pair_sum >= 1.0 {
        return (false, format!("pair_sum_too_high({pair_sum:.3})"));
    }
    (true, "ok".to_string())
}

/// Returns whether both quotes are fresh and observed post-open for startup timing.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_post_open_pair_quote_status(
    yes_quote: Option<(f64, f64, f64)>,
    no_quote: Option<(f64, f64, f64)>,
    open_confirmed_ts: f64,
    now: f64,
    stale_s: f64,
) -> (bool, String) {
    if open_confirmed_ts <= 0.0 {
        return (false, "open_unconfirmed".to_string());
    }
    let (ready, reason) = bot_runtime_startup_pair_quote_status(yes_quote, no_quote, now, stale_s);
    if !ready {
        return (false, reason);
    }
    let Some((_, _, yes_ts)) = yes_quote else {
        return (false, "missing_quotes_YES".to_string());
    };
    let Some((_, _, no_ts)) = no_quote else {
        return (false, "missing_quotes_NO".to_string());
    };
    if yes_ts + 1e-9 < open_confirmed_ts {
        return (false, "yes_quote_pre_open".to_string());
    }
    if no_ts + 1e-9 < open_confirmed_ts {
        return (false, "no_quote_pre_open".to_string());
    }
    (true, "ok".to_string())
}
/// Implements cumulative budget fractions for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_cumulative_budget_fractions(
    t_into_s: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> (f64, f64) {
    if t_into_s < 0.0 {
        return (0.0, 0.0);
    }
    let mut min_fraction = cfg.seed_budget_min_fraction.max(0.0);
    let mut max_fraction = cfg.seed_budget_max_fraction.max(min_fraction);
    if t_into_s >= 30.0 && 30.0 < cfg.taper_start_seconds {
        min_fraction += cfg.early_budget_min_fraction.max(0.0);
        max_fraction += cfg.early_budget_max_fraction.max(0.0);
    }
    if t_into_s >= 60.0 && 60.0 < cfg.taper_start_seconds {
        min_fraction += cfg.main_budget_min_fraction.max(0.0);
        max_fraction += cfg.main_budget_max_fraction.max(0.0);
    }
    if t_into_s >= 180.0 && 180.0 < cfg.taper_start_seconds {
        min_fraction += cfg.late_budget_min_fraction.max(0.0);
        max_fraction += cfg.late_budget_max_fraction.max(0.0);
    }
    if t_into_s >= cfg.taper_start_seconds {
        min_fraction += cfg.taper_budget_min_fraction.max(0.0);
        max_fraction += cfg.taper_budget_max_fraction.max(0.0);
    }
    let min_fraction = min_fraction.clamp(0.0, 1.0);
    let max_fraction = max_fraction.max(min_fraction).clamp(0.0, 1.0);
    (min_fraction, max_fraction)
}
/// Implements budget snapshot for the BOT runtime.
/// This is a pure BOT runtime helper used for configuration, policy, or metrics calculations.

pub(in crate::bot) fn bot_runtime_budget_snapshot(
    t_into_s: f64,
    total_usable_budget: f64,
    total_cost: f64,
    cfg: &BotRuntimeConfigSnapshot,
) -> BotRuntimeBudgetSnapshot {
    let budget = total_usable_budget.max(0.0);
    let cost = total_cost.max(0.0);
    let (cumulative_min_fraction, cumulative_max_fraction) =
        bot_runtime_cumulative_budget_fractions(t_into_s, cfg);
    let cumulative_min_cost = budget * cumulative_min_fraction;
    let cumulative_max_cost = budget * cumulative_max_fraction;
    BotRuntimeBudgetSnapshot {
        cumulative_min_fraction,
        cumulative_max_fraction,
        cumulative_min_cost,
        cumulative_max_cost,
        remaining_to_max_cost: (cumulative_max_cost - cost).max(0.0),
        under_min_target: cost + 1e-9 < cumulative_min_cost,
    }
}
