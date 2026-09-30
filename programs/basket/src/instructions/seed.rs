//! `seed` — the first buy (D4, D5, D8, §4, §5; change order §2, Q15).
//!
//! Deposits every component in kind, mints `X = initial_shares` (less the
//! mint fee, withheld as shares into the FeeVault) to the buyer and
//! `Y = X·r/(1−r)` treasury shares, and places the **tight** position on
//! the DLMM pool `create_basket` opened: `[active − w, active + w]` with the
//! type's half-width, Spot-shaped, shares as asks above the active bin and
//! the SOL leg as bids below. `sleeve_lamports` must agree with the launch
//! bin (`Y × price(active)`, one bin step of tolerance) — the program still
//! never prices anything, it only checks the client's split against the
//! pool's own price grid.
//!
//! `backstop_slice_bps` (20 %) of both legs stays idle in the basket's ATAs
//! for the keeper's `place_backstop` / `fund_backstop` (Q15: the launch
//! transaction does not carry the backstop). The 0.1 SOL creation fee goes
//! to the treasury. Bin arrays the tight range needs are created here (buyer
//! funds, reimbursed from the deployer's deposit); the position's own rent is
//! the buyer's and comes back when the position is closed.
//!
//! Remaining accounts: `[position, vault, buyer_ata, mint]` per position in
//! book order, then the bin arrays covering the tight range.

use anchor_lang::prelude::*;
use anchor_lang::system_program;
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
pub struct Seed<'info> {
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

    #[account(
        init,
        payer = payer,
        space = 8 + FeeVault::INIT_SPACE,
        seeds = [seeds::FEES, share_mint.key().as_ref()],
        bump,
    )]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint), created here.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// CHECK: receives the creation fee.
    #[account(mut, address = config.treasury @ BasketError::Unauthorized)]
    pub treasury: UncheckedAccount<'info>,

    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint): treasury shares (idle sleeve).
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL): the sleeve's SOL leg (idle sleeve).
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
    /// CHECK: the tight position PDA (`base` = basket), created here.
    #[account(mut)]
    pub tight_position: UncheckedAccount<'info>,
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
    pub rent: Sysvar<'info, Rent>,
}

pub fn handle_seed<'info>(ctx: Context<'info, Seed<'info>>, args: SeedArgs) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    {
        let basket = &ctx.accounts.basket;
        require!(basket.complete, BasketError::BasketIncomplete);
        require!(!basket.seeded, BasketError::AlreadySeeded);
        require!(basket.gate_open_at(now), BasketError::MintGateClosed);
    }
    require!(args.initial_shares > 0, BasketError::ZeroAmount);

    // 1. Components in.
    let n = ctx.accounts.basket.position_count as usize;
    require!(ctx.remaining_accounts.len() >= n * COMPONENT_GROUP, BasketError::ComponentCountMismatch);
    let (component_accounts, tail) = ctx.remaining_accounts.split_at(n * COMPONENT_GROUP);
    let components = load_components(&basket_key, component_accounts, n)?;
    let payer_ai = ctx.accounts.payer.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let token_2022_program = ctx.accounts.token_2022_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    transfer_components_in(&components, &args.deposits, &payer_ai, &token_program, &token_2022_program)?;

    // 2. Sleeve split. The pool is empty, so the step ratio applies.
    let basket = &ctx.accounts.basket;
    let r_bps = effective_r_bps(&basket.sleeve, 0);
    let treasury_shares = math::treasury_shares(args.initial_shares, r_bps)?;
    require!(treasury_shares > 0 && args.sleeve_lamports > 0, BasketError::ZeroAmount);
    let implied_total = math::implied_total_lamports(args.sleeve_lamports, r_bps)?
        .checked_add(config.fees.creation_fee_lamports)
        .ok_or_else(|| error!(BasketError::MathOverflow))?;
    require!(implied_total >= config.fees.min_seed_lamports, BasketError::BelowMinimum);
    let (to_buyer, fee_shares) = split_mint_fee(args.initial_shares, basket.fees.mint_fee_bps)?;

    // 3. The pool's price grid: the SOL leg must be Y priced at the active
    //    bin, one bin step either way.
    let dl = DlmmAccounts {
        program: &ctx.accounts.dlmm_program.to_account_info(),
        lb_pair: &ctx.accounts.lb_pair.to_account_info(),
        reserve_x: &ctx.accounts.reserve_x.to_account_info(),
        reserve_y: &ctx.accounts.reserve_y.to_account_info(),
        share_mint: &ctx.accounts.share_mint.to_account_info(),
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
    let (active_id, bin_step) = {
        let data = dl.lb_pair.try_borrow_data()?;
        dlmm::lb_pair_active_id(&data).ok_or_else(|| error!(BasketError::PoolMismatch))?
    };
    require!(bin_step == basket.pool.preset.bin_step, BasketError::PoolMismatch);
    let price = |id: i32| dlmm::bin_price_q64(id, bin_step).ok_or_else(|| error!(BasketError::MathOverflow));
    let low = dlmm::lamports_for_x(treasury_shares, price(active_id - 1)?).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let high = dlmm::lamports_for_x(treasury_shares, price(active_id + 1)?).ok_or_else(|| error!(BasketError::MathOverflow))?;
    require!(args.sleeve_lamports >= low && args.sleeve_lamports <= high, BasketError::LaunchPriceMismatch);

    // 4. Creation fee.
    system_program::transfer(
        CpiContext::new(
            system_program.key(),
            system_program::Transfer { from: payer_ai.clone(), to: ctx.accounts.treasury.to_account_info() },
        ),
        config.fees.creation_fee_lamports,
    )?;

    // 5. Token accounts. The basket's own ATAs are created now so `mint`
    //    and the keeper can rely on them.
    let basket_ai = ctx.accounts.basket.to_account_info();
    let share_mint_ai = ctx.accounts.share_mint.to_account_info();
    let wsol_mint_ai = ctx.accounts.wsol_mint.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let payer_share_ata = ctx.accounts.payer_share_ata.to_account_info();
    for (ata, authority, mint) in [
        (&payer_share_ata, &payer_ai, &share_mint_ai),
        (&ctx.accounts.fee_vault_share_ata.to_account_info(), &fee_vault_ai, &share_mint_ai),
        (dl.basket_share_ata, &basket_ai, &share_mint_ai),
        (dl.basket_wsol_ata, &basket_ai, &wsol_mint_ai),
    ] {
        ensure_ata(&payer_ai, ata, authority, mint, &system_program, &token_program, &ata_program)?;
    }

    // 6. Treasury shares and the SOL leg into the basket's ATAs.
    let signer = BasketSigner::new(basket);
    let share_auth_ai = ctx.accounts.share_auth.to_account_info();
    mint_shares(&share_mint_ai, dl.basket_share_ata, &share_auth_ai, &token_program, &signer, treasury_shares)?;
    wrap_sol(&payer_ai, dl.basket_wsol_ata, &system_program, &token_program, args.sleeve_lamports)?;

    // 7. The tight position: bin arrays (buyer funds, deposit reimburses),
    //    position PDA (buyer funds, refundable), 80 % of both legs in.
    let w = basket.pool.preset.tight_half_width_bins as i32;
    let lower = active_id - w;
    let upper = active_id + w;
    let (lo_idx, hi_idx) = dlmm::bin_array_range(lower, upper);
    let (array_rent, _) = dl.ensure_bin_arrays(tail, &payer_ai, lo_idx, hi_idx)?;
    let tight_ai = ctx.accounts.tight_position.to_account_info();
    dl.init_position(&signer, &tight_ai, false, &basket_ai, &payer_ai, &ctx.accounts.rent.to_account_info(), lower, upper - lower + 1)?;
    let slice = config.pools.backstop_slice_bps;
    let x_tight = treasury_shares - math::bps(treasury_shares, slice)?;
    let y_tight = args.sleeve_lamports - math::bps(args.sleeve_lamports, slice)?;
    let arrays = bin_arrays_from_tail(tail, &basket.pool.lb_pair, lower, upper)?;
    dl.add_liquidity(&signer, &tight_ai, arrays, x_tight, y_tight, active_id, lower, upper)?;

    // 8. Buyer and fee shares.
    mint_shares(&share_mint_ai, &payer_share_ata, &share_auth_ai, &token_program, &signer, to_buyer)?;
    mint_shares(&share_mint_ai, &ctx.accounts.fee_vault_share_ata.to_account_info(), &share_auth_ai, &token_program, &signer, fee_shares)?;

    // 9. Bookkeeping (the deposit move comes after the last CPI).
    let fee_vault = &mut ctx.accounts.fee_vault;
    fee_vault.bump = ctx.bumps.fee_vault;
    fee_vault.basket = basket_key;
    fee_vault.creator = ctx.accounts.basket.creator;
    fee_vault.payer = ctx.accounts.basket.payer;
    fee_vault.fees = ctx.accounts.basket.fees;
    let tight_key = ctx.accounts.tight_position.key();
    let creation_fee = config.fees.creation_fee_lamports;
    spend_deposit(&mut ctx.accounts.basket, &payer_ai, array_rent, SPEND_BIN_ARRAY)?;
    let basket = &mut ctx.accounts.basket;
    basket.seeded = true;
    basket.tight = PositionRef { key: tight_key, lower_bin_id: lower, upper_bin_id: upper };
    basket.last_recenter_ts = now;
    // D12: the high-water mark starts at the opening NAV, the fund's value
    // (components + sleeve, creation fee excluded) over the gross shares.
    let opening_value = implied_total - creation_fee;
    basket.hwm_nav_lamports = math::nav_per_share(opening_value, args.initial_shares)?;
    basket.last_crystallized_ts = now;
    basket.last_mgmt_accrual_ts = now;
    basket.touch(now);

    emit!(Seeded {
        basket: basket_key,
        buyer: ctx.accounts.payer.key(),
        tight_position: tight_key,
        tight_lower_bin_id: lower,
        tight_upper_bin_id: upper,
        shares_to_buyer: to_buyer,
        fee_shares,
        treasury_shares,
        sleeve_lamports: args.sleeve_lamports,
        creation_fee_lamports: creation_fee,
        bin_array_rent_lamports: array_rent,
    });
    Ok(())
}
