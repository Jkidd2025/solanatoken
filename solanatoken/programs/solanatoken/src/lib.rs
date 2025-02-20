use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount};
use anchor_spl::associated_token::AssociatedToken;
use std::collections::HashMap;
use pyth_sdk_solana::{load_price_feed_from_account_info, PriceStatus};

declare_id!("7MCEfe5NNGmfv2TiGDthDPF5T4TrsWFLRHAA5WMC7sTo");

// Token configuration constants
pub mod token_config {
    pub const NAME: &str = "Next Gen Crypto";
    pub const SYMBOL: &str = "NGC";
    pub const DECIMALS: u8 = 6;
    pub const TOTAL_SUPPLY: u64 = 1_000_000_000_000_000; // 1 billion with 6 decimals
    pub const REWARDS_RATE: u64 = 500; // 5% annual rewards rate (basis points)
    pub const MIN_HOLDING_PERIOD: i64 = 2_592_000; // 30 days in seconds
    pub const TRANSFER_COOLDOWN: i64 = 300; // 5 minutes in seconds
    
    // Transaction limits
    pub const MIN_PURCHASE_USD: u64 = 5000; // $50.00 in cents
    pub const MAX_TRANSACTION_SIZE: u64 = 1_000_000_000_000; // 1% of total supply
    pub const MAX_DAILY_TRANSACTIONS: u64 = 10;
    pub const PYTH_PRICE_FEED: &str = "Gv2NQnFfSQgzqFoGGm4bFX5q6oBKPPXRJQDG3voqfWJt"; // Pyth SOL/USD price feed
}

pub struct Processor {}

impl Processor {
    pub fn process_transfer(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        amount: u64,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        
        let from_account = next_account_info(account_info_iter)?;
        let to_account = next_account_info(account_info_iter)?;
        let authority = next_account_info(account_info_iter)?;
        let holder_data = next_account_info(account_info_iter)?;
        let price_feed = next_account_info(account_info_iter)?;
        let token_program = next_account_info(account_info_iter)?;

        // Verify authority
        if !authority.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        // Get current price from Pyth feed
        let current_price = Self::get_token_price(price_feed)?;
        
        // Get holder data
        let mut holder_data_account = HolderData::try_from_slice(&holder_data.data.borrow())?;
        
        // Validate transaction limits
        validate_transaction_limits(
            amount,
            current_price,
            holder_data_account.daily_transactions,
            Clock::get()?.unix_timestamp,
            holder_data_account.last_transaction_date,
        )?;

        // Process the transfer
        token::transfer(
            CpiContext::new(
                token_program.clone(),
                token::Transfer {
                    from: from_account.clone(),
                    to: to_account.clone(),
                    authority: authority.clone(),
                },
            ),
            amount,
        )?;

        // Update holder data
        let current_time = Clock::get()?.unix_timestamp;
        let today = (current_time / 86400) as i64;
        
        if holder_data_account.last_transaction_date != today {
            holder_data_account.daily_transactions = 0;
            holder_data_account.last_transaction_date = today;
        }
        
        holder_data_account.daily_transactions = holder_data_account.daily_transactions
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        
        holder_data_account.last_transfer = current_time;
        
        holder_data_account.serialize(&mut *holder_data.data.borrow_mut())?;

        Ok(())
    }

    pub fn get_token_price(price_feed_account: &AccountInfo) -> Result<u64, ProgramError> {
        let price_feed = load_price_feed_from_account_info(price_feed_account)
            .map_err(|_| TokenError::InvalidPriceFeed)?;
        
        let price_data = price_feed.get_current_price()
            .ok_or(TokenError::StalePrice)?;
            
        // Verify price feed status
        if price_data.status != PriceStatus::Trading {
            return Err(TokenError::InvalidPriceFeed.into());
        }
        
        // Check confidence interval
        let confidence_ratio = price_data.conf as f64 / price_data.price as f64;
        if confidence_ratio > 0.01 { // 1% confidence interval threshold
            return Err(TokenError::PriceConfidenceTooLow.into());
        }
        
        // Get the price in USD (6 decimals)
        let price_in_usd = price_data.price
            .checked_mul(10u64.pow(6))
            .ok_or(TokenError::ArithmeticOverflow)?
            .checked_div(10u64.pow(price_data.expo.abs() as u32))
            .ok_or(TokenError::ArithmeticOverflow)?;
            
        Ok(price_in_usd)
    }
}

#[program]
pub mod solanatoken {
    use super::*;

    pub fn initialize_token(
        ctx: Context<InitializeToken>,
    ) -> Result<()> {
        msg!("Initializing Next Gen Crypto Token");
        
        let rewards_vault = &mut ctx.accounts.rewards_vault;
        rewards_vault.authority = ctx.accounts.authority.key();
        rewards_vault.total_rewards = 0;
        rewards_vault.last_update = Clock::get()?.unix_timestamp;
        
        // Create the mint and set the mint authority
        token::mint_to(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token::MintTo {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.token_account.to_account_info(),
                    authority: ctx.accounts.authority.to_account_info(),
                },
            ),
            token_config::TOTAL_SUPPLY,
        )?;

        msg!("Minted {} tokens to {}", token_config::TOTAL_SUPPLY, ctx.accounts.authority.key());
        Ok(())
    }

    pub fn secure_transfer(
        ctx: Context<SecureTransfer>,
        amount: u64,
    ) -> Result<()> {
        let accounts = [
            ctx.accounts.from.to_account_info(),
            ctx.accounts.to.to_account_info(),
            ctx.accounts.authority.to_account_info(),
            ctx.accounts.holder_data.to_account_info(),
            ctx.accounts.price_feed.to_account_info(),
            ctx.accounts.token_program.to_account_info(),
        ];

        Processor::process_transfer(ctx.program_id, &accounts, amount)?;
        
        msg!("Secure transfer of {} tokens completed", amount);
        Ok(())
    }

    pub fn initialize_rewards(
        ctx: Context<InitializeRewards>,
    ) -> Result<()> {
        let holder_data = &mut ctx.accounts.holder_data;
        holder_data.authority = ctx.accounts.authority.key();
        holder_data.rewards_earned = 0;
        holder_data.last_claim = Clock::get()?.unix_timestamp;
        holder_data.last_transfer = 0;
        
        msg!("Initialized rewards for holder {}", ctx.accounts.authority.key());
        Ok(())
    }

    pub fn claim_rewards(
        ctx: Context<ClaimRewards>,
    ) -> Result<()> {
        let holder_data = &mut ctx.accounts.holder_data;
        let current_time = Clock::get()?.unix_timestamp;
        
        // Verify minimum holding period
        require!(
            current_time - holder_data.last_claim >= token_config::MIN_HOLDING_PERIOD,
            TokenError::MinHoldingPeriodNotMet
        );

        // Calculate rewards
        let holding_period = (current_time - holder_data.last_claim) as u64;
        let balance = ctx.accounts.token_account.amount;
        let rewards = calculate_rewards(balance, holding_period)?;

        // Update holder data
        holder_data.rewards_earned = holder_data.rewards_earned.checked_add(rewards)
            .ok_or(TokenError::ArithmeticOverflow)?;
        holder_data.last_claim = current_time;

        // Transfer rewards
        token::mint_to(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                token::MintTo {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.token_account.to_account_info(),
                    authority: ctx.accounts.mint_authority.to_account_info(),
                },
            ),
            rewards,
        )?;

        msg!("Claimed {} reward tokens", rewards);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeToken<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    
    #[account(
        init,
        payer = authority,
        mint::decimals = token_config::DECIMALS,
        mint::authority = authority.key(),
    )]
    pub mint: Account<'info, Mint>,
    
    #[account(
        init,
        payer = authority,
        associated_token::mint = mint,
        associated_token::authority = authority,
    )]
    pub token_account: Account<'info, TokenAccount>,
    
    #[account(
        init,
        payer = authority,
        space = 8 + RewardsVault::LEN
    )]
    pub rewards_vault: Account<'info, RewardsVault>,
    
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SecureTransfer<'info> {
    pub authority: Signer<'info>,
    
    #[account(
        mut,
        constraint = from.owner == authority.key(),
    )]
    pub from: Account<'info, TokenAccount>,
    
    #[account(mut)]
    pub to: Account<'info, TokenAccount>,
    
    #[account(
        mut,
        constraint = holder_data.authority == authority.key()
    )]
    pub holder_data: Account<'info, HolderData>,
    
    /// CHECK: This is safe as we validate it using Pyth SDK
    pub price_feed: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct InitializeRewards<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    
    #[account(
        init,
        payer = authority,
        space = 8 + HolderData::LEN
    )]
    pub holder_data: Account<'info, HolderData>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    pub authority: Signer<'info>,
    
    #[account(
        mut,
        constraint = holder_data.authority == authority.key()
    )]
    pub holder_data: Account<'info, HolderData>,
    
    #[account(mut)]
    pub mint: Account<'info, Mint>,
    
    #[account(
        mut,
        constraint = token_account.owner == authority.key()
    )]
    pub token_account: Account<'info, TokenAccount>,
    
    /// CHECK: This is safe because we verify it matches the mint authority
    #[account(
        constraint = mint_authority.key() == mint.mint_authority.unwrap()
    )]
    pub mint_authority: AccountInfo<'info>,
    
    pub token_program: Program<'info, Token>,
}

#[account]
pub struct RewardsVault {
    pub authority: Pubkey,
    pub total_rewards: u64,
    pub last_update: i64,
}

impl RewardsVault {
    pub const LEN: usize = 32 + 8 + 8;
}

#[account]
pub struct HolderData {
    pub authority: Pubkey,
    pub rewards_earned: u64,
    pub last_claim: i64,
    pub last_transfer: i64,
    pub daily_transactions: u64,
    pub last_transaction_date: i64,
}

impl HolderData {
    pub const LEN: usize = 32 + 8 + 8 + 8 + 8 + 8;
}

#[error_code]
pub enum TokenError {
    #[msg("Transfer amount exceeds 50% of balance")]
    TransferAmountTooLarge,
    #[msg("Transfer cooldown period is still active")]
    TransferCooldownActive,
    #[msg("Minimum holding period not met for rewards")]
    MinHoldingPeriodNotMet,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Invalid price feed")]
    InvalidPriceFeed,
    #[msg("Price feed is stale")]
    StalePrice,
    #[msg("Transaction amount below minimum USD value")]
    BelowMinimumUSD,
    #[msg("Transaction amount exceeds maximum size")]
    ExceedsMaxSize,
    #[msg("Daily transaction limit exceeded")]
    DailyLimitExceeded,
    #[msg("Price feed confidence interval too high")]
    PriceConfidenceTooLow,
}

// Helper function to calculate rewards
fn calculate_rewards(balance: u64, holding_period: u64) -> Result<u64> {
    // Annual rate in basis points (e.g., 500 = 5%)
    let annual_rate = token_config::REWARDS_RATE;
    
    // Calculate rewards: balance * (rate/10000) * (holding_period/31536000)
    // where 31536000 is seconds in a year
    let rewards = balance
        .checked_mul(annual_rate as u64)
        .ok_or(TokenError::ArithmeticOverflow)?
        .checked_mul(holding_period)
        .ok_or(TokenError::ArithmeticOverflow)?
        .checked_div(10000)
        .ok_or(TokenError::ArithmeticOverflow)?
        .checked_div(31_536_000)
        .ok_or(TokenError::ArithmeticOverflow)?;

    Ok(rewards)
}

// Helper function to validate transaction limits
fn validate_transaction_limits(
    amount: u64,
    price: u64,
    daily_transactions: u64,
    current_time: i64,
    last_transaction_date: i64,
) -> Result<()> {
    // Check minimum USD value
    let usd_value = (amount as u128 * price as u128) / 1_000_000;
    require!(
        usd_value >= token_config::MIN_PURCHASE_USD as u128,
        TokenError::BelowMinimumUSD
    );

    // Check maximum transaction size
    require!(
        amount <= token_config::MAX_TRANSACTION_SIZE,
        TokenError::ExceedsMaxSize
    );

    // Check daily transaction limit
    let today = (current_time / 86400) as i64;
    if last_transaction_date == today {
        require!(
            daily_transactions < token_config::MAX_DAILY_TRANSACTIONS,
            TokenError::DailyLimitExceeded
        );
    }

    Ok(())
}

// Helper function to get the next account from an iterator
fn next_account_info<'a, 'b>(
    iter: &mut std::slice::Iter<'a, AccountInfo<'b>>,
) -> Result<&'a AccountInfo<'b>, ProgramError> {
    iter.next().ok_or(ProgramError::NotEnoughAccountKeys)
}
