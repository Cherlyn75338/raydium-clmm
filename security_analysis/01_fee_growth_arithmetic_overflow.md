# Fee Growth Arithmetic Overflow via wrapping_sub Operations

## Project: Raydium CLMM (Concentrated Liquidity Market Maker)

## Severity: Critical

## Category: Arithmetic Overflow / Integer Underflow

---

## 🔍 Description

A critical vulnerability exists in the Raydium CLMM protocol where `wrapping_sub` operations in fee and reward growth calculations can cause integer underflows, leading to massive overflow values. This vulnerability affects multiple core components of the AMM system and can be exploited to drain pool reserves.

The vulnerability manifests when calculating fee growth inside a position's tick range. The formula used is:
```
fee_growth_inside = fee_growth_global - fee_growth_below - fee_growth_above
```

When manipulated tick crossing sequences cause `fee_growth_global < (fee_growth_below + fee_growth_above)`, the `wrapping_sub` operation causes the result to wrap around to near `u128::MAX`, creating an artificially inflated fee growth value.

## 📜 Affected Code

### 1. tick_array.rs - get_fee_growth_inside()
```rust
// programs/amm/src/states/tick_array.rs:432-437
let fee_growth_inside_0_x64 = fee_growth_global_0_x64
    .wrapping_sub(fee_growth_below_0_x64)
    .wrapping_sub(fee_growth_above_0_x64);
let fee_growth_inside_1_x64 = fee_growth_global_1_x64
    .wrapping_sub(fee_growth_below_1_x64)
    .wrapping_sub(fee_growth_above_1_x64);
```

### 2. tick_array.rs - get_reward_growths_inside()
```rust
// programs/amm/src/states/tick_array.rs:473-476
reward_growths_inside[i] = reward_infos[i]
    .reward_growth_global_x64
    .wrapping_sub(reward_growths_below)
    .wrapping_sub(reward_growths_above);
```

### 3. personal_position.rs - update_rewards()
```rust
// programs/amm/src/states/personal_position.rs:174-175
let reward_growth_delta =
    reward_growth_inside.wrapping_sub(curr_reward_info.growth_inside_last_x64);
```

### 4. increase_liquidity.rs - calculate_latest_token_fees()
```rust
// programs/amm/src/instructions/increase_liquidity.rs:208-209
let fee_growth_delta =
    U128::from(fee_growth_inside_latest_x64.wrapping_sub(fee_growth_inside_last_x64))
```

### 5. swap.rs - Fee Growth Global Accumulator
```rust
// programs/amm/src/instructions/swap.rs:378-381
state.fee_growth_global_x64 = state
    .fee_growth_global_x64
    .checked_add(fee_growth_global_x64_delta)
    .unwrap();
```

## 🧠 Root Cause

The root cause is the use of **wrapping arithmetic** (`wrapping_sub`) instead of **saturating arithmetic** (`saturating_sub`) for critical fee and reward calculations. This design choice allows arithmetic underflows to wrap around to maximum values rather than being clamped to zero.

Key contributing factors:
1. **Unsafe arithmetic operations**: Using `wrapping_sub` for financial calculations
2. **Missing invariant checks**: No validation that `fee_growth_global >= fee_growth_below + fee_growth_above`
3. **Masking overflow effects**: The `to_underflow_u64()` function returns 0 for values > u64::MAX, hiding but not preventing the overflow
4. **Accumulator overflow**: The fee_growth_global accumulator can overflow in high-volume scenarios

## ⚠️ Exploitability

**Is this vulnerability exploitable?** **Yes**

### Attack Vector:

1. **Setup Phase**:
   - Attacker creates multiple positions across different tick ranges
   - Positions are strategically placed to manipulate fee growth snapshots

2. **Manipulation Phase**:
   - Execute a series of swaps that cross ticks in a specific sequence
   - Manipulate the pool state so that:
     - `fee_growth_below` increases significantly
     - `fee_growth_above` increases significantly
     - `fee_growth_global` remains relatively lower

3. **Exploitation Phase**:
   - When decreasing liquidity, the calculation:
     ```rust
     fee_growth_inside = fee_growth_global.wrapping_sub(fee_growth_below).wrapping_sub(fee_growth_above)
     ```
   - Results in a massive overflow value (near u128::MAX)
   - This inflated fee_growth_inside value is used to calculate fees owed:
     ```rust
     fee_growth_delta = fee_growth_inside_latest.wrapping_sub(fee_growth_inside_last)
     amount_owed = (fee_growth_delta * liquidity) / Q64
     ```

4. **Fund Extraction**:
   - The artificially inflated fees allow the attacker to withdraw more tokens than legitimately earned
   - This can drain the pool's token vaults

### Proof of Concept Elements:

```rust
// Simplified exploitation scenario
let fee_growth_global = 1000;
let fee_growth_below = 800;
let fee_growth_above = 300;

// Normal calculation would be: 1000 - 800 - 300 = -100 (invalid)
// With wrapping_sub: 1000.wrapping_sub(800).wrapping_sub(300) = u128::MAX - 99

// This massive value is then used to calculate fees owed to the position
```

## 💥 Impact

### Direct Impact:
- **Direct theft of pool funds**: Attackers can drain token vaults by claiming inflated fees
- **Permanent loss of liquidity provider funds**: LPs lose their deposited assets
- **Protocol insolvency**: Multiple pools can be drained, making the protocol insolvent

### Secondary Impact:
- **Loss of user trust**: Critical vulnerability undermines protocol credibility
- **Cascading liquidations**: Drained pools affect dependent positions and strategies
- **Market manipulation**: Attackers can manipulate prices while executing the exploit

### Classification: **Critical** - Direct theft of any user funds

## ✅ Remediation Recommendations

### Immediate Fixes:

1. **Replace wrapping_sub with saturating_sub**:
```rust
// tick_array.rs - get_fee_growth_inside()
let fee_growth_inside_0_x64 = fee_growth_global_0_x64
    .saturating_sub(fee_growth_below_0_x64)
    .saturating_sub(fee_growth_above_0_x64);

// personal_position.rs - update_rewards()
let reward_growth_delta = 
    reward_growth_inside.saturating_sub(curr_reward_info.growth_inside_last_x64);

// increase_liquidity.rs - calculate_latest_token_fees()
let fee_growth_delta =
    U128::from(fee_growth_inside_latest_x64.saturating_sub(fee_growth_inside_last_x64))
```

2. **Add invariant validation**:
```rust
pub fn get_fee_growth_inside(...) -> Result<(u128, u128)> {
    // ... existing code ...
    
    // Validate invariant
    let total_outside = fee_growth_below_0_x64
        .checked_add(fee_growth_above_0_x64)
        .ok_or(ErrorCode::ArithmeticOverflow)?;
    
    require!(
        fee_growth_global_0_x64 >= total_outside,
        ErrorCode::InvalidFeeGrowthInvariant
    );
    
    // Safe calculation
    let fee_growth_inside_0_x64 = fee_growth_global_0_x64
        .checked_sub(fee_growth_below_0_x64)
        .and_then(|v| v.checked_sub(fee_growth_above_0_x64))
        .ok_or(ErrorCode::ArithmeticOverflow)?;
    
    // ... similar for token_1 ...
}
```

3. **Implement checked arithmetic for accumulators**:
```rust
// swap.rs - fee growth accumulator
state.fee_growth_global_x64 = state
    .fee_growth_global_x64
    .checked_add(fee_growth_global_x64_delta)
    .ok_or(ErrorCode::FeeGrowthOverflow)?;
```

### Long-term Improvements:

1. **Comprehensive arithmetic safety policy**:
   - Use `checked_*` operations for all financial calculations
   - Use `saturating_*` operations where overflow behavior is acceptable
   - Never use `wrapping_*` operations for financial values

2. **Add circuit breakers**:
   - Implement maximum fee growth limits per block/epoch
   - Add emergency pause functionality when anomalies detected

3. **Enhanced monitoring**:
   - Track fee growth rates and flag anomalies
   - Monitor for unusual liquidity withdrawal patterns

## 🔁 Related Issues

1. **Protocol Position Vulnerability**: Similar wrapping_sub usage in protocol_position.rs may have comparable issues
2. **Reward Distribution**: The reward growth calculations use the same vulnerable pattern
3. **Cross-tick Manipulation**: The tick crossing logic in swap.rs enables the attack vector

## 🧪 Test Cases

### Test 1: Basic Overflow Scenario
```rust
#[test]
fn test_fee_growth_overflow_exploit() {
    // Setup pool with initial liquidity
    let mut pool = setup_test_pool();
    
    // Create attacker position
    let position = create_position(tick_lower: -1000, tick_upper: 1000);
    
    // Manipulate fee growth values
    execute_swaps_to_increase_outside_growth(&mut pool);
    
    // Attempt to exploit
    let fees_before = get_vault_balance(&pool);
    decrease_liquidity(&position, liquidity_delta: position.liquidity);
    let fees_after = get_vault_balance(&pool);
    
    // Verify exploit prevented
    assert!(fees_after - fees_before <= legitimate_fee_amount);
}
```

### Test 2: Invariant Validation
```rust
#[test]
fn test_fee_growth_invariant() {
    let fee_growth_global = 1000u128;
    let fee_growth_below = 800u128;
    let fee_growth_above = 300u128;
    
    // Should fail with InvalidFeeGrowthInvariant
    let result = get_fee_growth_inside(
        fee_growth_global,
        fee_growth_below,
        fee_growth_above
    );
    
    assert_eq!(result.unwrap_err(), ErrorCode::InvalidFeeGrowthInvariant);
}
```

### Test 3: Accumulator Overflow Protection
```rust
#[test]
fn test_fee_growth_accumulator_overflow() {
    let mut state = SwapState {
        fee_growth_global_x64: u128::MAX - 100,
        // ... other fields
    };
    
    let fee_growth_delta = 200u128;
    
    // Should fail with FeeGrowthOverflow
    let result = state.add_fee_growth(fee_growth_delta);
    assert_eq!(result.unwrap_err(), ErrorCode::FeeGrowthOverflow);
}
```

### Integration Test:
```rust
#[test]
fn test_complete_attack_scenario() {
    // 1. Deploy vulnerable contract
    // 2. Setup attacker accounts and positions
    // 3. Execute manipulation transactions
    // 4. Attempt exploitation
    // 5. Verify funds cannot be drained
}
```

---

## Conclusion

This critical vulnerability in the Raydium CLMM protocol represents a severe risk to user funds and protocol integrity. The use of wrapping arithmetic in financial calculations, combined with missing invariant checks, creates an exploitable attack vector that can drain pool reserves. Immediate remediation through the replacement of `wrapping_sub` with `saturating_sub` or `checked_sub` operations, along with proper invariant validation, is essential to protect user funds and maintain protocol security.