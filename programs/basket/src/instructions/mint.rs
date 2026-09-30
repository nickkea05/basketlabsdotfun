//! `mint` — later buys (D4, D5, D6, §4, §5; change order Q6).
//!
//! `X = min_i(deposit_i · H / vault_i)` over the vaults before the deposit
//! (creation-unit rule; H = holder shares outstanding). The sleeve then
//! mints `Y = X·r/(1−r)` treasury shares and takes `Y × price(active bin)`
//! SOL from the buyer; `r` follows the step rule on the pool's SOL side.
//! While the backstop is live and the active bin is inside it,
//! `backstop_slice_bps` (20 %) of both legs is added to the backstop over
//! the ~70 bins around the active bin (a flat add over all 280 would not fit
//! a transaction) and the rest to the tight position; otherwise everything
//! goes to tight. If the active bin has left the tight range (keeper late),
//! only the leg that can be placed there is placed; the other waits idle for
//! `recenter_tight`. The mint fee is withheld as shares into the FeeVault.
//!
//! Remaining accounts: `[position, vault, buyer_ata, mint]` per position in
//! book order, then the bin arrays covering tight (and the backstop bins
//! being funded).

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Mint, Token};
use anchor_spl::token_2022::Token2022;

use crate::constants::*;
use crate::dlmm;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::{load_components, COMPONENT_GROUP};
use crate::instructions::sleeve::*;
use crate::math;
use crate::state::*;

#[derive(Accounts)]
pub struct MintShares<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Box<Account<'info, Basket>>,
    #[account(mut)]
    pub share_mint: Account<'info, Mint>,
    /// CHECK: mint authority PDA.
    #[account(seeds = [seeds::SHARE_AUTH, share_mint.key().as_ref()], bump = basket.share_auth_bump)]
    pub share_auth: UncheckedAccount<'info>,
    /// CHECK: ATA(payer, share_mint), created if missing.
    #[account(mut)]
    pub payer_share_ata: UncheckedAccount<'info>,
    #[account(seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint).
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,

    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint).
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL).
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,

    /// CHECK: the basket's DLMM pool.
    #[account(mut)]
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: pool reserve for shares.
    #[account(mut)]
    pub reserve_x: UncheckedAccount<'info>,
    /// CHECK: pool reserve for wSOL.
    #[account(mut)]
    pub reserve_y: UncheckedAccount<'info>,
    /// CHECK: the tight position.
    #[account(mut)]
    pub tight_position: UncheckedAccount<'info>,
    /// CHECK: the backstop position (any account when the basket has none).
    #[account(mut)]
    pub backstop_position: UncheckedAccount<'info>,
    /// CHECK: lb_clmm event authority.
    pub dlmm_event_authority: UncheckedAccount<'info>,
    /// CHECK: the DLMM program.
    #[account(address = dlmm::LB_CLMM_ID)]
    pub dlmm_program: UncheckedAccount<'info>,
    /// CHECK: SPL Memo.
    #[account(address = dlmm::MEMO_PROGRAM_ID)]
    pub memo_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Where a mint's sleeve goes: `(x, y, lower, upper)` for the backstop add
/// (zero when not applicable) and for the tight add, with legs the tight
/// range cannot hold left out (they stay idle).
pub struct SleevePlan {
    pub backstop: (u64, u64, i32, i32),
    pub tight: (u64, u64),
}

pub fn plan_sleeve(basket: &Basket, view: &PoolView, slice_bps: u16, x: u64, y: u64) -> Result<SleevePlan> {
    let active = view.active_id;
    let mut plan = SleevePlan { backstop: (0, 0, 0, 0), tight: (x, y) };
    if basket.backstop_live() && basket.backstop.contains(active) && view.backstop.is_some() {
        // Stay inside the bin arrays the tight add already carries: the
        // widest basket has no account locks left for extra arrays.
        let (t_lo, t_hi) = dlmm::bin_array_range(basket.tight.lower_bin_id, basket.tight.upper_bin_id);
        let floor = dlmm::array_lower_bin(t_lo).max(basket.backstop.lower_bin_id);
        let ceiling = dlmm::array_upper_bin(t_hi).min(basket.backstop.upper_bin_id);
        let lower = (active - dlmm::DEFAULT_BIN_PER_POSITION / 2).max(floor);
        let upper = (lower + dlmm::DEFAULT_BIN_PER_POSITION - 1).min(ceiling);
        let (x_bins, y_bins) = if lower <= upper { side_bins(lower, upper, active) } else { (0, 0) };
        let bx = if x_bins > 0 { math::bps(x, slice_bps)? } else { 0 };
        let by = if y_bins > 0 { math::bps(y, slice_bps)? } else { 0 };
        plan.backstop = (bx, by, lower, upper);
        plan.tight = (x - bx, y - by);
    }
    let (x_bins, y_bins) = side_bins(basket.tight.lower_bin_id, basket.tight.upper_bin_id, active);
    if x_bins == 0 {
        plan.tight.0 = 0;
    }
    if y_bins == 0 {
        plan.tight.1 = 0;
    }
    Ok(plan)
}

pub fn handle_mint<'info>(ctx: Context<'info, MintShares<'info>>, args: MintArgs) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    {
        let basket = &ctx.accounts.basket;
        require!(basket.seeded, BasketError::NotSeeded);
        require!(basket.gate_open_at(now), BasketError::MintGateClosed);
        // Vault ratios are in flux while positions are being traded; a
        // deposit priced off them would be wrong for either side.
        require!(!basket.rebalance.active, BasketError::RebalanceActive);
    }

    // 1. Shares from the vaults *before* the deposit (creation-unit rule).
    let n = ctx.accounts.basket.position_count as usize;
    require!(
        args.deposits.len() == n && ctx.remaining_accounts.len() >= n * COMPONENT_GROUP,
        BasketError::ComponentCountMismatch
    );
    let (component_accounts, tail) = ctx.remaining_accounts.split_at(n * COMPONENT_GROUP);
    let components = load_components(&basket_key, component_accounts, n)?;

    let basket = &ctx.accounts.basket;
    let view = PoolView::load(
        basket,
        &basket_key,
        &ctx.accounts.lb_pair.to_account_info(),
        Some(&ctx.accounts.tight_position.to_account_info()),
        Some(&ctx.accounts.backstop_position.to_account_info()),
        tail,
    )?;
    let idle_shares = idle_amount(&ctx.accounts.basket_share_ata.to_account_info())?;
    let idle_sol = idle_amount(&ctx.accounts.basket_wsol_ata.to_account_info())?;
    let holder_shares_before = view.holder_shares(ctx.accounts.share_mint.supply, idle_shares, basket.pending_redeem_shares)?;
    let pool_sol = view.sol_in_positions().checked_add(idle_sol).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let r_bps = effective_r_bps(&basket.sleeve, pool_sol);

    let vaults: Vec<u64> = components.iter().map(|c| c.available).collect();
    let gross_shares = math::shares_for_deposits(&args.deposits, &vaults, holder_shares_before)?;
    require!(gross_shares > 0, BasketError::ZeroAmount);
    let (to_buyer, fee_shares) = split_mint_fee(gross_shares, basket.fees.mint_fee_bps)?;
    require!(to_buyer >= args.min_shares_out, BasketError::SlippageExceeded);

    // 2. Sleeve at the pool's price: Y treasury shares, Y × P(active) SOL.
    let treasury_shares = math::treasury_shares(gross_shares, r_bps)?;
    let sleeve_lamports = dlmm::lamports_for_x(treasury_shares, view.price_q64(view.active_id)?)
        .ok_or_else(|| error!(BasketError::MathOverflow))?;
    require!(sleeve_lamports <= args.max_sleeve_lamports, BasketError::SlippageExceeded);
    if sleeve_lamports > 0 {
        let implied_total = math::implied_total_lamports(sleeve_lamports, r_bps)?;
        require!(implied_total >= config.fees.min_mint_lamports, BasketError::BelowMinimum);
    }
    let plan = plan_sleeve(basket, &view, config.pools.backstop_slice_bps, treasury_shares, sleeve_lamports)?;

    // 3. Components in.
    let payer_ai = ctx.accounts.payer.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let token_2022_program = ctx.accounts.token_2022_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    transfer_components_in(&components, &args.deposits, &payer_ai, &token_program, &token_2022_program)?;

    // 4. Sleeve in through the basket's own ATAs.
    let share_mint_ai = ctx.accounts.share_mint.to_account_info();
    let share_auth_ai = ctx.accounts.share_auth.to_account_info();
    let signer = BasketSigner::new(basket);
    let dl = DlmmAccounts {
        program: &ctx.accounts.dlmm_program.to_account_info(),
        lb_pair: &ctx.accounts.lb_pair.to_account_info(),
        reserve_x: &ctx.accounts.reserve_x.to_account_info(),
        reserve_y: &ctx.accounts.reserve_y.to_account_info(),
        share_mint: &share_mint_ai,
        wsol_mint: &ctx.accounts.wsol_mint.to_account_info(),
        basket_share_ata: &ctx.accounts.basket_share_ata.to_account_info(),
        basket_wsol_ata: &ctx.accounts.basket_wsol_ata.to_account_info(),
        basket: &ctx.accounts.basket.to_account_info(),
        token_program: &token_program,
        memo_program: &ctx.accounts.memo_program.to_account_info(),
        event_authority: &ctx.accounts.dlmm_event_authority.to_account_info(),
        system_program: &system_program,
    };
    dl.check(basket)?;
    mint_shares(&share_mint_ai, dl.basket_share_ata, &share_auth_ai, &token_program, &signer, treasury_shares)?;
    wrap_sol(&payer_ai, dl.basket_wsol_ata, &system_program, &token_program, sleeve_lamports)?;

    let (bx, by, b_lower, b_upper) = plan.backstop;
    if bx > 0 || by > 0 {
        let arrays = view.bin_arrays_for(b_lower, b_upper)?;
        dl.add_liquidity(&signer, &ctx.accounts.backstop_position.to_account_info(), arrays, bx, by, view.active_id, b_lower, b_upper)?;
    }
    let (tx, ty) = plan.tight;
    if tx > 0 || ty > 0 {
        let arrays = view.bin_arrays_for(basket.tight.lower_bin_id, basket.tight.upper_bin_id)?;
        dl.add_liquidity(
            &signer,
            &ctx.accounts.tight_position.to_account_info(),
            arrays,
            tx,
            ty,
            view.active_id,
            basket.tight.lower_bin_id,
            basket.tight.upper_bin_id,
        )?;
    }

    // 5. Buyer and fee shares (fee ATA address verified; buyer ATA created if missing).
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    for (ata, authority) in [
        (&ctx.accounts.payer_share_ata, &payer_ai),
        (&ctx.accounts.fee_vault_share_ata, &fee_vault_ai),
    ] {
        ensure_ata(&payer_ai, &ata.to_account_info(), authority, &share_mint_ai, &system_program, &token_program, &ata_program)?;
    }
    mint_shares(&share_mint_ai, &ctx.accounts.payer_share_ata.to_account_info(), &share_auth_ai, &token_program, &signer, to_buyer)?;
    mint_shares(&share_mint_ai, &ctx.accounts.fee_vault_share_ata.to_account_info(), &share_auth_ai, &token_program, &signer, fee_shares)?;

    let active_id = view.active_id;
    ctx.accounts.basket.touch(now);
    emit!(Minted {
        basket: basket_key,
        buyer: ctx.accounts.payer.key(),
        shares_to_buyer: to_buyer,
        fee_shares,
        treasury_shares,
        sleeve_lamports,
        backstop_lamports: by,
        r_bps,
        active_id,
    });
    Ok(())
}
