//! Protocol-level vaults (change order §2.5, Q8, Q9): the BUYBACK and PRIZE
//! PDAs that the fee path credits, `stage_buyback` / `execute_buyback`, and
//! `post_prize_payout`.
//!
//! `execute_buyback` swaps staged wSOL for $BSKT through an allow-listed
//! venue and burns what it gets; it refuses while `Config.bskt_mint` is
//! unset. Staging is its own instruction because the program moves lamports
//! out of its own PDA only after an instruction's last CPI.
//!
//! `post_prize_payout` pays one epoch's prizes by the configured split. The
//! ranking is off-chain; on-chain checks are the ones the program can make:
//! epoch elapsed and not yet paid, total within the PRIZE balance, recipient
//! count within Config, each recipient a live CreatorLock, and each
//! recipient's basket having claimed at least `min_epoch_pool_fees` of pool
//! fees in SOL during that epoch.

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount};
use anchor_spl::token_interface;

use crate::constants::*;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::fees::run_allowlisted_swap;
use crate::instructions::sleeve::{ensure_ata, move_lamports};
use crate::math;
use crate::state::*;

#[derive(Accounts)]
pub struct InitTreasuryVaults<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump, has_one = admin @ BasketError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
    #[account(init, payer = admin, space = 8 + BuybackVault::INIT_SPACE, seeds = [seeds::BUYBACK], bump)]
    pub buyback_vault: Account<'info, BuybackVault>,
    #[account(init, payer = admin, space = 8 + PrizeVault::INIT_SPACE, seeds = [seeds::PRIZE], bump)]
    pub prize_vault: Account<'info, PrizeVault>,
    pub system_program: Program<'info, System>,
}

pub fn handle_init_treasury_vaults(ctx: Context<InitTreasuryVaults>) -> Result<()> {
    let b = &mut ctx.accounts.buyback_vault;
    b.bump = ctx.bumps.buyback_vault;
    b.received_lamports = 0;
    b.spent_lamports = 0;
    b.burned_bskt = 0;
    let p = &mut ctx.accounts.prize_vault;
    p.bump = ctx.bumps.prize_vault;
    p.received_lamports = 0;
    p.paid_lamports = 0;
    p.last_paid_epoch = 0;
    p.paid_any = false;
    Ok(())
}

// ---------------------------------------------------------------------------
// Buyback
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct StageBuyback<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [seeds::BUYBACK], bump = buyback_vault.bump)]
    pub buyback_vault: Account<'info, BuybackVault>,
    /// ATA(buyback_vault, wSOL); the keeper creates it beforehand.
    #[account(
        mut,
        associated_token::mint = wsol_mint,
        associated_token::authority = buyback_vault,
        associated_token::token_program = token_program,
    )]
    pub buyback_wsol_ata: Account<'info, TokenAccount>,
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
}

/// Move `lamports` from the BUYBACK PDA onto its wSOL account (no CPI here;
/// `execute_buyback` syncs it).
pub fn handle_stage_buyback(ctx: Context<StageBuyback>, lamports: u64) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    require!(config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(config.bskt_mint.is_some(), BasketError::BsktMintState);
    require!(lamports > 0, BasketError::ZeroAmount);
    let vault_ai = ctx.accounts.buyback_vault.to_account_info();
    let free = vault_ai.lamports().saturating_sub(Rent::get()?.minimum_balance(vault_ai.data_len()));
    require!(lamports <= free, BasketError::MathOverflow);
    move_lamports(&vault_ai, &ctx.accounts.buyback_wsol_ata.to_account_info(), lamports)?;
    ctx.accounts.buyback_vault.spent_lamports += lamports;
    Ok(())
}

#[derive(Accounts)]
pub struct ExecuteBuyback<'info> {
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [seeds::BUYBACK], bump = buyback_vault.bump)]
    pub buyback_vault: Account<'info, BuybackVault>,
    #[account(
        mut,
        associated_token::mint = wsol_mint,
        associated_token::authority = buyback_vault,
        associated_token::token_program = token_program,
    )]
    pub buyback_wsol_ata: Account<'info, TokenAccount>,
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: `config.bskt_mint`, checked in the handler.
    #[account(mut)]
    pub bskt_mint: UncheckedAccount<'info>,
    /// CHECK: ATA(buyback_vault, bskt_mint), created if missing.
    #[account(mut)]
    pub buyback_bskt_ata: UncheckedAccount<'info>,
    /// CHECK: must be in `config.swap_programs`.
    pub swap_program: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    /// The program owning `bskt_mint` (Token or Token-2022).
    pub bskt_token_program: Interface<'info, token_interface::TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    // remaining: the inner instruction's accounts, in order. The BUYBACK PDA
    // is passed unsigned; the program signs for it.
}

pub fn handle_execute_buyback<'info>(ctx: Context<'info, ExecuteBuyback<'info>>, amount_in: u64, min_amount_out: u64, data: Vec<u8>) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    require!(config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(config.allows_swap_program(ctx.accounts.swap_program.key), BasketError::SwapProgramNotAllowed);
    require!(config.bskt_mint == Some(ctx.accounts.bskt_mint.key()), BasketError::BsktMintState);
    require_keys_eq!(*ctx.accounts.bskt_mint.owner, ctx.accounts.bskt_token_program.key(), BasketError::BsktMintState);

    let token_program = ctx.accounts.token_program.to_account_info();
    let vault_ai = ctx.accounts.buyback_vault.to_account_info();
    token::sync_native(CpiContext::new(token_program.key(), token::SyncNative { account: ctx.accounts.buyback_wsol_ata.to_account_info() }))?;
    ensure_ata(
        &ctx.accounts.keeper.to_account_info(),
        &ctx.accounts.buyback_bskt_ata,
        &vault_ai,
        &ctx.accounts.bskt_mint,
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.bskt_token_program.to_account_info(),
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;

    let bump = [ctx.accounts.buyback_vault.bump];
    let vault_seeds: [&[u8]; 2] = [seeds::BUYBACK, &bump];
    let wsol_ata = ctx.accounts.buyback_wsol_ata.to_account_info();
    let bskt_ata = ctx.accounts.buyback_bskt_ata.to_account_info();
    let allowed = [wsol_ata.key(), bskt_ata.key()];
    let (in_delta, out_delta) = run_allowlisted_swap(
        &ctx.accounts.swap_program.to_account_info(),
        &vault_ai,
        &vault_seeds,
        &allowed,
        &wsol_ata,
        &bskt_ata,
        ctx.remaining_accounts,
        amount_in,
        min_amount_out,
        data,
    )?;

    // Burn everything the ATA holds (this swap's output plus any dust).
    let to_burn = crate::instructions::positions::read_token_amount(&bskt_ata)?;
    if to_burn > 0 {
        token_interface::burn(
            CpiContext::new_with_signer(
                ctx.accounts.bskt_token_program.key(),
                token_interface::Burn { mint: ctx.accounts.bskt_mint.to_account_info(), from: bskt_ata.clone(), authority: vault_ai.clone() },
                &[&vault_seeds],
            ),
            to_burn,
        )?;
    }
    ctx.accounts.buyback_vault.burned_bskt += to_burn;
    let _ = out_delta;
    emit!(BuybackExecuted { lamports_in: in_delta, bskt_burned: to_burn });
    Ok(())
}

// ---------------------------------------------------------------------------
// Prizes
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct PostPrizePayout<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(mut, seeds = [seeds::PRIZE], bump = prize_vault.bump)]
    pub prize_vault: Account<'info, PrizeVault>,
    // remaining: `[creator_wallet, creator_lock, basket]` per recipient, in
    // rank order.
}

/// Pay `total_lamports` for `epoch` across the recipients by
/// `config.prizes.split_bps`; whatever the split leaves over stays in the
/// vault.
pub fn handle_post_prize_payout<'info>(ctx: Context<'info, PostPrizePayout<'info>>, epoch: u64, total_lamports: u64) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    require!(config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    let now = Clock::get()?.unix_timestamp;
    require!(config.prizes.epoch_at(now) > epoch, BasketError::PrizeEpoch);
    let vault = &ctx.accounts.prize_vault;
    require!(!vault.paid_any || epoch > vault.last_paid_epoch, BasketError::PrizeEpoch);
    require!(ctx.remaining_accounts.len() % 3 == 0, BasketError::InvalidArgument);
    let count = ctx.remaining_accounts.len() / 3;
    require!(count >= 1 && count <= config.prizes.max_recipients as usize, BasketError::InvalidArgument);
    let vault_ai = vault.to_account_info();
    let free = vault_ai.lamports().saturating_sub(Rent::get()?.minimum_balance(vault_ai.data_len()));
    require!(total_lamports > 0 && total_lamports <= free, BasketError::MathOverflow);

    let mut recipients = Vec::with_capacity(count);
    let mut amounts = Vec::with_capacity(count);
    let mut seen: Vec<Pubkey> = Vec::with_capacity(count);
    for i in 0..count {
        let wallet = &ctx.remaining_accounts[3 * i];
        let lock_ai = &ctx.remaining_accounts[3 * i + 1];
        let basket_ai = &ctx.remaining_accounts[3 * i + 2];
        require_keys_eq!(*basket_ai.owner, crate::ID, BasketError::PrizeIneligible);
        require_keys_eq!(*lock_ai.owner, crate::ID, BasketError::PrizeIneligible);
        let basket = {
            let data = basket_ai.try_borrow_data()?;
            Basket::try_deserialize(&mut &data[..])?
        };
        require!(!seen.contains(basket_ai.key), BasketError::PrizeIneligible);
        seen.push(*basket_ai.key);
        let expected_basket = Pubkey::find_program_address(&[seeds::BASKET, basket.share_mint.as_ref()], &crate::ID).0;
        require_keys_eq!(*basket_ai.key, expected_basket, BasketError::PrizeIneligible);
        require_keys_eq!(*wallet.key, basket.creator, BasketError::PrizeIneligible);
        let lock = {
            let data = lock_ai.try_borrow_data()?;
            CreatorLock::try_deserialize(&mut &data[..])?
        };
        let expected_lock = Pubkey::find_program_address(&[seeds::LOCK, basket.share_mint.as_ref()], &crate::ID).0;
        require_keys_eq!(*lock_ai.key, expected_lock, BasketError::PrizeIneligible);
        require!(lock.basket == *basket_ai.key && lock.amount > 0 && lock.unlock_at > now, BasketError::PrizeIneligible);
        require!(basket.epoch_pool_fees(epoch) >= config.prizes.min_epoch_pool_fees, BasketError::PrizeIneligible);

        let amount = math::bps(total_lamports, config.prizes.split_bps[i])?;
        recipients.push(*wallet.key);
        amounts.push(amount);
    }

    let mut paid = 0u64;
    for (i, amount) in amounts.iter().enumerate() {
        move_lamports(&vault_ai, &ctx.remaining_accounts[3 * i], *amount)?;
        paid += amount;
    }
    let vault = &mut ctx.accounts.prize_vault;
    vault.paid_lamports += paid;
    vault.last_paid_epoch = epoch;
    vault.paid_any = true;
    emit!(PrizesPaid { epoch, total_lamports: paid, recipients, amounts });
    Ok(())
}
