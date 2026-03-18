use super::*;

#[derive(Debug, Clone, Default)]
pub(super) struct TakerOrderRecord {
    pub(super) order_id: String,
    pub(super) asset_id: String,
    pub(super) size: f64,
    pub(super) applied: f64,
    pub(super) px_limit: f64,
    pub(super) side: String,
    pub(super) ts: f64,
    pub(super) liquidity_intent: LiquidityIntent,
    pub(super) taker_exception_reason: Option<TakerExceptionReason>,
    pub(super) taker_cap_policy: TakerCapPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum LiquidityIntent {
    #[default]
    Maker,
    TakerException,
}

impl LiquidityIntent {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Maker => "maker",
            Self::TakerException => "taker_exception",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TakerExceptionReason {
    AwaitSecondFillRescue,
    RebalanceRepair,
    RecoveryBypass,
}

impl TakerExceptionReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::AwaitSecondFillRescue => "await_second_fill_rescue",
            Self::RebalanceRepair => "rebalance_repair",
            Self::RecoveryBypass => "recovery_bypass",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum TakerCapPolicy {
    #[default]
    EnforceCap,
    RecoveryBypass,
}

impl TakerCapPolicy {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::EnforceCap => "enforce_cap",
            Self::RecoveryBypass => "recovery_bypass",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) struct TakerShareSnapshot {
    pub(super) pair_taker_share: f64,
    pub(super) projected_pair_taker_share: f64,
    pub(super) daily_taker_share: f64,
    pub(super) projected_daily_taker_share: f64,
}

pub(super) fn taker_submit_reason_allowed(
    side: &str,
    reason: Option<TakerExceptionReason>,
    cap_policy: TakerCapPolicy,
) -> Result<TakerExceptionReason, &'static str> {
    let side_u = side.trim().to_ascii_uppercase();
    let Some(reason) = reason else {
        return Err("taker_exception_reason_missing");
    };
    match cap_policy {
        TakerCapPolicy::RecoveryBypass => {
            if reason == TakerExceptionReason::RecoveryBypass {
                Ok(reason)
            } else {
                Err("taker_exception_reason_disallowed")
            }
        }
        TakerCapPolicy::EnforceCap => match (side_u.as_str(), reason) {
            ("BUY", TakerExceptionReason::AwaitSecondFillRescue)
            | ("BUY", TakerExceptionReason::RebalanceRepair) => Ok(reason),
            _ => Err("taker_exception_reason_disallowed"),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum OutcomeSide {
    #[default]
    Yes,
    No,
}

impl OutcomeSide {
    /// Returns the stable string label for this enum or state value.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "YES",
            Self::No => "NO",
        }
    }

    /// Returns the opposite outcome side.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn opposite(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PairIdentity {
    pub(crate) pair_id: String,
    pub(crate) market_slug: String,
    pub(crate) condition_id: Option<String>,
    pub(crate) yes_asset_id: Option<String>,
    pub(crate) no_asset_id: Option<String>,
}

impl PairIdentity {
    pub(crate) fn from_slug(slug: &str) -> Self {
        Self {
            pair_id: canonical_pair_id_from_slug(slug),
            market_slug: slug.trim().to_string(),
            condition_id: None,
            yes_asset_id: None,
            no_asset_id: None,
        }
    }

    pub(crate) fn update_market_metadata(
        &mut self,
        condition_id: Option<String>,
        yes_asset_id: Option<String>,
        no_asset_id: Option<String>,
    ) {
        if let Some(condition_id) = condition_id.filter(|value| !value.trim().is_empty()) {
            self.condition_id = Some(condition_id);
        }
        if let Some(yes_asset_id) = yes_asset_id.filter(|value| !value.trim().is_empty()) {
            self.yes_asset_id = Some(yes_asset_id);
        }
        if let Some(no_asset_id) = no_asset_id.filter(|value| !value.trim().is_empty()) {
            self.no_asset_id = Some(no_asset_id);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct PairPosition {
    pub(crate) q_yes: f64,
    pub(crate) q_no: f64,
    pub(crate) c_yes: f64,
    pub(crate) c_no: f64,
}

impl PairPosition {
    pub(crate) fn total_cost(self) -> f64 {
        self.c_yes.max(0.0) + self.c_no.max(0.0)
    }

    pub(crate) fn paired_size(self) -> f64 {
        self.q_yes.max(0.0).min(self.q_no.max(0.0))
    }

    pub(crate) fn unmatched_size(self) -> f64 {
        (self.q_yes.max(0.0) - self.q_no.max(0.0)).abs()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct PairQuote {
    pub(crate) bid: f64,
    pub(crate) ask: f64,
    pub(crate) ts: f64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct PairSnapshot {
    pub(crate) identity: PairIdentity,
    pub(crate) position: PairPosition,
    pub(crate) phase: String,
    pub(crate) t_into_s: f64,
    pub(crate) total_cost: f64,
    pub(crate) paired_size: f64,
    pub(crate) unmatched_size: f64,
    pub(crate) yes_quote: Option<PairQuote>,
    pub(crate) no_quote: Option<PairQuote>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SnapshotPricingSource {
    Midpoint,
    AskBidProxy,
    FairPriceFallback,
}

impl SnapshotPricingSource {
    /// Returns the stable string label for this enum or state value.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Midpoint => "midpoint",
            Self::AskBidProxy => "ask_bid_proxy",
            Self::FairPriceFallback => "fair_price_fallback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct MakerPairOrderAsymmetry {
    pub(super) live_side: OutcomeSide,
    pub(super) state: MakerOrderLifecycle,
    pub(super) age_s: f64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct LadderOrderState {
    pub(super) key: String,
    pub(super) asset_id: String,
    pub(super) role: String,
    pub(super) level: i64,
    pub(super) order_id: String,
    pub(super) price: f64,
    pub(super) size: f64,
    pub(super) ts: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct MakerOrderKey {
    pub(super) asset_id: String,
    pub(super) side: String,
}

impl MakerOrderKey {
    /// Builds a BUY-side maker order key for the provided asset.
    /// This is a helper used by the BOT runtime for normalization, state labels, or
    /// calculations.

    pub(super) fn buy(asset_id: &str) -> Self {
        Self {
            asset_id: asset_id.trim().to_string(),
            side: "BUY".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum MakerOrderLifecycle {
    #[default]
    Idle,
    SubmitPending,
    Working,
    CancelPending,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MakerOrderReplaceTarget {
    pub(super) price: f64,
    pub(super) size: f64,
    pub(super) origin: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MakerOrderSlot {
    pub(super) state: MakerOrderLifecycle,
    pub(super) order_id: Option<String>,
    pub(super) price: f64,
    pub(super) size: f64,
    pub(super) remaining: f64,
    pub(super) last_submit_ts: f64,
    pub(super) last_cancel_ts: f64,
    pub(super) last_reject_ts: f64,
    pub(super) consecutive_rejects: u32,
    pub(super) last_reject_origin: String,
    pub(super) origin: String,
    pub(super) replace_target: Option<MakerOrderReplaceTarget>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MakerExecProgress {
    pub(super) applied_qty: f64,
    pub(super) last_update_ts: f64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MakerExecCandidate {
    pub(super) order_id: String,
    pub(super) asset_id: String,
    pub(super) side: String,
    pub(super) qty: f64,
    pub(super) price: f64,
    pub(super) tx_hash: Option<String>,
    pub(super) trade_id: Option<String>,
    pub(super) taker_order_id: Option<String>,
    pub(super) match_time: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MakerExecRecord {
    pub(super) canonical_id: String,
    pub(super) order_id: String,
    pub(super) qty: f64,
    pub(super) price: f64,
    pub(super) asset_id: String,
    pub(super) side: String,
    pub(super) aliases: Vec<String>,
    pub(super) applied_ts: f64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MakerExecLedger {
    pub(super) alias_to_canonical: HashMap<String, String>,
    pub(super) records: HashMap<String, MakerExecRecord>,
    pub(super) per_order_applied: HashMap<String, MakerExecProgress>,
    pub(super) per_order_origin: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub(super) enum MakerExecApplyResult {
    Applied {
        canonical_id: String,
    },
    Duplicate {
        canonical_id: String,
    },
    Conflict {
        canonical_id: String,
        reason: String,
    },
    DroppedWeakId {
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
pub(super) struct ApplyFillMutationMeta {
    pub(super) opened_position: bool,
    pub(super) closed_position: bool,
    pub(super) mark_first_entry_fill: bool,
}

/// Computes usable budget after reserve for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn usable_budget_after_reserve(max_total_cost: f64, reserve_usd: f64) -> f64 {
    (max_total_cost - reserve_usd).max(0.0)
}

/// Returns whether the BOT currently has side participation.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn has_side_participation(qty: f64, cost: f64) -> bool {
    qty > 1e-9 || cost > 1e-9
}

/// Computes round down to lot for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn round_down_to_lot(value: f64, lot: f64) -> f64 {
    let lot = lot.max(1.0);
    if !value.is_finite() || value <= 0.0 {
        return 0.0;
    }
    ((value / lot).floor() * lot).max(0.0)
}

/// Computes round up to lot for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn round_up_to_lot(value: f64, lot: f64) -> f64 {
    let lot = lot.max(1.0);
    if !value.is_finite() || value <= 0.0 {
        return lot;
    }
    let quotient = value / lot;
    let rounded = quotient.round();
    if (quotient - rounded).abs() <= 1e-9 {
        return (rounded.max(1.0) * lot).max(lot);
    }
    (quotient.ceil() * lot).max(lot)
}

/// Computes pair coverage for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn pair_coverage(q_yes: f64, q_no: f64) -> f64 {
    let mn = q_yes.max(0.0).min(q_no.max(0.0));
    let mx = q_yes.max(0.0).max(q_no.max(0.0));
    if mx > 1e-9 {
        mn / mx
    } else {
        1.0
    }
}

/// Computes share skew ratio for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn share_skew_ratio(q_yes: f64, q_no: f64) -> f64 {
    let mn = q_yes.max(0.0).min(q_no.max(0.0));
    let mx = q_yes.max(0.0).max(q_no.max(0.0));
    if mn > 1e-9 {
        mx / mn
    } else if mx > 1e-9 {
        f64::INFINITY
    } else {
        1.0
    }
}

/// Computes unmatched fraction for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn unmatched_fraction(q_yes: f64, q_no: f64) -> f64 {
    let yes = q_yes.max(0.0);
    let no = q_no.max(0.0);
    let total = yes + no;
    if total > 1e-9 {
        ((yes - no).abs() / total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Computes match ratio for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn match_ratio(q_yes: f64, q_no: f64) -> f64 {
    let mn = q_yes.max(0.0).min(q_no.max(0.0));
    let mx = q_yes.max(0.0).max(q_no.max(0.0));
    if mx > 1e-9 {
        (mn / mx).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Computes complement price for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn complement_price(price: f64) -> Option<f64> {
    if price > 0.0 {
        Some((1.0 - price).clamp(0.0, 1.0))
    } else {
        None
    }
}

/// Computes midpoint price for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn midpoint_price(bid: f64, ask: f64) -> Option<f64> {
    if bid > 0.0 && ask > 0.0 {
        Some(0.5 * (bid + ask))
    } else {
        None
    }
}

/// Computes ask bid proxy price for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn ask_bid_proxy_price(ask: f64, opposite_bid: f64) -> Option<f64> {
    if ask > 0.0 {
        Some(ask)
    } else {
        complement_price(opposite_bid)
    }
}

/// Computes fair price fallback for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn fair_price_fallback(
    bid: f64,
    ask: f64,
    opposite_bid: f64,
    opposite_ask: f64,
) -> Option<f64> {
    midpoint_price(bid, ask)
        .or_else(|| midpoint_price(opposite_bid, opposite_ask).and_then(complement_price))
        .or_else(|| (ask > 0.0).then_some(ask))
        .or_else(|| complement_price(opposite_bid))
        .or_else(|| complement_price(opposite_ask))
        .or_else(|| (bid > 0.0).then_some(bid))
}

/// Computes market snapshot price for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn market_snapshot_price(
    bid: f64,
    ask: f64,
    opposite_bid: f64,
    opposite_ask: f64,
) -> Option<(f64, SnapshotPricingSource)> {
    if let Some(price) = ask_bid_proxy_price(ask, opposite_bid) {
        Some((price, SnapshotPricingSource::AskBidProxy))
    } else {
        fair_price_fallback(bid, ask, opposite_bid, opposite_ask)
            .map(|price| (price, SnapshotPricingSource::FairPriceFallback))
    }
}

/// Computes inventory VWAP sum for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn inventory_vwap_sum(q_yes: f64, q_no: f64, cost_yes: f64, cost_no: f64) -> f64 {
    let yes_qty = q_yes.max(0.0);
    let no_qty = q_no.max(0.0);
    if yes_qty <= 1e-9 || no_qty <= 1e-9 {
        return f64::INFINITY;
    }
    (cost_yes.max(0.0) / yes_qty) + (cost_no.max(0.0) / no_qty)
}

/// Computes market snapshot VWAP sum for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn market_snapshot_vwap_sum(y_bid: f64, y_ask: f64, n_bid: f64, n_ask: f64) -> f64 {
    let Some((yes_price, _)) = market_snapshot_price(y_bid, y_ask, n_bid, n_ask) else {
        return f64::INFINITY;
    };
    let Some((no_price, _)) = market_snapshot_price(n_bid, n_ask, y_bid, y_ask) else {
        return f64::INFINITY;
    };
    yes_price + no_price
}

/// Computes origin owns recovery for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn origin_owns_recovery(origin: &str) -> bool {
    origin.starts_with("BOT_")
}

/// Implements slot family live for the maker-side BOT workflow.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn maker_slot_family_live(slot: &MakerOrderSlot, origin_prefix: &str) -> bool {
    let outstanding = match slot.state {
        MakerOrderLifecycle::SubmitPending => slot.remaining.max(slot.size).max(0.0),
        MakerOrderLifecycle::Working | MakerOrderLifecycle::CancelPending => {
            slot.remaining.max(0.0)
        }
        MakerOrderLifecycle::Idle => 0.0,
    };
    slot.order_id.is_some()
        && !origin_prefix.trim().is_empty()
        && slot.origin.starts_with(origin_prefix)
        && outstanding > 1e-6
        && matches!(
            slot.state,
            MakerOrderLifecycle::Working
                | MakerOrderLifecycle::SubmitPending
                | MakerOrderLifecycle::CancelPending
        )
}

/// Implements pair submit leg is new for the maker-side BOT workflow.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn maker_pair_submit_leg_is_new(
    live_oid: Option<&str>,
    prev_slot: &MakerOrderSlot,
) -> bool {
    let Some(live_oid) = live_oid.filter(|oid| !oid.trim().is_empty()) else {
        return false;
    };
    prev_slot.order_id.as_deref() != Some(live_oid)
        || prev_slot.state != MakerOrderLifecycle::Working
}

/// Implements order lifecycle label for the maker-side BOT workflow.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn maker_order_lifecycle_label(state: MakerOrderLifecycle) -> &'static str {
    match state {
        MakerOrderLifecycle::Idle => "Idle",
        MakerOrderLifecycle::SubmitPending => "SubmitPending",
        MakerOrderLifecycle::Working => "Working",
        MakerOrderLifecycle::CancelPending => "CancelPending",
    }
}

/// Implements pair order asymmetry for the maker-side BOT workflow.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn maker_pair_order_asymmetry(
    yes_slot: &MakerOrderSlot,
    no_slot: &MakerOrderSlot,
    origin_prefix: &str,
    now: f64,
) -> Option<MakerPairOrderAsymmetry> {
    let yes_live = maker_slot_family_live(yes_slot, origin_prefix);
    let no_live = maker_slot_family_live(no_slot, origin_prefix);
    match (yes_live, no_live) {
        (true, false) => Some(MakerPairOrderAsymmetry {
            live_side: OutcomeSide::Yes,
            state: yes_slot.state,
            age_s: (now - yes_slot.last_submit_ts).max(0.0),
        }),
        (false, true) => Some(MakerPairOrderAsymmetry {
            live_side: OutcomeSide::No,
            state: no_slot.state,
            age_s: (now - no_slot.last_submit_ts).max(0.0),
        }),
        _ => None,
    }
}

/// Computes pair submit tracks taker fallback for the BOT runtime.
/// This is a helper used by the BOT runtime for normalization, state labels, or calculations.

pub(super) fn pair_submit_tracks_taker_fallback(resolved_order_type: &str) -> bool {
    resolved_order_type.trim().to_ascii_uppercase() != "GTC"
}
