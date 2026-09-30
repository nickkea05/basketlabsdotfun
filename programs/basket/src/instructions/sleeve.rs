//! Shared pieces of `seed` / `mint` / `redeem` / the keeper instructions:
//! component transfers, share minting/burning under the basket's PDAs, wSOL
//! wrapping, the DLMM pool view (both positions read from their bin arrays)
//! and thin CPI wrappers around `lb_clmm` (D3, D5, change order §2).
//!
//! Sleeve model on DLMM: treasury shares (minted, backed by nothing) sit as
//! asks above the active bin, sleeve SOL as bids below, in two basket-owned
//! positions. `NAV = (components + SOL in positions + idle SOL) /
//! (supply − shares in positions − idle shares)`; `holder_shares()` is that
//! denominator. Anything left in the basket's own share / wSOL ATAs is
//! "idle sleeve": the unplaced backstop slice right after `seed`, and
//! deposit rounding afterwards.

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token;
use anchor_spl::token;
use anchor_spl::token_interface;

use crate::constants::*;
use crate::dlmm::{self, lb_clmm};
use crate::error::BasketError;
use crate::events::DepositSpent;
use crate::instructions::positions::{read_token_amount, ComponentAccounts};
use crate::math::{self, Rounding};
use crate::state::*;

pub struct BasketSigner {
    pub share_mint: Pubkey,
    pub bump: [u8; 1],
    pub share_auth_bump: [u8; 1],
}

impl BasketSigner {
    pub fn new(basket: &Basket) -> Self {
        BasketSigner { share_mint: basket.share_mint, bump: [basket.bump], share_auth_bump: [basket.share_auth_bump] }
    }

    pub fn basket_seeds(&self) -> [&[u8]; 3] {
        [seeds::BASKET, self.share_mint.as_ref(), &self.bump]
    }

    pub fn share_auth_seeds(&self) -> [&[u8]; 3] {
        [seeds::SHARE_AUTH, self.share_mint.as_ref(), &self.share_auth_bump]
    }
}

/// Create an ATA if it does not exist yet (`payer` funds it).
pub fn ensure_ata<'info>(
    payer: &AccountInfo<'info>,
    ata: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    associated_token_program: &AccountInfo<'info>,
) -> Result<()> {
    let expected = associated_token::get_associated_token_address_with_program_id(authority.key, mint.key, token_program.key);
    require_keys_eq!(ata.key(), expected, BasketError::ComponentMismatch);
    if !ata.data_is_empty() {
        return Ok(());
    }
    associated_token::create_idempotent(CpiContext::new(
        associated_token_program.key(),
        associated_token::Create {
            payer: payer.clone(),
            associated_token: ata.clone(),
            authority: authority.clone(),
            mint: mint.clone(),
            system_program: system_program.clone(),
            token_program: token_program.clone(),
        },
    ))
}

/// Move `deposits[i]` of every component from the buyer's ATA into the
/// basket vault. Every deposit must be positive: a zero leg would let the
/// creation-unit rule mint zero shares for real deposits elsewhere.
pub fn transfer_components_in<'info>(
    components: &[ComponentAccounts<'info>],
    deposits: &[u64],
    payer: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    token_2022_program: &AccountInfo<'info>,
) -> Result<()> {
    require!(components.len() == deposits.len(), BasketError::ComponentCountMismatch);
    for (i, (c, amount)) in components.iter().zip(deposits).enumerate() {
        // Deposits are indexed by book order.
        require!(c.position.index as usize == i, BasketError::ComponentMismatch);
        require!(*amount > 0, BasketError::ZeroAmount);
        let tp = if c.position.token_program == token::ID { token_program } else { token_2022_program };
        token_interface::transfer_checked(
            CpiContext::new(
                tp.key(),
                token_interface::TransferChecked {
                    from: c.user_ata.clone(),
                    mint: c.mint.clone(),
                    to: c.vault.clone(),
                    authority: payer.clone(),
                },
            ),
            *amount,
            c.position.decimals,
        )?;
    }
    Ok(())
}

pub fn mint_shares<'info>(
    share_mint: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    share_auth: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    signer: &BasketSigner,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let seeds = signer.share_auth_seeds();
    token::mint_to(
        CpiContext::new_with_signer(
            token_program.key(),
            token::MintTo { mint: share_mint.clone(), to: to.clone(), authority: share_auth.clone() },
            &[&seeds],
        ),
        amount,
    )
}

/// Burn `amount` shares from the basket's own share ATA (treasury shares
/// that came back out of a position, or their idle pro-rata).
pub fn burn_basket_shares<'info>(
    basket: &AccountInfo<'info>,
    basket_share_ata: &AccountInfo<'info>,
    share_mint: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    signer: &BasketSigner,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let seeds = signer.basket_seeds();
    token::burn(
        CpiContext::new_with_signer(
            token_program.key(),
            token::Burn { mint: share_mint.clone(), from: basket_share_ata.clone(), authority: basket.clone() },
            &[&seeds],
        ),
        amount,
    )
}

/// Transfer `amount` from a basket-owned token account.
pub fn transfer_from_basket<'info>(
    basket: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    signer: &BasketSigner,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let seeds = signer.basket_seeds();
    token::transfer(
        CpiContext::new_with_signer(
            token_program.key(),
            token::Transfer { from: from.clone(), to: to.clone(), authority: basket.clone() },
            &[&seeds],
        ),
        amount,
    )
}

/// Move `lamports` from a system-owned payer into a wSOL account.
pub fn wrap_sol<'info>(
    payer: &AccountInfo<'info>,
    wsol_ata: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    lamports: u64,
) -> Result<()> {
    if lamports == 0 {
        return Ok(());
    }
    system_program::transfer(
        CpiContext::new(system_program.key(), system_program::Transfer { from: payer.clone(), to: wsol_ata.clone() }),
        lamports,
    )?;
    token::sync_native(CpiContext::new(token_program.key(), token::SyncNative { account: wsol_ata.clone() }))
}

/// Effective sleeve ratio: the step ratio until the pool's SOL side reaches
/// the threshold, then the creator's ratio (D5).
pub fn effective_r_bps(sleeve: &SleeveParams, pool_sol_lamports: u64) -> u16 {
    if pool_sol_lamports < sleeve.step_threshold_lamports {
        sleeve.step_r_bps
    } else {
        sleeve.r_bps
    }
}

/// Split gross shares into (to buyer, fee) per the basket's mint fee (§4).
pub fn split_mint_fee(gross: u64, fee_bps: u16) -> Result<(u64, u64)> {
    let fee = math::bps(gross, fee_bps)?;
    Ok((gross - fee, fee))
}

pub fn round_up_bps(amount: u64, bps: u16) -> Result<u64> {
    math::mul_div_u64(amount, bps as u64, BPS_TOTAL as u64, Rounding::Up)
}

/// Move lamports between two accounts this program may debit. Must come
/// after the instruction's last CPI (the runtime re-checks the caller's
/// lamport balance at every CPI boundary).
pub fn move_lamports<'info>(from: &AccountInfo<'info>, to: &AccountInfo<'info>, amount: u64) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    **from.try_borrow_mut_lamports()? = from.lamports().checked_sub(amount).ok_or_else(|| error!(BasketError::MathOverflow))?;
    **to.try_borrow_mut_lamports()? = to.lamports().checked_add(amount).ok_or_else(|| error!(BasketError::MathOverflow))?;
    Ok(())
}

/// Reimburse `lamports` of non-refundable rent (pool, bin arrays) that
/// `funder` just paid, out of the deployer's deposit held on the Basket
/// account; whatever the deposit cannot cover stays with the funder. Call
/// after the last CPI. Returns what was reimbursed.
pub fn spend_deposit<'info>(
    basket: &mut Account<'info, Basket>,
    funder: &AccountInfo<'info>,
    lamports: u64,
    reason: u8,
) -> Result<u64> {
    if lamports == 0 {
        return Ok(0);
    }
    let available = basket.deposit_lamports.saturating_sub(basket.deposit_spent_lamports);
    let paid = lamports.min(available);
    if paid > 0 {
        let basket_ai = basket.to_account_info();
        // Never dip into the account's own rent exemption.
        let rent_min = Rent::get()?.minimum_balance(basket_ai.data_len());
        require!(basket_ai.lamports().saturating_sub(paid) >= rent_min, BasketError::MathOverflow);
        move_lamports(&basket_ai, funder, paid)?;
        basket.deposit_spent_lamports += paid;
    }
    emit!(DepositSpent { basket: basket.key(), lamports: paid, reason, spent_total: basket.deposit_spent_lamports });
    Ok(paid)
}

pub const SPEND_POOL: u8 = 0;
pub const SPEND_BIN_ARRAY: u8 = 1;

// ---------------------------------------------------------------------------
// Pool view: both positions, read from the bin arrays passed in
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct PositionAmounts {
    pub key: Pubkey,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    /// Treasury shares (X) the position holds, rounded up.
    pub amount_x: u64,
    /// SOL (Y) the position holds, rounded down.
    pub amount_y: u64,
}

pub struct PoolView<'a, 'info> {
    pub lb_pair: Pubkey,
    pub active_id: i32,
    pub bin_step: u16,
    arrays: Vec<(i64, &'a AccountInfo<'info>)>,
    pub tight: Option<PositionAmounts>,
    pub backstop: Option<PositionAmounts>,
}

impl<'a, 'info> PoolView<'a, 'info> {
    /// `lb_pair` is address-checked against the basket; `tight` / `backstop`
    /// are required whenever the basket has them; `tail` is scanned for the
    /// pool's bin arrays (any other accounts in it are ignored).
    pub fn load(
        basket: &Basket,
        basket_key: &Pubkey,
        lb_pair: &AccountInfo<'info>,
        tight: Option<&AccountInfo<'info>>,
        backstop: Option<&AccountInfo<'info>>,
        tail: &'a [AccountInfo<'info>],
    ) -> Result<Self> {
        require_keys_eq!(lb_pair.key(), basket.pool.lb_pair, BasketError::PoolMismatch);
        require_keys_eq!(*lb_pair.owner, dlmm::LB_CLMM_ID, BasketError::PoolMismatch);
        let (active_id, bin_step) = {
            let data = lb_pair.try_borrow_data()?;
            dlmm::lb_pair_active_id(&data).ok_or_else(|| error!(BasketError::PoolMismatch))?
        };
        let mut arrays: Vec<(i64, &'a AccountInfo<'info>)> = Vec::with_capacity(tail.len());
        for ai in tail {
            if *ai.owner != dlmm::LB_CLMM_ID {
                continue;
            }
            let data = ai.try_borrow_data()?;
            if let Some(h) = dlmm::bin_array_header(&data) {
                if h.lb_pair == basket.pool.lb_pair && arrays.iter().all(|(i, _)| *i != h.index) {
                    arrays.push((h.index, ai));
                }
            }
        }
        let mut view = PoolView { lb_pair: basket.pool.lb_pair, active_id, bin_step, arrays, tight: None, backstop: None };
        if basket.tight.is_set() {
            let ai = tight.ok_or_else(|| error!(BasketError::PositionMismatch))?;
            view.tight = Some(view.read_position(basket_key, &basket.tight, ai)?);
        }
        let backstop_has_liquidity = matches!(basket.backstop_state, BACKSTOP_FUNDING | BACKSTOP_LIVE | BACKSTOP_WITHDRAWING);
        if backstop_has_liquidity && basket.backstop.is_set() {
            let ai = backstop.ok_or_else(|| error!(BasketError::PositionMismatch))?;
            view.backstop = Some(view.read_position(basket_key, &basket.backstop, ai)?);
        }
        Ok(view)
    }

    pub fn bin_array(&self, index: i64) -> Result<&'a AccountInfo<'info>> {
        self.arrays.iter().find(|(i, _)| *i == index).map(|(_, ai)| *ai).ok_or_else(|| error!(BasketError::BinArrayMismatch))
    }

    /// Writable bin-array infos covering `[lower, upper]`, for a CPI's
    /// remaining accounts.
    pub fn bin_arrays_for(&self, lower: i32, upper: i32) -> Result<Vec<AccountInfo<'info>>> {
        let (lo, hi) = dlmm::bin_array_range(lower, upper);
        (lo..=hi).map(|i| self.bin_array(i).cloned()).collect()
    }

    fn read_position(&self, basket_key: &Pubkey, r: &PositionRef, ai: &AccountInfo<'info>) -> Result<PositionAmounts> {
        require_keys_eq!(ai.key(), r.key, BasketError::PositionMismatch);
        require_keys_eq!(*ai.owner, dlmm::LB_CLMM_ID, BasketError::PositionMismatch);
        let data = ai.try_borrow_data()?;
        let h = dlmm::position_header(&data).ok_or_else(|| error!(BasketError::PositionMismatch))?;
        require!(h.lb_pair == self.lb_pair && h.owner == *basket_key, BasketError::PositionMismatch);
        // A backstop still being funded is narrower than its planned range
        // (it grows one bin array per `fund_backstop`).
        require!(
            h.lower_bin_id == r.lower_bin_id && h.upper_bin_id <= r.upper_bin_id && h.upper_bin_id >= h.lower_bin_id,
            BasketError::PositionMismatch
        );
        let mut out = PositionAmounts { key: r.key, lower_bin_id: r.lower_bin_id, upper_bin_id: r.upper_bin_id, amount_x: 0, amount_y: 0 };
        let width = h.upper_bin_id - h.lower_bin_id + 1;
        let mut offset = 0i32;
        while offset < width {
            let bin_id = r.lower_bin_id + offset;
            let share = dlmm::position_bin_share(&data, offset as usize).ok_or_else(|| error!(BasketError::PositionMismatch))?;
            if share == 0 {
                offset += 1;
                continue;
            }
            let idx = dlmm::bin_array_index(bin_id);
            let arr = self.bin_array(idx)?;
            let arr_data = arr.try_borrow_data()?;
            let slot = (bin_id - dlmm::array_lower_bin(idx)) as usize;
            let bin = dlmm::bin_at(&arr_data, slot).ok_or_else(|| error!(BasketError::BinArrayMismatch))?;
            if bin.liquidity_supply > 0 {
                let (x, y) = if share >= bin.liquidity_supply {
                    (bin.amount_x, bin.amount_y)
                } else {
                    (
                        math::to_u64(math::mul_div_u256(
                            ruint::aliases::U256::from(bin.amount_x),
                            ruint::aliases::U256::from(share),
                            ruint::aliases::U256::from(bin.liquidity_supply),
                            Rounding::Up,
                        )?)?,
                        math::to_u64(math::mul_div_u256(
                            ruint::aliases::U256::from(bin.amount_y),
                            ruint::aliases::U256::from(share),
                            ruint::aliases::U256::from(bin.liquidity_supply),
                            Rounding::Down,
                        )?)?,
                    )
                };
                out.amount_x = out.amount_x.checked_add(x).ok_or_else(|| error!(BasketError::MathOverflow))?;
                out.amount_y = out.amount_y.checked_add(y).ok_or_else(|| error!(BasketError::MathOverflow))?;
            }
            offset += 1;
        }
        Ok(out)
    }

    /// Treasury shares sitting in positions.
    pub fn shares_in_positions(&self) -> u64 {
        self.tight.map(|p| p.amount_x).unwrap_or(0) + self.backstop.map(|p| p.amount_x).unwrap_or(0)
    }

    /// SOL sitting in positions.
    pub fn sol_in_positions(&self) -> u64 {
        self.tight.map(|p| p.amount_y).unwrap_or(0) + self.backstop.map(|p| p.amount_y).unwrap_or(0)
    }

    /// `supply − shares in positions − idle shares + shares mid-redemption`:
    /// the denominator of every pro-rata rule (§2 NAV note).
    pub fn holder_shares(&self, supply: u64, idle_shares: u64, pending_redeem_shares: u64) -> Result<u64> {
        supply
            .checked_sub(self.shares_in_positions())
            .and_then(|h| h.checked_sub(idle_shares))
            .and_then(|h| h.checked_add(pending_redeem_shares))
            .ok_or_else(|| error!(BasketError::MathOverflow))
    }

    pub fn price_q64(&self, bin_id: i32) -> Result<u128> {
        dlmm::bin_price_q64(bin_id, self.bin_step).ok_or_else(|| error!(BasketError::MathOverflow))
    }
}

/// Idle sleeve in the basket's own ATAs (0 when an ATA does not exist yet).
pub fn idle_amount(ata: &AccountInfo) -> Result<u64> {
    if ata.data_is_empty() {
        Ok(0)
    } else {
        read_token_amount(ata)
    }
}

// ---------------------------------------------------------------------------
// DLMM CPI wrappers
// ---------------------------------------------------------------------------

/// The accounts every liquidity CPI needs; `user_token_*` are the basket's
/// own ATAs so deposits are pulled from, and withdrawals land in, the
/// basket's idle sleeve.
pub struct DlmmAccounts<'a, 'info> {
    pub program: &'a AccountInfo<'info>,
    pub lb_pair: &'a AccountInfo<'info>,
    pub reserve_x: &'a AccountInfo<'info>,
    pub reserve_y: &'a AccountInfo<'info>,
    pub share_mint: &'a AccountInfo<'info>,
    pub wsol_mint: &'a AccountInfo<'info>,
    pub basket_share_ata: &'a AccountInfo<'info>,
    pub basket_wsol_ata: &'a AccountInfo<'info>,
    pub basket: &'a AccountInfo<'info>,
    pub token_program: &'a AccountInfo<'info>,
    pub memo_program: &'a AccountInfo<'info>,
    pub event_authority: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
}

impl<'a, 'info> DlmmAccounts<'a, 'info> {
    pub fn check(&self, basket: &Basket) -> Result<()> {
        require_keys_eq!(self.program.key(), dlmm::LB_CLMM_ID, BasketError::PoolMismatch);
        require_keys_eq!(self.lb_pair.key(), basket.pool.lb_pair, BasketError::PoolMismatch);
        require_keys_eq!(self.reserve_x.key(), dlmm::reserve(&basket.pool.lb_pair, &basket.share_mint), BasketError::PoolMismatch);
        require_keys_eq!(self.reserve_y.key(), dlmm::reserve(&basket.pool.lb_pair, &token::spl_token::native_mint::ID), BasketError::PoolMismatch);
        require_keys_eq!(self.share_mint.key(), basket.share_mint, BasketError::PoolMismatch);
        require_keys_eq!(self.wsol_mint.key(), token::spl_token::native_mint::ID, BasketError::PoolMismatch);
        require_keys_eq!(self.memo_program.key(), dlmm::MEMO_PROGRAM_ID, BasketError::PoolMismatch);
        require_keys_eq!(self.event_authority.key(), dlmm::event_authority(), BasketError::PoolMismatch);
        Ok(())
    }

    /// `add_liquidity_by_strategy2`, Spot / imbalanced: X over bins ≥ active,
    /// Y over bins ≤ active, within `[min_bin_id, max_bin_id]` (which must
    /// lie inside the position). `bin_arrays` cover the range.
    #[allow(clippy::too_many_arguments)]
    pub fn add_liquidity(
        &self,
        signer: &BasketSigner,
        position: &AccountInfo<'info>,
        bin_arrays: Vec<AccountInfo<'info>>,
        amount_x: u64,
        amount_y: u64,
        active_id: i32,
        min_bin_id: i32,
        max_bin_id: i32,
    ) -> Result<()> {
        if amount_x == 0 && amount_y == 0 {
            return Ok(());
        }
        let seeds = signer.basket_seeds();
        let signer_seeds: [&[&[u8]]; 1] = [&seeds];
        let ctx = CpiContext::new_with_signer(
            self.program.key(),
            lb_clmm::cpi::accounts::AddLiquidityByStrategy2 {
                position: position.clone(),
                lb_pair: self.lb_pair.clone(),
                bin_array_bitmap_extension: None,
                user_token_x: self.basket_share_ata.clone(),
                user_token_y: self.basket_wsol_ata.clone(),
                reserve_x: self.reserve_x.clone(),
                reserve_y: self.reserve_y.clone(),
                token_x_mint: self.share_mint.clone(),
                token_y_mint: self.wsol_mint.clone(),
                sender: self.basket.clone(),
                token_x_program: self.token_program.clone(),
                token_y_program: self.token_program.clone(),
                event_authority: self.event_authority.clone(),
                program: self.program.clone(),
            },
            &signer_seeds,
        )
        .with_remaining_accounts(bin_arrays);
        lb_clmm::cpi::add_liquidity_by_strategy2(
            ctx,
            lb_clmm::types::LiquidityParameterByStrategy {
                amount_x,
                amount_y,
                active_id,
                max_active_bin_slippage: 1,
                strategy_parameters: lb_clmm::types::StrategyParameters {
                    min_bin_id,
                    max_bin_id,
                    strategy_type: lb_clmm::types::StrategyType::SpotImBalanced,
                    parameteres: [0u8; 64],
                },
            },
            lb_clmm::types::RemainingAccountsInfo { slices: vec![] },
        )
    }

    /// `remove_liquidity_by_range2(bps)` over `[from, to]` into the basket's ATAs.
    pub fn remove_liquidity(
        &self,
        signer: &BasketSigner,
        position: &AccountInfo<'info>,
        bin_arrays: Vec<AccountInfo<'info>>,
        from_bin_id: i32,
        to_bin_id: i32,
        bps_to_remove: u16,
    ) -> Result<()> {
        if bps_to_remove == 0 {
            return Ok(());
        }
        let seeds = signer.basket_seeds();
        let signer_seeds: [&[&[u8]]; 1] = [&seeds];
        let ctx = CpiContext::new_with_signer(
            self.program.key(),
            lb_clmm::cpi::accounts::RemoveLiquidityByRange2 {
                position: position.clone(),
                lb_pair: self.lb_pair.clone(),
                bin_array_bitmap_extension: None,
                user_token_x: self.basket_share_ata.clone(),
                user_token_y: self.basket_wsol_ata.clone(),
                reserve_x: self.reserve_x.clone(),
                reserve_y: self.reserve_y.clone(),
                token_x_mint: self.share_mint.clone(),
                token_y_mint: self.wsol_mint.clone(),
                sender: self.basket.clone(),
                token_x_program: self.token_program.clone(),
                token_y_program: self.token_program.clone(),
                memo_program: self.memo_program.clone(),
                event_authority: self.event_authority.clone(),
                program: self.program.clone(),
            },
            &signer_seeds,
        )
        .with_remaining_accounts(bin_arrays);
        lb_clmm::cpi::remove_liquidity_by_range2(
            ctx,
            from_bin_id,
            to_bin_id,
            bps_to_remove,
            lb_clmm::types::RemainingAccountsInfo { slices: vec![] },
        )
    }

    /// `claim_fee2` over `[min, max]` into the basket's ATAs (SOL only under
    /// `collect_fee_mode = OnlyY`). Returns the wSOL delta.
    pub fn claim_fee(
        &self,
        signer: &BasketSigner,
        position: &AccountInfo<'info>,
        bin_arrays: Vec<AccountInfo<'info>>,
        min_bin_id: i32,
        max_bin_id: i32,
    ) -> Result<u64> {
        let before = read_token_amount(self.basket_wsol_ata)?;
        let seeds = signer.basket_seeds();
        let signer_seeds: [&[&[u8]]; 1] = [&seeds];
        let ctx = CpiContext::new_with_signer(
            self.program.key(),
            lb_clmm::cpi::accounts::ClaimFee2 {
                lb_pair: self.lb_pair.clone(),
                position: position.clone(),
                sender: self.basket.clone(),
                reserve_x: self.reserve_x.clone(),
                reserve_y: self.reserve_y.clone(),
                user_token_x: self.basket_share_ata.clone(),
                user_token_y: self.basket_wsol_ata.clone(),
                token_x_mint: self.share_mint.clone(),
                token_y_mint: self.wsol_mint.clone(),
                token_program_x: self.token_program.clone(),
                token_program_y: self.token_program.clone(),
                memo_program: self.memo_program.clone(),
                event_authority: self.event_authority.clone(),
                program: self.program.clone(),
            },
            &signer_seeds,
        )
        .with_remaining_accounts(bin_arrays);
        lb_clmm::cpi::claim_fee2(ctx, min_bin_id, max_bin_id, lb_clmm::types::RemainingAccountsInfo { slices: vec![] })?;
        Ok(read_token_amount(self.basket_wsol_ata)? - before)
    }

    /// `close_position2`; rent to `rent_receiver`.
    pub fn close_position(&self, signer: &BasketSigner, position: &AccountInfo<'info>, rent_receiver: &AccountInfo<'info>) -> Result<()> {
        let seeds = signer.basket_seeds();
        lb_clmm::cpi::close_position2(CpiContext::new_with_signer(
            self.program.key(),
            lb_clmm::cpi::accounts::ClosePosition2 {
                position: position.clone(),
                sender: self.basket.clone(),
                rent_receiver: rent_receiver.clone(),
                event_authority: self.event_authority.clone(),
                program: self.program.clone(),
            },
            &[&seeds],
        ))
    }

    /// `initialize_position_pda` owned by the basket with `base` = the
    /// basket (tight) or the share authority (backstop), then grown to
    /// `width` in ≤ 91-bin steps. `payer` funds the rent. Returns the rent.
    #[allow(clippy::too_many_arguments)]
    pub fn init_position(
        &self,
        signer: &BasketSigner,
        position: &AccountInfo<'info>,
        base_is_share_auth: bool,
        base: &AccountInfo<'info>,
        payer: &AccountInfo<'info>,
        rent: &AccountInfo<'info>,
        lower_bin_id: i32,
        width: i32,
    ) -> Result<u64> {
        // A position is created at most one bin array wide (the PDA is
        // seeded with that width) and grown later with `extend_position`,
        // one array per transaction: the runtime caps account growth at
        // 10 KB per transaction and a bin costs 112 bytes.
        require!(width >= 1 && width <= dlmm::DEFAULT_BIN_PER_POSITION, BasketError::InvalidArgument);
        let init_width = width;
        let expected = dlmm::position_pda(&self.lb_pair.key(), base.key, lower_bin_id, init_width);
        require_keys_eq!(position.key(), expected, BasketError::PositionMismatch);
        require!(position.data_is_empty(), BasketError::PositionMismatch);
        let basket_seeds = signer.basket_seeds();
        let auth_seeds = signer.share_auth_seeds();
        let signers: &[&[&[u8]]] = if base_is_share_auth { &[&basket_seeds, &auth_seeds] } else { &[&basket_seeds] };
        let before = payer.lamports();
        lb_clmm::cpi::initialize_position_pda(
            CpiContext::new_with_signer(
                self.program.key(),
                lb_clmm::cpi::accounts::InitializePositionPda {
                    payer: payer.clone(),
                    base: base.clone(),
                    position: position.clone(),
                    lb_pair: self.lb_pair.clone(),
                    owner: self.basket.clone(),
                    system_program: self.system_program.clone(),
                    rent: rent.clone(),
                    event_authority: self.event_authority.clone(),
                    program: self.program.clone(),
                },
                signers,
            ),
            lower_bin_id,
            init_width,
        )?;
        Ok(before.saturating_sub(payer.lamports()))
    }

    /// Grow `position` so it covers up to `minimum_upper_bin_id` (at most
    /// `MAX_RESIZE_LENGTH` bins more than it has, one call per transaction).
    /// Returns the rent the payer put in.
    pub fn extend_position(
        &self,
        signer: &BasketSigner,
        position: &AccountInfo<'info>,
        payer: &AccountInfo<'info>,
        minimum_upper_bin_id: i32,
    ) -> Result<u64> {
        let current_upper = {
            let data = position.try_borrow_data()?;
            dlmm::position_header(&data).ok_or_else(|| error!(BasketError::PositionMismatch))?.upper_bin_id
        };
        if minimum_upper_bin_id <= current_upper {
            return Ok(0);
        }
        require!(minimum_upper_bin_id - current_upper <= dlmm::MAX_RESIZE_LENGTH, BasketError::InvalidArgument);
        let basket_seeds = signer.basket_seeds();
        let before = payer.lamports();
        lb_clmm::cpi::increase_position_length2(
            CpiContext::new_with_signer(
                self.program.key(),
                lb_clmm::cpi::accounts::IncreasePositionLength2 {
                    funder: payer.clone(),
                    lb_pair: self.lb_pair.clone(),
                    position: position.clone(),
                    owner: self.basket.clone(),
                    system_program: self.system_program.clone(),
                    event_authority: self.event_authority.clone(),
                    program: self.program.clone(),
                },
                &[&basket_seeds],
            ),
            minimum_upper_bin_id,
        )?;
        Ok(before.saturating_sub(payer.lamports()))
    }

    /// Create every missing bin array in `[lo_index, hi_index]` (accounts
    /// found in `tail`), `funder` paying. Returns the rent paid (to be
    /// reimbursed from the deposit) and the number created.
    pub fn ensure_bin_arrays(
        &self,
        tail: &[AccountInfo<'info>],
        funder: &AccountInfo<'info>,
        lo_index: i64,
        hi_index: i64,
    ) -> Result<(u64, u8)> {
        let mut rent = 0u64;
        let mut created = 0u8;
        for index in lo_index..=hi_index {
            let key = dlmm::bin_array(&self.lb_pair.key(), index);
            let ai = tail.iter().find(|a| *a.key == key).ok_or_else(|| error!(BasketError::BinArrayMismatch))?;
            if !ai.data_is_empty() {
                continue;
            }
            lb_clmm::cpi::initialize_bin_array(
                CpiContext::new(
                    self.program.key(),
                    lb_clmm::cpi::accounts::InitializeBinArray {
                        lb_pair: self.lb_pair.clone(),
                        bin_array: ai.clone(),
                        funder: funder.clone(),
                        system_program: self.system_program.clone(),
                    },
                ),
                index,
            )?;
            rent += ai.lamports();
            created += 1;
        }
        Ok((rent, created))
    }
}

/// Writable bin-array infos for `[lower, upper]` looked up by address in
/// `tail` (for CPIs before a `PoolView` exists, e.g. right after creating
/// the arrays).
pub fn bin_arrays_from_tail<'info>(tail: &[AccountInfo<'info>], lb_pair: &Pubkey, lower: i32, upper: i32) -> Result<Vec<AccountInfo<'info>>> {
    let (lo, hi) = dlmm::bin_array_range(lower, upper);
    (lo..=hi)
        .map(|i| {
            let key = dlmm::bin_array(lb_pair, i);
            tail.iter().find(|a| *a.key == key).cloned().ok_or_else(|| error!(BasketError::BinArrayMismatch))
        })
        .collect()
}

/// Bins of `[lower, upper]` on each side of `active`: `(x_bins, y_bins)`,
/// the active bin counting on both sides (Spot puts both tokens there).
pub fn side_bins(lower: i32, upper: i32, active: i32) -> (u64, u64) {
    if upper < lower {
        return (0, 0);
    }
    let x = (upper - active.max(lower) + 1).max(0) as u64;
    let y = (active.min(upper) - lower + 1).max(0) as u64;
    (x, y)
}
