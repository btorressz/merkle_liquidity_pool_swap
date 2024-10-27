use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint}; // Add SPL token support
use anchor_lang::solana_program::keccak::{hashv};

declare_id!("6RSYJVrYn1fy1LXwXuyz2A9REuzyajwNdLRfpUcSvbY5");

#[program]
pub mod merkle_liquidity_pool_swap {
    use super::*;

    // Initialize the liquidity pool with an initial Merkle root and mint SPL tokens for LPs
    pub fn initialize_pool(
        ctx: Context<InitializePool>, 
        initial_root: [u8; 32], 
        mint_lp_token: Pubkey
    ) -> Result<()> {
        let pool = &mut ctx.accounts.pool;
        pool.token_a_balance = 0;
        pool.token_b_balance = 0;
        pool.merkle_root = initial_root; // Store the initial Merkle root
        pool.mint_lp_token = mint_lp_token; // Store the LP token mint address
        pool.in_progress = false; // Initialize reentrancy protection
        pool.swap_fee = 30; // 0.3% default fee
        Ok(())
    }

    // Swap tokens using the liquidity pool and verify the user's LP status using a Merkle proof
    pub fn swap_tokens(
        ctx: Context<SwapTokens>,
        amount_in: u64,
        proof: Vec<[u8; 32]>,
        root: [u8; 32],
    ) -> Result<()> {
        // Borrow the pool authority before mutably borrowing the pool
        let pool_authority = ctx.accounts.pool.to_account_info().clone();

        // Now we can safely mutably borrow the pool
        let pool = &mut ctx.accounts.pool;

        // Reentrancy protection
        require!(!pool.in_progress, CustomError::ReentrancyGuardActive);
        pool.in_progress = true;

        // Hash the user's public key and the amount they wish to swap
        let user_hash = hashv(&[
            &ctx.accounts.user.key().to_bytes(),
            &amount_in.to_le_bytes(),
        ]);
        
        // Verify the provided Merkle proof
        require!(
            verify_proof(user_hash.to_bytes(), proof, root),
            CustomError::InvalidMerkleProof
        );

        // Calculate the swap ratio (token B balance divided by token A balance)
        let swap_ratio = calculate_swap_ratio(pool.token_a_balance, pool.token_b_balance);
        let amount_out = (amount_in as f64 * swap_ratio) as u64;

        // Apply swap fee
        let fee_amount = amount_in * pool.swap_fee / 10000; // e.g., 0.3% fee
        pool.fee_accumulation += fee_amount;

        // Adjust the liquidity pool balances
        pool.token_a_balance += amount_in - fee_amount;
        pool.token_b_balance -= amount_out;

        // Transfer the swapped tokens using SPL token transfers
        let cpi_accounts = Transfer {
            from: ctx.accounts.token_account_a.to_account_info(),
            to: ctx.accounts.token_account_b.to_account_info(),
            authority: pool_authority, // Use the cloned pool authority here
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, amount_in)?;

        // End reentrancy protection
        pool.in_progress = false;

        Ok(())
    }

    // Allow LPs to claim their liquidity by submitting a Merkle proof
    pub fn claim_liquidity(
        ctx: Context<ClaimLiquidity>,
        proof: Vec<[u8; 32]>,
        root: [u8; 32],
        amount: u64,
    ) -> Result<()> {
        // Borrow the pool authority before mutably borrowing the pool
        let pool_authority = ctx.accounts.pool.to_account_info().clone();

        // Now we can safely mutably borrow the pool
        let pool = &mut ctx.accounts.pool;

        // Reentrancy protection
        require!(!pool.in_progress, CustomError::ReentrancyGuardActive);
        pool.in_progress = true;

        // Hash the user's public key and their contribution
        let user_hash = hashv(&[
            &ctx.accounts.user.key().to_bytes(),
            &amount.to_le_bytes(),
        ]);

        // Verify the Merkle proof for the user's contribution
        require!(
            verify_proof(user_hash.to_bytes(), proof, root),
            CustomError::InvalidMerkleProof
        );

        // Calculate the user's share of the pool
        let user_share = calculate_user_share(pool.token_a_balance, amount);
        pool.token_a_balance -= user_share;

        // Transfer tokens to the LP
        let cpi_accounts = Transfer {
            from: ctx.accounts.pool_token_account.to_account_info(),
            to: ctx.accounts.user_token_account.to_account_info(),
            authority: pool_authority, // Use the cloned pool authority here
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, user_share)?;

        // End reentrancy protection
        pool.in_progress = false;

        Ok(())
    }

    // Allow LPs to partially withdraw their liquidity
    pub fn partial_withdraw(
        ctx: Context<PartialWithdraw>, 
        proof: Vec<[u8; 32]>, 
        root: [u8; 32], 
        withdraw_amount: u64
    ) -> Result<()> {
        // Borrow the pool authority before mutably borrowing the pool
        let pool_authority = ctx.accounts.pool.to_account_info().clone();

        // Now we can safely mutably borrow the pool
        let pool = &mut ctx.accounts.pool;

        // Reentrancy protection
        require!(!pool.in_progress, CustomError::ReentrancyGuardActive);
        pool.in_progress = true;

        // Hash the user's public key and the amount they wish to withdraw
        let user_hash = hashv(&[
            &ctx.accounts.user.key().to_bytes(),
            &withdraw_amount.to_le_bytes(),
        ]);

        // Verify the Merkle proof
        require!(
            verify_proof(user_hash.to_bytes(), proof, root),
            CustomError::InvalidMerkleProof
        );

        // Calculate the withdrawal amount and update pool balance
        pool.token_a_balance -= withdraw_amount;

        // Transfer the partial amount to the user
        let cpi_accounts = Transfer {
            from: ctx.accounts.pool_token_account.to_account_info(),
            to: ctx.accounts.user_token_account.to_account_info(),
            authority: pool_authority, // Use the cloned pool authority here
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, withdraw_amount)?;

        // End reentrancy protection
        pool.in_progress = false;

        Ok(())
    }

    // Emergency withdrawal with penalty
    pub fn emergency_withdraw(
        ctx: Context<PartialWithdraw>, 
        proof: Vec<[u8; 32]>, 
        root: [u8; 32], 
        withdraw_amount: u64
    ) -> Result<()> {
        // Borrow the pool authority before mutably borrowing the pool
        let pool_authority = ctx.accounts.pool.to_account_info().clone();

        // Now we can safely mutably borrow the pool
        let pool = &mut ctx.accounts.pool;

        // Reentrancy protection
        require!(!pool.in_progress, CustomError::ReentrancyGuardActive);
        pool.in_progress = true;

        // Hash the user's public key and the amount they wish to withdraw
        let user_hash = hashv(&[
            &ctx.accounts.user.key().to_bytes(),
            &withdraw_amount.to_le_bytes(),
        ]);

        // Verify the Merkle proof
        require!(
            verify_proof(user_hash.to_bytes(), proof, root),
            CustomError::InvalidMerkleProof
        );

        // Calculate penalty for early withdrawal (10% penalty)
        let penalty = withdraw_amount * 10 / 100;
        let amount_after_penalty = withdraw_amount - penalty;

        // Adjust pool balance and transfer remaining amount to the user
        pool.token_a_balance -= amount_after_penalty;

        let cpi_accounts = Transfer {
            from: ctx.accounts.pool_token_account.to_account_info(),
            to: ctx.accounts.user_token_account.to_account_info(),
            authority: pool_authority, // Use the cloned pool authority here
        };
        let cpi_program = ctx.accounts.token_program.to_account_info();
        let cpi_ctx = CpiContext::new(cpi_program, cpi_accounts);
        token::transfer(cpi_ctx, amount_after_penalty)?;

        // End reentrancy protection
        pool.in_progress = false;

        Ok(())
    }

    // Governance function to update the Merkle root on-chain after LPs add/remove liquidity
    pub fn update_merkle_root(ctx: Context<UpdateMerkleRoot>, new_root: [u8; 32]) -> Result<()> {
        let pool = &mut ctx.accounts.pool;
        pool.merkle_root = new_root;
        Ok(())
    }

    // Governance function to allow LPs to vote on changing pool parameters (e.g., fees)
    pub fn vote_on_pool_parameters(ctx: Context<VoteOnPoolParameters>, new_fee: u64) -> Result<()> {
        let pool = &mut ctx.accounts.pool;
        pool.swap_fee = new_fee; // Change the fee based on governance vote
        Ok(())
    }

    // Function to add time-locks or vesting for LPs
    pub fn lock_liquidity(ctx: Context<LockLiquidity>, lock_time: i64) -> Result<()> {
        let pool = &mut ctx.accounts.pool;
        let clock = Clock::get()?;
        pool.lock_until = clock.unix_timestamp + lock_time; // Lock LP's liquidity until a certain timestamp
        Ok(())
    }

    // Simulated rebalancing based on external factors
    pub fn rebalance_pool(ctx: Context<RebalancePool>) -> Result<()> {
        let pool = &mut ctx.accounts.pool;
        let price_adjustment = adjust_pool_ratio_based_on_external_factors();
        pool.token_a_balance = (pool.token_a_balance as f64 * price_adjustment) as u64;

        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializePool<'info> {
    #[account(init, payer = user, space = 8 + 128)] 
    pub pool: Account<'info, Pool>,               
    #[account(mut)]
    pub user: Signer<'info>,                      
    pub system_program: Program<'info, System>,   
    pub mint_lp_token: Account<'info, Mint>,       // SPL Token Mint for LP tokens
}

#[derive(Accounts)]
pub struct SwapTokens<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,               
    #[account(mut)]
    pub user: Signer<'info>,                      
    #[account(mut)]
    pub token_account_a: Account<'info, TokenAccount>, // Token A account of user
    #[account(mut)]
    pub token_account_b: Account<'info, TokenAccount>, // Token B account of user
    pub token_program: Program<'info, Token>,      // Token program to handle SPL token transfers
}

#[derive(Accounts)]
pub struct ClaimLiquidity<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,                
    #[account(mut)]
    pub user: Signer<'info>,                       
    #[account(mut)]
    pub pool_token_account: Account<'info, TokenAccount>, // Token account of pool
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>, // Token account of user
    pub token_program: Program<'info, Token>,      
}

#[derive(Accounts)]
pub struct PartialWithdraw<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,                
    #[account(mut)]
    pub user: Signer<'info>,                       
    #[account(mut)]
    pub pool_token_account: Account<'info, TokenAccount>, 
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>, 
    pub token_program: Program<'info, Token>,      
}

#[derive(Accounts)]
pub struct UpdateMerkleRoot<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,                
    pub user: Signer<'info>,                      
}

#[derive(Accounts)]
pub struct VoteOnPoolParameters<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,               
    pub user: Signer<'info>,                      
}

#[derive(Accounts)]
pub struct LockLiquidity<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,                
    pub user: Signer<'info>,                      
}

#[derive(Accounts)]
pub struct RebalancePool<'info> {
    #[account(mut)]
    pub pool: Account<'info, Pool>,
}

// Pool data structure
#[account]
pub struct Pool {
    pub token_a_balance: u64,                      
    pub token_b_balance: u64,                      
    pub merkle_root: [u8; 32],                     
    pub swap_fee: u64,                             
    pub lock_until: i64,                          
    pub mint_lp_token: Pubkey,
    pub in_progress: bool,                        // Reentrancy guard
    pub fee_accumulation: u64,                     // Accumulated fees for LPs
}

// Helper function to calculate the swap ratio between the tokens in the pool
fn calculate_swap_ratio(token_a_balance: u64, token_b_balance: u64) -> f64 {
    (token_b_balance as f64) / (token_a_balance as f64)
}

// Helper function to verify the Merkle proof
fn verify_proof(leaf: [u8; 32], proof: Vec<[u8; 32]>, root: [u8; 32]) -> bool {
    let mut hash = leaf;
    for p in proof {
        hash = if hash < p {
            hashv(&[&hash, &p]).to_bytes()
        } else {
            hashv(&[&p, &hash]).to_bytes()
        };
    }
    hash == root
}

// Helper function to calculate the user's share of the pool based on their contribution
fn calculate_user_share(pool_balance: u64, user_contribution: u64) -> u64 {
    (pool_balance as f64 * (user_contribution as f64 / 100.0)) as u64
}

// Simulate external factors (for dynamic rebalancing)
fn adjust_pool_ratio_based_on_external_factors() -> f64 {
    // In production, i will use an oracle for price data, e.g., Pyth
    // Simulating an arbitrary price increase factor
    1.05 // 5% price increase for simulation
}

// Custom Error for reentrancy guard and Merkle proof validation
#[error_code]
pub enum CustomError {
    #[msg("Reentrancy protection active")]
    ReentrancyGuardActive,
    #[msg("Invalid Merkle proof")]
    InvalidMerkleProof,
}
