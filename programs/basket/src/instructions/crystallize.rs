//! `crystallize` (D12, §4, §5) and the management-fee accrual shared with
//! `apply_book`.
//!
//! Both fees are share inflation to the creator. The management fee accrues
//! pro rata temporis on the holder-share count. The performance fee is
//! charged only when the attested NAV (after the management dilution) is
//! above the fund-level high-water mark; the HWM then moves to the NAV the
//! holders actually see after the fee shares are minted. The NAV itself is
//! attested by a configured keeper — the program never prices (D4) — and is
//! reproducible off-chain from the vault balances and the pool.

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Mint, Token};

use crate::constants::*;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::sleeve::*;
use crate::math;
use crate::state::*;

#[derive(Accounts)]
pub struct Crystallize<'info> {
    /// Attests `nav_lamports_per_share`; must be in `config.keepers`.
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
    )]
    pub basket: Box<Account<'info, Basket>>,
    #[account(mut)]
    pub share_mint: Account<'info, Mint>,
    /// CHECK: mint authority PDA, seeds checked.
    #[account(seeds = [seeds::SHARE_AUTH, share_mint.key().as_ref()], bump = basket.share_auth_bump)]
    pub share_auth: UncheckedAccount<'info>,
    /// CHECK: `basket.creator`.
    pub creator: UncheckedAccount<'info>,
    /// CHECK: ATA(creator, share_mint), created if missing (keeper pays).
    #[account(mut)]
    pub creator_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, share_mint) — idle shares for the denominator.
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: the basket's DLMM pool.
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: the tight position.
    pub tight_position: UncheckedAccount<'info>,
    /// CHECK: the backstop position (any account when the basket has none).
    pub backstop_position: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Management fee due since `last_mgmt_accrual_ts`, minted to the creator.
/// Returns the shares minted; updates the accrual timestamp.
#[allow(clippy::too_many_arguments)]
pub fn accrue_mgmt_fee<'info>(
    basket: &mut Account<'info, Basket>,
    holder_shares: u64,
    now: i64,
    share_mint: &AccountInfo<'info>,
    creator_share_ata: &AccountInfo<'info>,
    share_auth: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
) -> Result<u64> {
    let elapsed = now.saturating_sub(basket.last_mgmt_accrual_ts);
    let shares = math::mgmt_fee_shares(holder_shares, basket.fees.mgmt_fee_bps_per_year, elapsed)?;
    if shares > 0 {
        let signer = BasketSigner::new(basket);
        mint_shares(share_mint, creator_share_ata, share_auth, token_program, &signer, shares)?;
    }
    basket.last_mgmt_accrual_ts = now;
    Ok(shares)
}

/// Remaining accounts: the pool's bin arrays.
pub fn handle_crystallize<'info>(ctx: Context<'info, Crystallize<'info>>, nav_lamports_per_share: u64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(nav_lamports_per_share > 0, BasketError::ZeroAmount);
    {
        let b = &ctx.accounts.basket;
        require!(b.is_managed(), BasketError::InvalidBasketType);
        require!(b.seeded, BasketError::NotSeeded);
        require!(
            now >= b.last_crystallized_ts.saturating_add(b.managed.crystallize_period_s),
            BasketError::CrystallizeTooSoon
        );
    }

    let h = {
        let view = PoolView::load(
            &ctx.accounts.basket,
            &ctx.accounts.basket.key(),
            &ctx.accounts.lb_pair.to_account_info(),
            Some(&ctx.accounts.tight_position.to_account_info()),
            Some(&ctx.accounts.backstop_position.to_account_info()),
            ctx.remaining_accounts,
        )?;
        let idle = idle_amount(&ctx.accounts.basket_share_ata.to_account_info())?;
        view.holder_shares(ctx.accounts.share_mint.supply, idle, ctx.accounts.basket.pending_redeem_shares)?
    };

    let keeper = ctx.accounts.keeper.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let share_auth = ctx.accounts.share_auth.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    ensure_ata(
        &keeper,
        &ctx.accounts.creator_share_ata,
        &ctx.accounts.creator,
        &share_mint,
        &ctx.accounts.system_program.to_account_info(),
        &token_program,
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;

    // 1. Management fee for the elapsed time, then the NAV holders see.
    let mgmt_shares = accrue_mgmt_fee(
        &mut ctx.accounts.basket,
        h,
        now,
        &share_mint,
        &ctx.accounts.creator_share_ata,
        &share_auth,
        &token_program,
    )?;
    let h1 = h.checked_add(mgmt_shares).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let nav1 = math::diluted_nav(nav_lamports_per_share, h, mgmt_shares)?;

    // 2. Performance fee above the high-water mark.
    let basket = &mut ctx.accounts.basket;
    let hwm_before = basket.hwm_nav_lamports;
    let above = nav1 > hwm_before;
    let perf_shares = if above { math::perf_fee_shares(h1, nav1, hwm_before, basket.fees.perf_fee_bps)? } else { 0 };
    require!(mgmt_shares > 0 || above, BasketError::NothingToCrystallize);
    if perf_shares > 0 {
        let signer = BasketSigner::new(basket);
        mint_shares(&share_mint, &ctx.accounts.creator_share_ata, &share_auth, &token_program, &signer, perf_shares)?;
    }
    if above {
        basket.hwm_nav_lamports = math::diluted_nav(nav1, h1, perf_shares)?;
    }
    basket.last_crystallized_ts = now;
    basket.touch(now);

    emit!(Crystallized {
        basket: basket.key(),
        nav_lamports: nav_lamports_per_share,
        hwm_before,
        perf_fee_shares: perf_shares,
        mgmt_fee_shares: mgmt_shares,
    });
    Ok(())
}
