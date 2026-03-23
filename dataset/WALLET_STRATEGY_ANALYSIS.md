# Vidarx Wallet Strategy Analysis

**Wallet:** `0x2d8b401d2f0e6937afebf18e19e11ca568a5260a`
**Dataset:** 218,047 trades across 790 BTC 5-minute markets (March 9-12, 2026)
**Closed Positions:** 387 positions across 195 markets
**Net Profit:** $10,296.36

---

## Executive Summary

This wallet runs a **dual-side market-making / arbitrage strategy** on Polymarket BTC 5-minute Up/Down markets. The core profit mechanism is NOT directional prediction — it is **buying BOTH sides (Up AND Down) at a combined VWAP below $1.00**, guaranteeing a structural edge regardless of outcome. The wallet compounds this with a mild mean-reversion tilt and dominant maker (limit-order) execution.

---

## 1. Core Strategy: Dual-Side Structural Arbitrage

### The Fundamental Mechanism

| Metric | Value |
|--------|-------|
| Markets with BOTH Up+Down positions | **99.4%** (785/790) |
| Mean VWAP sum (Up_price + Down_price) | **0.9657** |
| Markets where VWAP sum < 1.00 | **83.9%** |
| Mean structural edge per market | **3.43 cents per dollar** |

**How it works:** In a binary market, one side ALWAYS resolves to $1.00 and the other to $0.00. If you buy $X of Up at price `p_up` and $X of Down at price `p_down`, and `p_up + p_down < 1.00`, you **profit regardless of outcome**. The wallet achieves an average combined price of $0.9657, yielding ~3.4% edge per market.

### Evidence from Example Markets

| Market | Up VWAP | Down VWAP | Sum | Edge |
|--------|---------|-----------|-----|------|
| March 10, 11:45AM | 0.3553 | 0.5813 | 0.9366 | **6.34%** |
| March 10, 11:00AM | 0.5121 | 0.4565 | 0.9686 | **3.14%** |
| March 10, 12:00PM | 0.3113 | 0.6229 | 0.9342 | **6.58%** |
| March 10, 4:05AM | 0.5571 | 0.3454 | 0.9025 | **9.75%** |

### Profit Factor Breakdown

- **Profit factor:** 1.035 (modest but consistent)
- Winning positions: 193 (49.9%), Losing positions: 194 (50.1%)
- Average win: $1,558.75, Average loss: $1,497.64
- The near-50/50 win rate is expected — profits come from the STRUCTURAL EDGE, not win rate

---

## 2. Execution Strategy: Maker-Dominant Limit Orders

### Taker vs Maker Split

| Execution Type | Trades | Percentage | Avg Price | Avg Size |
|----------------|--------|------------|-----------|----------|
| **Maker (limit)** | 196,658 | **90.2%** | 0.4542 | 23.2 |
| Taker (market) | 21,389 | 9.8% | 0.4967 | 64.7 |

**Key insight:** The wallet places limit orders overwhelmingly. Maker trades get filled at 4.25 cents cheaper on average. This is critical to achieving the sub-1.00 VWAP sum.

### Snapshot Price Advantage

- **58.8% of Up trades** bought below the snapshot mid-price (avg discount: 1.37 cents)
- **58.2% of Down trades** bought below the snapshot mid-price (avg discount: 1.38 cents)
- The wallet consistently fills below the visible market price by posting limit orders

### Order Book Context

- All trades fill **BETWEEN** the best bid and ask (100% of trades)
- Order book spreads are consistently wide (~0.98), meaning the wallet places orders inside the spread
- The wallet operates in a market with deep liquidity (~10K+ shares at top-of-book)

---

## 3. Trade Velocity & Accumulation Pattern

### Ultra-High Frequency Accumulation

| Metric | Value |
|--------|-------|
| Mean trades per market | 276 |
| Max trades in one market | 1,062 |
| Median time gap between trades | **0 seconds** (same-second) |
| Same-second trades | **82.3%** |
| Trades within 2 seconds | **94.4%** |
| Trade rate (busiest markets) | 260-300 trades/min |

**This is automated bot behavior.** The wallet fires hundreds of small limit orders per second, continuously accumulating on both sides throughout the market window.

### Small, Granular Order Sizes

- Mean trade size: 27.3 shares
- Median trade size: 12.2 shares
- The wallet splits capital into many small orders rather than few large ones — this is a market-making approach to avoid slippage

---

## 4. Timing Strategy

### Entry Window

| Time into Window | Trades | Percentage |
|-----------------|--------|------------|
| 0-60s | 58,073 | 26.6% |
| 60-120s | 56,044 | 25.7% |
| 120-180s | 48,774 | 22.4% |
| 180-240s | 49,813 | 22.8% |
| 240-300s (final min) | 5,327 | **2.4%** |

**The wallet trades throughout the first ~4 minutes** and almost completely stops in the final minute. Trading begins early (within the first 30 seconds, 12.3% of trades), peaks in the 30-90 second window, and tapers off after 240 seconds.

### No Sell, Hold to Resolution

- **ZERO sell trades** in the entire 218K trade dataset
- All positions are held until market resolution (5-minute expiry)
- Profit is realized at settlement, not through active trading/flipping

---

## 5. Directional Tilt: Mild Mean-Reversion

### RSI-Based Outcome Selection

| RSI Range | Chose Up | Chose Down | Interpretation |
|-----------|----------|------------|---------------|
| < 30 (oversold) | **57.9%** | 42.1% | Buys more Up |
| 30-40 | **55.6%** | 44.4% | Buys more Up |
| 40-50 | **52.4%** | 47.6% | Slight Up tilt |
| 50-60 | 47.9% | **52.1%** | Slight Down tilt |
| 60-70 | 46.3% | **53.7%** | Buys more Down |
| > 70 (overbought) | 45.9% | **54.1%** | Buys more Down |

**Clear mean-reversion signal:** Buys more Up when RSI is low, more Down when RSI is high.

### BTC Price Delta from Window Start

| BTC Direction | Chose Up | Chose Down | Interpretation |
|--------------|----------|------------|---------------|
| BTC moving UP | 46.0% | **54.0%** | Bets on reversal |
| BTC moving DOWN | **55.6%** | 44.4% | Bets on reversal |

- **Momentum-aligned:** 45.2% of trades
- **Mean-reversion:** 54.8% of trades
- When |delta| > 0.001: mean-reversion rate is **57.2%**

### Capital Allocation to Winner

- Mean capital on winning side: **58.6%**
- Markets with >50% capital on winner: **69.3%**
- Correctly heavier on winning side: **70.3%** of markets

**The directional tilt is modest but consistent.** The wallet doesn't need to be right every time — it just needs to allocate ~55-60% of capital to the correct side while maintaining the sub-1.00 VWAP sum.

---

## 6. Market Condition Awareness

### Volatility

- Mean 30m rolling vol: 0.000818
- The wallet trades across ALL volatility regimes — no evidence of volatility filtering
- No significant sizing adjustment based on volatility

### Sizing

- Sizing is remarkably consistent across all conditions
- No significant size-up on high-edge trades (contradicts edge-based sizing)
- Slightly larger sizes at higher prices (Q4-Q5 price quintiles: 31 avg vs Q1: 25 avg)

---

## 7. PnL Breakdown

### By Entry Price Bucket

| Avg Entry Price | Positions | PnL | Win Rate |
|----------------|-----------|-----|----------|
| 0.10-0.20 | 8 | -$986 | 12.5% |
| 0.20-0.30 | 53 | **-$38,788** | 3.8% |
| 0.30-0.40 | 73 | -$7,506 | 30.1% |
| 0.40-0.50 | 68 | -$15,410 | 36.8% |
| 0.50-0.60 | 83 | **+$22,924** | 67.5% |
| 0.60-0.70 | 69 | **+$34,020** | 81.2% |
| 0.70-0.80 | 28 | **+$16,101** | 96.4% |

**The wallet's profit comes from positions where it enters at 0.50-0.80** — these are the "winning side" of the dual-side strategy. Entries at 0.20-0.40 are the "hedging side" that loses but at lower cost because the entry prices are cheap.

### By Outcome Side

- **Down positions:** +$26,980 total PnL (51.8% win rate)
- **Up positions:** -$16,683 total PnL (47.9% win rate)
- Slight Down-side edge during this period, consistent with the mean-reversion behavior in a period where BTC was trending down

---

## 8. Strategy Summary: What to Copy

### The Formula

```
FOR each 5-minute BTC Up/Down market:
  1. START trading within the first 30 seconds
  2. PLACE limit orders on BOTH Up and Down sides simultaneously
  3. TARGET a combined VWAP (Up_price + Down_price) < 1.00
  4. TILT capital slightly toward mean-reversion:
     - If RSI < 50 or BTC delta < 0: allocate ~55% to Up
     - If RSI > 50 or BTC delta > 0: allocate ~55% to Down
  5. USE small order sizes (10-50 shares) at high frequency (~1 trade/second)
  6. EXECUTE 90%+ as limit orders (maker) for price improvement
  7. STOP entering in the final 60 seconds
  8. NEVER sell — hold all positions to resolution
  9. TARGET ~$3,500 total capital per market across both sides
```

### Critical Parameters

| Parameter | Value |
|-----------|-------|
| Order type | 90% limit (maker) |
| Trade size | 10-50 shares (median 12) |
| Trades per market | 200-300 |
| Capital per market | ~$3,500 |
| Entry window | 0s to ~240s into market |
| Stop entries | Last 60s |
| Mean-reversion strength | 55/45 split |
| Target VWAP sum | < 0.97 |
| Structural edge target | 3-4% per market |
| Hold strategy | To resolution (no sells) |

### Gap Analysis vs Your Bot

The key differentiators that likely separate this wallet from a typical directional bot:

1. **Dual-side coverage** — Most bots pick a direction. This wallet buys BOTH sides and profits from the spread, not prediction accuracy.
2. **Maker-dominant execution** — 90% limit orders. If your bot uses market orders, you're losing 4+ cents per trade in execution quality.
3. **Ultra-high frequency accumulation** — 200-300 trades per market, building the position gradually rather than single large entries.
4. **Structural arbitrage mindset** — The profit comes from buying both sides below $1.00 combined, not from being "right" about direction.
5. **Mild directional tilt** — Only 55/45, not aggressive. The tilt adds a few percent on top of the structural edge.
6. **No exits, no sells** — Removes emotional/timing risk. Hold to resolution and collect.
