# Fee/Reward Growth Underflow via wrapping_sub in Inside Calculations

## Project: Raydium CLMM

## Severity: High

## Category: Arithmetic, Logic Error

---

## 🔍 Description
The program computes fee and reward growth “inside” a position range using the formula:

- fee_growth_inside = fee_growth_global − fee_growth_below − fee_growth_above
- reward_growth_inside = reward_growth_global − reward_growth_below − reward_growth_above

In several places, these differences are performed using `wrapping_sub`. If the arithmetic relation is violated at runtime (e.g., due to tick crossing sequences and the relative meaning of “outside” values), `wrapping_sub` will underflow and produce a near-`u128::MAX` value rather than clamping to zero or returning an error. This value then propagates into fee/reward delta computations for positions.

Because downstream conversions use a helper that converts over-large values to `0` (`to_underflow_u64()`), this underflow does not panic but instead silently zeroes out fee/reward deltas. The practical effect is loss or freezing of unclaimed yield for LPs, which is a High severity issue.

## 📜 Affected Code
```rust
// programs/amm/src/states/tick_array.rs
// get_fee_growth_inside: underflows if global < below + above
let fee_growth_inside_0_x64 = fee_growth_global_0_x64
    .wrapping_sub(fee_growth_below_0_x64)
    .wrapping_sub(fee_growth_above_0_x64);
let fee_growth_inside_1_x64 = fee_growth_global_1_x64
    .wrapping_sub(fee_growth_below_1_x64)
    .wrapping_sub(fee_growth_above_1_x64);
```

```rust
// programs/amm/src/states/tick_array.rs
// get_reward_growths_inside: same pattern
reward_growths_inside[i] = reward_infos[i]
    .reward_growth_global_x64
    .wrapping_sub(reward_growths_below)
    .wrapping_sub(reward_growths_above);
```

```rust
// programs/amm/src/states/personal_position.rs
// update_rewards: wrapping_sub on reward growth delta
let reward_growth_delta =
    reward_growth_inside.wrapping_sub(curr_reward_info.growth_inside_last_x64);
```

```rust
// programs/amm/src/instructions/increase_liquidity.rs
// calculate_latest_token_fees: wrapping_sub on fee growth delta
let fee_growth_delta =
    U128::from(fee_growth_inside_latest_x64.wrapping_sub(fee_growth_inside_last_x64))
        .mul_div_floor(U128::from(liquidity), U128::from(fixed_point_64::Q64))
        .unwrap()
        .to_underflow_u64();
```

## 🧠 Root Cause
- Use of `wrapping_sub` allows negative intermediate results to wrap to large `u128` values.
- Mixed arithmetic semantics: parts of the computation use `checked_sub` while the final inside calculation uses `wrapping_sub`, breaking the invariant that “inside” should never be negative.
- Lack of an explicit invariant enforcing `fee_growth_inside >= 0` and `reward_growth_inside >= 0` in the presence of relative “outside” counters.

## ⚠️ Exploitability
- Is this vulnerability exploitable? Yes

How it can be exploited:
- An attacker induces tick crossing sequences with small “dust” swaps to manipulate the relative `fee_growth_outside` values at the lower/upper ticks such that:
  - `fee_growth_below + fee_growth_above > fee_growth_global` at the instant of computation, even though each individual term was derived with safe `checked_sub` paths.
- When the position owner increases/decreases liquidity or updates rewards, the code computes deltas using `wrapping_sub`, yielding a very large `u128` delta which, after scaling, is truncated to `0` by `to_underflow_u64()`.
- Net effect: the position’s newly accrued fee/reward delta becomes `0` instead of the correct positive value. Repeating this around position updates can repeatedly zero out accruals, denying yield.

Constraints and observations:
- Due to `to_underflow_u64()` returning `0` on overflow, the bug does not directly credit an attacker with outsized fees. Instead, it causes loss/freezing of the victim’s newly accrued fees/rewards.
- Global fee growth is updated using `checked_add`, so attempts to create wraparound credits via global accumulators are prevented by panics; the loss occurs locally in per-position delta computation.

## 💥 Impact
- For smart contracts: High
  - Freezing or loss of unclaimed yield for LPs (victims cannot claim the correct fees/rewards accrued since last update).
  - Does not directly enable draining vault balances, but results in material economic loss to affected LPs.

## ✅ Remediation Recommendations
- Replace all `wrapping_sub` in fee/reward inside and delta calculations with `saturating_sub`:
  - `tick_array.rs::get_fee_growth_inside`
  - `tick_array.rs::get_reward_growths_inside`
  - `personal_position.rs::update_rewards`
  - `increase_liquidity.rs::calculate_latest_token_fees`
- Add invariant validations before computing deltas:
  - If `global < below + above`, clamp inside growth to zero and log an error/metric.
- Consider adding property-based tests that assert inside growth is never negative and that deltas are monotonic for a fixed position unless liquidity changes.

## 🔁 Related Issues (if any)
- `protocol_position.rs` already uses `saturating_sub` for fee deltas, which is the correct pattern and should be mirrored elsewhere.
- The `to_underflow_u64()` helper silently converts large intermediate values to `0`, masking the bug at runtime. See separate report on this anti-pattern.

## 🧪 Test Cases
- Unit tests for underflow in inside computations:
  - Construct ticks where `fee_growth_outside` at both lower and upper artificially exceed the global value when summed.
  - Verify that with `wrapping_sub`, inside growth underflows; with `saturating_sub`, inside growth is clamped to `0`.
- Integration tests:
  - Create a position, accrue fees by swapping back and forth to cross boundaries, then call increase/decrease liquidity. Assert that fee/reward deltas are positive and match expected values with `saturating_sub`.
  - Property-test sequences of tick crossings to ensure deltas never decrease without liquidity changes and never wrap.