//! Keeper instructions on the two DLMM positions (change order §2.4, Q4–Q7,
//! Q15, round 2): `place_backstop`, `fund_backstop`, `withdraw_backstop`,
//! `close_backstop`, `recenter_tight`.
//!
//! **Backstop** lifecycle: `Unplaced → (place) Funding → (fund, per bin
//! array) Live → (withdraw, per bin array) Withdrawing → (close) Closed →
//! (place) Funding …`. A reset is withdraw + close + place around the
//! current active bin; the program only checks that the active bin is
//! outside the backstop's range when a reset starts, the keeper counts the
//! consecutive checks (`reset_confirm_checks`). Bin arrays the backstop
//! needs are created here (keeper funds, deposit reimburses); the position's
//! own rent is the keeper's and comes back at close.
//!
//! **Tight** re-centring: whenever the active bin is outside the tight range,
//! or past `recenter_trigger_bps` of the half-width from the centre and
//! `recenter_min_interval_s` has passed: remove everything, claim fees,
//! close the position (rent to the keeper), open `[active − w, active + w]`
//! (keeper pays), re-deposit. While the mint gate is open the keeper may
//! also mint treasury shares so asks are rebuilt after a run-up
//! (`Y_new = SOL_bids / P(active) − X_now`); while it is closed asks come
//! only from holders selling, which is what lets the premium form (Q7).
//!
//! Remaining accounts: the bin arrays each instruction names below.

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Mint, Token};

use crate::constants::*;
use crate::dlmm;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::read_token_amount;
use crate::instructions::sleeve::*;
use crate::math;
use crate::state::*;

/// Accounts every keeper pool instruction shares.
#[derive(Accounts)]
pub struct KeeperPool<'info> {
    #[account(mut)]
    pub keeper: Signer<'info>,
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
    /// CHECK: mint authority PDA (`base` of the backstop position).
    #[account(seeds = [seeds::SHARE_AUTH, share_mint.key().as_ref()], bump = basket.share_auth_bump)]
    pub share_auth: UncheckedAccount<'info>,
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint).
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL).
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, wSOL); created, filled, closed onto the FeeVault within this ix.
    #[account(mut)]
    pub fee_vault_wsol_ata: UncheckedAccount<'info>,

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
    /// CHECK: the backstop position (current or to be created).
    #[account(mut)]
    pub backstop_position: UncheckedAccount<'info>,
    /// CHECK: `recenter_tight`: the new tight position PDA; otherwise any account.
    #[account(mut)]
    pub new_position: UncheckedAccount<'info>,
    /// CHECK: lb_clmm event authority.
    pub dlmm_event_authority: UncheckedAccount<'info>,
    /// CHECK: the DLMM program.
    #[account(address = dlmm::LB_CLMM_ID)]
    pub dlmm_program: UncheckedAccount<'info>,
    /// CHECK: SPL Memo.
    #[account(address = dlmm::MEMO_PROGRAM_ID)]
    pub memo_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

struct Ctx<'a, 'info> {
    dl: DlmmAccounts<'a, 'info>,
    signer: BasketSigner,
    keeper: AccountInfo<'info>,
    basket_ai: AccountInfo<'info>,
}

fn setup<'a, 'info>(accounts: &'a KeeperPool<'info>, infos: &'a KeeperInfos<'info>) -> Result<Ctx<'a, 'info>> {
    let config = &accounts.config;
    require!(!config.paused, BasketError::Paused);
    require!(config.is_keeper(accounts.keeper.key), BasketError::Unauthorized);
    require!(accounts.basket.seeded, BasketError::NotSeeded);
    let dl = DlmmAccounts {
        program: &infos.dlmm_program,
        lb_pair: &infos.lb_pair,
        reserve_x: &infos.reserve_x,
        reserve_y: &infos.reserve_y,
        share_mint: &infos.share_mint,
        wsol_mint: &infos.wsol_mint,
        basket_share_ata: &infos.basket_share_ata,
        basket_wsol_ata: &infos.basket_wsol_ata,
        basket: &infos.basket,
        token_program: &infos.token_program,
        memo_program: &infos.memo_program,
        event_authority: &infos.dlmm_event_authority,
        system_program: &infos.system_program,
    };
    dl.check(&accounts.basket)?;
    Ok(Ctx { dl, signer: BasketSigner::new(&accounts.basket), keeper: infos.keeper.clone(), basket_ai: infos.basket.clone() })
}

/// Owned `AccountInfo`s so `DlmmAccounts` can borrow them for the whole handler.
struct KeeperInfos<'info> {
    keeper: AccountInfo<'info>,
    basket: AccountInfo<'info>,
    share_mint: AccountInfo<'info>,
    wsol_mint: AccountInfo<'info>,
    basket_share_ata: AccountInfo<'info>,
    basket_wsol_ata: AccountInfo<'info>,
    lb_pair: AccountInfo<'info>,
    reserve_x: AccountInfo<'info>,
    reserve_y: AccountInfo<'info>,
    dlmm_program: AccountInfo<'info>,
    dlmm_event_authority: AccountInfo<'info>,
    memo_program: AccountInfo<'info>,
    token_program: AccountInfo<'info>,
    system_program: AccountInfo<'info>,
}

impl<'info> KeeperInfos<'info> {
    fn collect(a: &KeeperPool<'info>) -> Self {
        KeeperInfos {
            keeper: a.keeper.to_account_info(),
            basket: a.basket.to_account_info(),
            share_mint: a.share_mint.to_account_info(),
            wsol_mint: a.wsol_mint.to_account_info(),
            basket_share_ata: a.basket_share_ata.to_account_info(),
            basket_wsol_ata: a.basket_wsol_ata.to_account_info(),
            lb_pair: a.lb_pair.to_account_info(),
            reserve_x: a.reserve_x.to_account_info(),
            reserve_y: a.reserve_y.to_account_info(),
            dlmm_program: a.dlmm_program.to_account_info(),
            dlmm_event_authority: a.dlmm_event_authority.to_account_info(),
            memo_program: a.memo_program.to_account_info(),
            token_program: a.token_program.to_account_info(),
            system_program: a.system_program.to_account_info(),
        }
    }
}

/// Claimed pool fees land in the basket's wSOL ATA; move them onto the
/// FeeVault through its temporary wSOL account (the keeper instructions do
/// not carry the routing accounts; the next `claim_pool_fees` routes them
/// via `FeeVault.unrouted_lamports`). Returns the temporary account's rent,
/// now sitting on the FeeVault, to hand back to the keeper after the last
/// CPI.
fn park_fees_on_fee_vault<'info>(a: &KeeperPool<'info>, c: &Ctx<'_, 'info>, claimed: u64) -> Result<u64> {
    if claimed == 0 {
        return Ok(0);
    }
    let fee_vault_ai = a.fee_vault.to_account_info();
    let token_program = a.token_program.to_account_info();
    let created = a.fee_vault_wsol_ata.data_is_empty();
    ensure_ata(
        &c.keeper,
        &a.fee_vault_wsol_ata,
        &fee_vault_ai,
        &a.wsol_mint.to_account_info(),
        &a.system_program.to_account_info(),
        &token_program,
        &a.associated_token_program.to_account_info(),
    )?;
    let wsol_rent = if created { a.fee_vault_wsol_ata.lamports() } else { 0 };
    transfer_from_basket(&c.basket_ai, c.dl.basket_wsol_ata, &a.fee_vault_wsol_ata, &token_program, &c.signer, claimed)?;
    let bump = [a.fee_vault.bump];
    let fee_seeds: [&[u8]; 3] = [seeds::FEES, a.basket.share_mint.as_ref(), &bump];
    anchor_spl::token::close_account(CpiContext::new_with_signer(
        token_program.key(),
        anchor_spl::token::CloseAccount { account: a.fee_vault_wsol_ata.to_account_info(), destination: fee_vault_ai.clone(), authority: fee_vault_ai.clone() },
        &[&fee_seeds],
    ))?;
    Ok(wsol_rent)
}

/// Bookkeeping for parked fees (after the last CPI).
fn record_parked(ctx_accounts: &mut KeeperPool, claimed: u64, now: i64) {
    if claimed == 0 {
        return;
    }
    ctx_accounts.fee_vault.unrouted_lamports += claimed;
    let epoch = ctx_accounts.config.prizes.epoch_at(now);
    ctx_accounts.basket.record_pool_fees(epoch, claimed);
}

// ---------------------------------------------------------------------------
// place_backstop
// ---------------------------------------------------------------------------

/// Remaining accounts: the bin arrays of the backstop range (existing or to
/// be created).
pub fn handle_place_backstop<'info>(ctx: Context<'info, KeeperPool<'info>>) -> Result<()> {
    let a = &ctx.accounts;
    let infos = KeeperInfos::collect(a);
    let c = setup(a, &infos)?;
    let basket = &a.basket;
    require!(
        matches!(basket.backstop_state, BACKSTOP_UNPLACED | BACKSTOP_CLOSED),
        BasketError::BackstopState
    );
    let preset = basket.pool.preset;
    let (active_id, _) = {
        let data = c.dl.lb_pair.try_borrow_data()?;
        dlmm::lb_pair_active_id(&data).ok_or_else(|| error!(BasketError::PoolMismatch))?
    };
    // First placement is around the launch bin (Q5); a re-placement after a
    // reset is around the current active bin.
    let center = if basket.backstop_state == BACKSTOP_UNPLACED { basket.pool.launch_active_id } else { active_id };
    let (lo_idx, hi_idx) = dlmm::backstop_arrays(center, preset.backstop_half_width_bins as i32, preset.backstop_max_arrays.min(MAX_BACKSTOP_ARRAYS));
    let lower = dlmm::array_lower_bin(lo_idx);
    let upper = dlmm::array_upper_bin(hi_idx);
    require!(upper - lower + 1 <= dlmm::POSITION_MAX_LENGTH, BasketError::InvalidArgument);

    // The position is created over the first bin array only; each
    // `fund_backstop` grows it by one array (10 KB realloc cap per tx).
    let (array_rent, _) = c.dl.ensure_bin_arrays(ctx.remaining_accounts, &c.keeper, lo_idx, hi_idx)?;
    let position_rent = c.dl.init_position(
        &c.signer,
        &a.backstop_position.to_account_info(),
        true,
        &a.share_auth.to_account_info(),
        &c.keeper,
        &a.rent.to_account_info(),
        lower,
        dlmm::MAX_BIN_PER_ARRAY,
    )?;

    let keeper = c.keeper.clone();
    let position = a.backstop_position.key();
    spend_deposit(&mut ctx.accounts.basket, &keeper, array_rent, SPEND_BIN_ARRAY)?;
    let basket = &mut ctx.accounts.basket;
    basket.backstop = PositionRef { key: position, lower_bin_id: lower, upper_bin_id: upper };
    basket.backstop_state = BACKSTOP_FUNDING;
    basket.backstop_mask = 0;
    emit!(BackstopPlaced {
        basket: basket.key(),
        position,
        lower_bin_id: lower,
        upper_bin_id: upper,
        bin_array_rent_lamports: array_rent,
        position_rent_lamports: position_rent,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// fund_backstop
// ---------------------------------------------------------------------------

/// Deposit the idle sleeve's share for bin array `array_index` of the
/// backstop: the idle amounts are split across the not-yet-funded arrays by
/// how many ask (X) / bid (Y) bins each holds relative to the active bin.
/// Remaining accounts: that bin array.
pub fn handle_fund_backstop<'info>(ctx: Context<'info, KeeperPool<'info>>, array_index: i64) -> Result<()> {
    let a = &ctx.accounts;
    let infos = KeeperInfos::collect(a);
    let c = setup(a, &infos)?;
    let basket = &a.basket;
    require!(basket.backstop_state == BACKSTOP_FUNDING, BasketError::BackstopState);
    require_keys_eq!(a.backstop_position.key(), basket.backstop.key, BasketError::PositionMismatch);
    let (lo_idx, hi_idx) = dlmm::bin_array_range(basket.backstop.lower_bin_id, basket.backstop.upper_bin_id);
    require!(array_index >= lo_idx && array_index <= hi_idx, BasketError::BinArrayMismatch);
    let bit = 1u8 << (array_index - lo_idx);
    // Arrays are funded in ascending order: the position grows one array at
    // a time and can only grow upwards.
    require!(basket.backstop_mask == bit - 1, BasketError::BackstopState);

    let (active_id, _) = {
        let data = c.dl.lb_pair.try_borrow_data()?;
        dlmm::lb_pair_active_id(&data).ok_or_else(|| error!(BasketError::PoolMismatch))?
    };
    let array_upper = dlmm::array_upper_bin(array_index).min(basket.backstop.upper_bin_id);
    let extend_rent = c.dl.extend_position(&c.signer, &a.backstop_position.to_account_info(), &c.keeper, array_upper)?;
    let idle_x = idle_amount(c.dl.basket_share_ata)?;
    let idle_y = idle_amount(c.dl.basket_wsol_ata)?;

    // Bins per side over the unfunded arrays, and this array's share.
    let mut x_total = 0u64;
    let mut y_total = 0u64;
    let mut x_here = 0u64;
    let mut y_here = 0u64;
    for idx in lo_idx..=hi_idx {
        if basket.backstop_mask & (1u8 << (idx - lo_idx)) != 0 {
            continue;
        }
        let lower = dlmm::array_lower_bin(idx).max(basket.backstop.lower_bin_id);
        let upper = dlmm::array_upper_bin(idx).min(basket.backstop.upper_bin_id);
        let (xb, yb) = side_bins(lower, upper, active_id);
        x_total += xb;
        y_total += yb;
        if idx == array_index {
            x_here = xb;
            y_here = yb;
        }
    }
    let amount_x = if x_total > 0 { math::mul_div_u64(idle_x, x_here, x_total, math::Rounding::Down)? } else { 0 };
    let amount_y = if y_total > 0 { math::mul_div_u64(idle_y, y_here, y_total, math::Rounding::Down)? } else { 0 };

    let lower = dlmm::array_lower_bin(array_index).max(basket.backstop.lower_bin_id);
    let upper = dlmm::array_upper_bin(array_index).min(basket.backstop.upper_bin_id);
    if amount_x > 0 || amount_y > 0 {
        let arrays = bin_arrays_from_tail(ctx.remaining_accounts, &basket.pool.lb_pair, lower, upper)?;
        c.dl.add_liquidity(&c.signer, &a.backstop_position.to_account_info(), arrays, amount_x, amount_y, active_id, lower, upper)?;
    }

    let basket = &mut ctx.accounts.basket;
    basket.backstop_mask |= bit;
    let all = (0..(hi_idx - lo_idx + 1)).fold(0u8, |m, i| m | (1u8 << i));
    let complete = basket.backstop_mask == all;
    if complete {
        basket.backstop_state = BACKSTOP_LIVE;
    }
    emit!(BackstopFunded { basket: basket.key(), array_index, amount_x, amount_y, position_rent_lamports: extend_rent, complete });
    Ok(())
}

// ---------------------------------------------------------------------------
// withdraw_backstop / close_backstop
// ---------------------------------------------------------------------------

/// Remove everything in bin array `array_index` of the backstop and claim
/// its fees (parked on the FeeVault). Allowed when the basket is closing
/// (caller: `close_basket` flow) or when a reset is due: the active bin is
/// outside the backstop's range. Remaining accounts: that bin array.
pub fn handle_withdraw_backstop<'info>(mut ctx: Context<'info, KeeperPool<'info>>, array_index: i64) -> Result<()> {
    let a = &ctx.accounts;
    let infos = KeeperInfos::collect(a);
    let c = setup(a, &infos)?;
    let basket = &a.basket;
    require!(
        matches!(basket.backstop_state, BACKSTOP_FUNDING | BACKSTOP_LIVE | BACKSTOP_WITHDRAWING),
        BasketError::BackstopState
    );
    require_keys_eq!(a.backstop_position.key(), basket.backstop.key, BasketError::PositionMismatch);
    let (lo_idx, hi_idx) = dlmm::bin_array_range(basket.backstop.lower_bin_id, basket.backstop.upper_bin_id);
    require!(array_index >= lo_idx && array_index <= hi_idx, BasketError::BinArrayMismatch);
    // Only arrays that were funded hold anything (and are covered by the position).
    require!(basket.backstop_mask & (1u8 << (array_index - lo_idx)) != 0, BasketError::BackstopState);
    let (active_id, _) = {
        let data = c.dl.lb_pair.try_borrow_data()?;
        dlmm::lb_pair_active_id(&data).ok_or_else(|| error!(BasketError::PoolMismatch))?
    };
    if basket.backstop_state != BACKSTOP_WITHDRAWING {
        // Starting a withdrawal: a reset is due (active bin has left the
        // backstop) or the basket is winding down (gate closed and idle).
        let now = Clock::get()?.unix_timestamp;
        let reset_due = !basket.backstop.contains(active_id);
        let winding_down = !basket.gate_open_at(now) && now - basket.last_activity_at >= a.config.close_idle_s;
        require!(reset_due || winding_down, BasketError::BackstopState);
    }

    let lower = dlmm::array_lower_bin(array_index).max(basket.backstop.lower_bin_id);
    let upper = dlmm::array_upper_bin(array_index).min(basket.backstop.upper_bin_id);
    let x_before = idle_amount(c.dl.basket_share_ata)?;
    let y_before = idle_amount(c.dl.basket_wsol_ata)?;
    let arrays = bin_arrays_from_tail(ctx.remaining_accounts, &basket.pool.lb_pair, lower, upper)?;
    c.dl.remove_liquidity(&c.signer, &a.backstop_position.to_account_info(), arrays.clone(), lower, upper, BPS_TOTAL)?;
    let amount_x = read_token_amount(c.dl.basket_share_ata)? - x_before;
    let amount_y = read_token_amount(c.dl.basket_wsol_ata)? - y_before;
    let claimed = c.dl.claim_fee(&c.signer, &a.backstop_position.to_account_info(), arrays, lower, upper)?;
    let wsol_rent = park_fees_on_fee_vault(a, &c, claimed)?;
    move_lamports(&a.fee_vault.to_account_info(), &c.keeper, wsol_rent)?;
    let now = Clock::get()?.unix_timestamp;

    record_parked(&mut ctx.accounts, claimed, now);
    let basket = &mut ctx.accounts.basket;
    basket.backstop_state = BACKSTOP_WITHDRAWING;
    basket.backstop_mask &= !(1u8 << (array_index - lo_idx));
    let complete = basket.backstop_mask == 0;
    emit!(BackstopWithdrawn { basket: basket.key(), array_index, amount_x, amount_y, fee_lamports: claimed, complete });
    Ok(())
}

/// Close the emptied backstop position (rent to the keeper).
pub fn handle_close_backstop<'info>(ctx: Context<'info, KeeperPool<'info>>) -> Result<()> {
    let a = &ctx.accounts;
    let infos = KeeperInfos::collect(a);
    let c = setup(a, &infos)?;
    let basket = &a.basket;
    require!(basket.backstop_state == BACKSTOP_WITHDRAWING && basket.backstop_mask == 0, BasketError::BackstopState);
    require_keys_eq!(a.backstop_position.key(), basket.backstop.key, BasketError::PositionMismatch);
    {
        let data = a.backstop_position.try_borrow_data()?;
        // The position may be narrower than the planned range if the
        // withdrawal started before funding finished.
        let header = dlmm::position_header(&data).ok_or_else(|| error!(BasketError::PositionMismatch))?;
        let width = (header.upper_bin_id - header.lower_bin_id + 1) as usize;
        for offset in 0..width {
            require!(dlmm::position_bin_share(&data, offset).unwrap_or(1) == 0, BasketError::BackstopState);
        }
    }
    c.dl.close_position(&c.signer, &a.backstop_position.to_account_info(), &c.keeper)?;
    let position = a.backstop_position.key();
    let basket = &mut ctx.accounts.basket;
    basket.backstop_state = BACKSTOP_CLOSED;
    basket.backstop = PositionRef::default();
    emit!(BackstopClosed { basket: basket.key(), position });
    Ok(())
}

// ---------------------------------------------------------------------------
// recenter_tight
// ---------------------------------------------------------------------------

/// Is a re-centre allowed now? Out of range: always. Inside: past the
/// trigger band and past the minimum interval.
pub fn recenter_allowed(basket: &Basket, config: &Config, active_id: i32, now: i64) -> bool {
    let t = &basket.tight;
    if !t.contains(active_id) {
        return true;
    }
    let half = (t.width() / 2).max(1) as i64;
    let center = t.lower_bin_id as i64 + half;
    let drift = (active_id as i64 - center).abs();
    let trigger = half * config.pools.recenter_trigger_bps as i64 / BPS_TOTAL as i64;
    drift >= trigger && now - basket.last_recenter_ts >= config.pools.recenter_min_interval_s
}

/// Remaining accounts: bin arrays of the old tight range, then of the new
/// one (existing or to be created).
pub fn handle_recenter_tight<'info>(mut ctx: Context<'info, KeeperPool<'info>>) -> Result<()> {
    let a = &ctx.accounts;
    let infos = KeeperInfos::collect(a);
    let c = setup(a, &infos)?;
    let basket = &a.basket;
    let config = &a.config;
    let now = Clock::get()?.unix_timestamp;
    require!(!basket.rebalance.active, BasketError::RebalanceActive);
    require_keys_eq!(a.tight_position.key(), basket.tight.key, BasketError::PositionMismatch);
    let (active_id, _) = {
        let data = c.dl.lb_pair.try_borrow_data()?;
        dlmm::lb_pair_active_id(&data).ok_or_else(|| error!(BasketError::PoolMismatch))?
    };
    require!(recenter_allowed(basket, config, active_id, now), BasketError::RecenterNotNeeded);
    let old = basket.tight;

    // 1. Everything out of the old position, fees claimed, position closed.
    let x_before = idle_amount(c.dl.basket_share_ata)?;
    let y_before = idle_amount(c.dl.basket_wsol_ata)?;
    let old_arrays = bin_arrays_from_tail(ctx.remaining_accounts, &basket.pool.lb_pair, old.lower_bin_id, old.upper_bin_id)?;
    c.dl.remove_liquidity(&c.signer, &a.tight_position.to_account_info(), old_arrays.clone(), old.lower_bin_id, old.upper_bin_id, BPS_TOTAL)?;
    let removed_x = read_token_amount(c.dl.basket_share_ata)? - x_before;
    let removed_y = read_token_amount(c.dl.basket_wsol_ata)? - y_before;
    let claimed = c.dl.claim_fee(&c.signer, &a.tight_position.to_account_info(), old_arrays, old.lower_bin_id, old.upper_bin_id)?;
    let wsol_rent = park_fees_on_fee_vault(a, &c, claimed)?;
    c.dl.close_position(&c.signer, &a.tight_position.to_account_info(), &c.keeper)?;

    // 2. The new range; arrays and position (keeper pays; arrays reimbursed).
    let w = basket.pool.preset.tight_half_width_bins as i32;
    let lower = active_id - w;
    let upper = active_id + w;
    let (lo_idx, hi_idx) = dlmm::bin_array_range(lower, upper);
    let (array_rent, _) = c.dl.ensure_bin_arrays(ctx.remaining_accounts, &c.keeper, lo_idx, hi_idx)?;
    c.dl.init_position(&c.signer, &a.new_position.to_account_info(), false, &c.basket_ai, &c.keeper, &a.rent.to_account_info(), lower, upper - lower + 1)?;

    // 3. What goes back in: the removed amounts, plus all idle — unless the
    //    idle is the backstop's (its slice waits there until it is placed
    //    and funded; a withdrawn backstop's SOL waits for the re-placement).
    //    While the gate is open, top the ask side up so bids and asks are
    //    balanced at the new price.
    let idle_is_backstops = basket.backstop_state != BACKSTOP_LIVE;
    let (mut amount_x, amount_y) = if idle_is_backstops {
        (removed_x, removed_y)
    } else {
        (read_token_amount(c.dl.basket_share_ata)?, read_token_amount(c.dl.basket_wsol_ata)?)
    };
    let mut topped_up = 0u64;
    if basket.gate_open_at(now) {
        let price = dlmm::bin_price_q64(active_id, basket.pool.preset.bin_step).ok_or_else(|| error!(BasketError::MathOverflow))?;
        let target_x = dlmm::x_for_lamports(amount_y, price).ok_or_else(|| error!(BasketError::MathOverflow))?;
        if target_x > amount_x {
            topped_up = target_x - amount_x;
            mint_shares(c.dl.share_mint, c.dl.basket_share_ata, &a.share_auth.to_account_info(), c.dl.token_program, &c.signer, topped_up)?;
            amount_x = target_x;
        }
    }
    let new_arrays = bin_arrays_from_tail(ctx.remaining_accounts, &basket.pool.lb_pair, lower, upper)?;
    c.dl.add_liquidity(&c.signer, &a.new_position.to_account_info(), new_arrays, amount_x, amount_y, active_id, lower, upper)?;

    // 4. Lamport moves after the last CPI: array rent from the deposit,
    //    the temporary fee account's rent back to the keeper.
    let keeper = c.keeper.clone();
    move_lamports(&a.fee_vault.to_account_info(), &keeper, wsol_rent)?;
    let new_key = a.new_position.key();
    record_parked(&mut ctx.accounts, claimed, now);
    spend_deposit(&mut ctx.accounts.basket, &keeper, array_rent, SPEND_BIN_ARRAY)?;
    let basket = &mut ctx.accounts.basket;
    basket.tight = PositionRef { key: new_key, lower_bin_id: lower, upper_bin_id: upper };
    basket.last_recenter_ts = now;
    basket.recenter_count += 1;
    emit!(TightRecentered {
        basket: basket.key(),
        keeper: keeper.key(),
        active_id,
        old_position: old.key,
        old_lower_bin_id: old.lower_bin_id,
        old_upper_bin_id: old.upper_bin_id,
        new_position: new_key,
        new_lower_bin_id: lower,
        new_upper_bin_id: upper,
        amount_x,
        amount_y,
        topped_up_shares: topped_up,
        fee_lamports: claimed,
        bin_array_rent_lamports: array_rent,
    });
    Ok(())
}
