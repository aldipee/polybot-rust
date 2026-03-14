# Sprint 4 Consultant Questionnaire

## Purpose

This questionnaire is for a focused consultation on the `Sprint 4` wallet-clone objective.

The goal is **not** to discuss generic trading ideas.
The goal is to extract concrete, implementation-grade rules for a bot that behaves like a:

1. two-sided BTC 5-minute pair-pricing / inventory-building strategy
2. maker-first, `BUY`-only normal-flow accumulator
3. hold-to-settlement system with only mild and carefully controlled directional skew

## Working Diagnosis

Our current diagnosis of the target trader is:

1. the real edge is **cheap two-sided execution**
2. the strategy is **not** primarily a directional BTC prediction strategy
3. the strategy should trade both sides almost every market
4. the strategy should open both sides early
5. the strategy should stay mostly passive
6. the strategy should hold to settlement
7. the directional overlay, if any, should be secondary and tightly controlled

## What We Need From The Consultant

Please answer in **rule form**, not broad opinion.

For each question, please give:

1. the direct rule
2. the threshold or numeric band if one exists
3. any explicit exception
4. whether the rule should be:
   - `hard`
   - `soft`
   - `informational only`

If a rule should vary by time in the 5-minute window, say exactly how.

---

## Section A: Core Objective

1. Is the correct core objective exactly this:
   - build a cheap two-sided book
   - keep the residual tail small
   - hold to settlement
   and **not**:
   - finish exactly equal every market
   - predict BTC direction first?

2. What is the primary end-state metric the bot should optimize:
   - `min(qYES, qNO) - total_cost`
   - `combined_avg_paid`
   - both together
   - something else?

3. What is the right pass/fail market-level objective:
   - positive worst-case settlement floor
   - positive expected average across many markets
   - small tail with acceptable downside
   - some combination?

4. If the bot must choose between:
   - higher participation
   - better pair cost
   which should dominate?

---

## Section B: Market Participation And Startup

5. Should the bot follow this hard rule:
   - trade both sides or skip the market entirely
   - never accept normal one-sided participation?

6. What are the exact skip conditions for a market?
   Please specify hard skip rules such as:
   - quote quality
   - spread
   - missing edge
   - pair too expensive
   - insufficient depth
   - timing too late

7. How fast should both sides be live after market open?
   Please give target thresholds for:
   - median target
   - acceptable upper bound
   - failure threshold

8. If one side fills and the other does not, should missing-side startup repair override all normal shape logic until both sides exist?

9. During startup completion, how far is the bot allowed to chase the missing side before it should stop or downgrade size?

---

## Section C: Price Quality Rules

10. Should optional buys require the paid price to be **below same-side snapshot price**?
    If yes, how strict should that be:
    - any improvement
    - at least 1 tick better
    - some basis-point or cents threshold

11. Should `edge_model_minus_price < 0` be a hard no-trade rule for optional adds?

12. If yes, should startup completion be exempt from that rule?

13. At what positive edge level should size-up begin?
    Examples:
    - `>= 0`
    - `>= 0.02`
    - `>= 0.05`
    - some other threshold

14. If price is below snapshot but model edge is weak, should the bot:
    - still trade
    - trade smaller
    - skip entirely

---

## Section D: Combined Pair Cost Rules

15. Are these pair-cost regimes directionally correct?

1. `< 0.90` = very strong
2. `0.90 - 0.94` = strong
3. `0.94 - 0.96` = good
4. `0.96 - 0.98` = acceptable
5. `0.98 - 1.00` = cautious
6. `1.00 - 1.02` = stop optional adds
7. `> 1.02` = strong skip / bad market

16. Which pair-cost measure should drive the bot in normal flow:
   - current held-book combined cost
   - projected post-add combined cost
   - both

17. Should optional paired growth be blocked whenever projected post-add paired cost goes above a specific threshold?
    If yes, what threshold?

18. Should required lighter-side repair still be allowed when the projected repaired book is above that threshold, as long as it improves the current book?

---

## Section E: Skew And Directional Overlay

19. Should the default normal-flow posture be:
   - nearly balanced
   - only mild skew
   - never large directional bets?

20. If extra size is allowed, should the default bias be toward the **higher-priced side**, not the lower-priced side?

21. Under what exact conditions, if any, should the bot intentionally overweight the lower-priced side?

22. After the first minute, should directional adds only be allowed when they agree with the sign of `binance_delta_from_start`?

23. What is the maximum acceptable skew:
   - during normal flow
   - late in the market
   - at expiry

Please answer in measurable terms, for example:

1. `share_skew_ratio`
2. absolute share tail
3. tail as a percent of paired size
4. worst-case settlement loss

---

## Section F: Sizing And Clip Ladder

24. What should the clip ladder look like for a wallet-like implementation?
    Please answer separately for:
    - opener
    - startup completion
    - normal paired growth
    - lighter-side repair

25. Should the bot preserve a hard per-fill ceiling similar to the observed `80` share cap?

26. Should higher-priced sides receive larger clips on average than lower-priced sides?

27. When repairing the lighter side, should the bot use:
   - fixed clips
   - exact gap
   - max affordable clip
   - smallest valid repair above exchange minimum notional

28. If exchange minimum notional blocks a repair, should the bot automatically size up to the smallest valid repair instead of retrying invalid tiny orders?

---

## Section G: Asymmetry And Cleanup

29. When one leg of a paired submit is live and the opposite leg rejects or never becomes live, how fast should the surviving leg be canceled?

30. Should this asymmetric cleanup timeout be shorter than the normal stale-order timeout for healthy live orders?

31. Should the bot preserve a good opposite-side live order during lighter-side repair, or should lighter-side repair take full ownership and cancel the opposite leg first?

32. Should heavy-side growth be blocked whenever the remaining budget is no longer enough to fund the likely lighter-side repair?

---

## Section H: Late-Market Behavior

33. When should normal growth materially slow down:
   - after 180 seconds
   - after 210 seconds
   - after 240 seconds
   - some other threshold?

34. In the final minute, should the bot allow only actions that improve the worse-side settlement floor?

35. Should the bot stop all optional adds in the final 30 seconds?

36. Should late repair be allowed if it reduces the tail but slightly worsens average paired cost?

---

## Section I: Execution Style

37. Should normal flow remain strictly:
   - maker-first
   - `BUY`-only
   - hold-to-settlement

38. Are intramarket sells ever part of the target clone behavior, or should they remain emergency-only?

39. Should taker use be limited strictly to:
   - emergency risk exit
   - terminal cleanup
   - never normal accumulation

---

## Section J: Acceptance Criteria

40. What exact metrics should define "wallet-like enough"?

Please rank the importance of:

1. both-side participation rate
2. first-fill timing
3. time-to-second-side
4. fills per market
5. maker share
6. below-snapshot fill rate
7. positive-edge fill rate
8. combined paired cost
9. settlement floor
10. tail size at expiry
11. activity before 240 seconds
12. near-zero activity in the final 30 seconds

41. What concrete thresholds should we use for a canary pass?

Please give a recommended pass line for:

1. both-side participation
2. median time-to-both-sides
3. maker share
4. combined paired cost
5. maximum acceptable tail
6. minimum acceptable worst-case settlement floor

---

## Final Summary Request

Please end the consultation with:

1. the **top 5 hard rules** the bot must follow
2. the **top 5 soft rules** the bot should usually follow
3. the **top 3 things the current clone is most likely doing wrong**
4. the **single most important rule** for making the strategy profitable over many markets
