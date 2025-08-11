# Global Fee Growth Accumulator Overflow Risk in swap.rs

## Project: Raydium CLMM

## Severity: Critical (if unchecked overflow allowed) → Present code uses checked_add; impact downgraded to Medium/High for denial-of-service risk if panics occur in production builds without overflow checks.

## Category: Arithmetic

---

## 🔍 Description
The swap loop updates the global fee growth accumulator `fee_growth_global_x64` per step. If this uses unchecked arithmetic, it can wrap and corrupt accounting, potentially enabling theft. In the current code, the accumulator is updated with `checked_add`, which will panic on overflow. If release builds were configured to disable checks or if a variant used `wrapping_add`, this would become Critical. As written, the main risk is a denial-of-service due to panics if the accumulator ever reaches near-`u128::MAX`.

## 📜 Affected Code
```rust
// programs/amm/src/instructions/swap.rs
let fee_growth_global_x64_delta = U128::from(step.fee_amount)
    .mul_div_floor(U128::from(fixed_point_64::Q64), U128::from(state.liquidity))
    .unwrap()
    .as_u128();

state.fee_growth_global_x64 = state
    .fee_growth_global_x64
    .checked_add(fee_growth_global_x64_delta)
    .unwrap();
```

## 🧠 Root Cause
- Potential overflow of cumulative per-liquidity fee growth if run for extremely long periods or in manipulated scenarios.
- Reliance on `checked_add(...).unwrap()` can cause panics and halt transactions if accumulator nears `u128::MAX`.

## ⚠️ Exploitability
- Is this vulnerability exploitable? No direct fund theft under current implementation.
- With `checked_add`, the failure mode is a panic (transaction revert) rather than wraparound. This can be abused to cause DoS if an attacker can drive fees and growth sufficiently high (unlikely in practice but theoretically possible over long timelines).

## 💥 Impact
- For smart contracts: Medium/High (DoS)
  - Transactions that update fee growth would fail once the accumulator approaches `u128::MAX`, blocking swaps and position operations.

## ✅ Remediation Recommendations
- Keep `checked_add` to prevent wraparound.
- Add guardrails:
  - Enforce upper bounds and/or periodic normalization of `fee_growth_global_*_x64` (e.g., compress epochs by snapshotting and rebasing per-position baselines) to keep values comfortably below `u128::MAX`.
  - Emit warnings/metrics when approaching a safety threshold.
- Consider using saturating math plus invariant checks if you prefer not to panic, but ensure accounting remains conservative and non-crediting.

## 🔁 Related Issues (if any)
- The inside-growth underflow issue interacts with this accumulator: although the global accumulator is safe from wraparound, underflow inside computations can still zero out LP accruals. See 01-fee-reward-growth-wrapping-underflow.

## 🧪 Test Cases
- Fuzz/property test the swap loop to simulate very high sustained fee accrual and verify that `checked_add` prevents wrap but would panic near bounds.
- Add monitoring tests that assert values remain below configured safety margins under realistic operation.