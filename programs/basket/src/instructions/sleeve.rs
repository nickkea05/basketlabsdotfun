//! Shared pieces of `seed` / `mint` / `redeem`: component transfers, share
//! minting/burning under the basket's PDAs, wSOL wrapping and the DAMM v2
//! fee configuration (D3, D5, D13, §3).

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token;
use anchor_spl::token;
use anchor_spl::token_interface;

use crate::constants::*;
use crate::damm::{self, cp_amm};
use crate::error::BasketError;
use crate::instructions::positions::ComponentAccounts;
use crate::math::{self, Rounding};
use crate::state::*;

pub struct BasketSigner {
    pub share_mint: Pubkey,
    pub bump: [u8; 1],
    pub share_auth_bump: [u8; 1],
}

impl BasketSigner {
    pub fn new(basket: &Basket) -> Self {
        BasketSigner {
            share_mint: basket.share_mint,
            bump: [basket.bump],
            share_auth_bump: [basket.share_auth_bump],
        }
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
    let expected = associated_token::get_associated_token_address_with_program_id(
        authority.key,
        mint.key,
        token_program.key,
    );
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

/// Burn whatever shares are left in the basket's own share ATA (treasury
/// shares the pool did not pull because of rounding). Keeps
/// `supply − pool_shares` an exact holder count.
pub fn burn_basket_share_dust<'info>(
    basket: &AccountInfo<'info>,
    basket_share_ata: &AccountInfo<'info>,
    share_mint: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    signer: &BasketSigner,
) -> Result<u64> {
    let left = crate::instructions::positions::read_token_amount(basket_share_ata)?;
    if left == 0 {
        return Ok(0);
    }
    let seeds = signer.basket_seeds();
    token::burn(
        CpiContext::new_with_signer(
            token_program.key(),
            token::Burn { mint: share_mint.clone(), from: basket_share_ata.clone(), authority: basket.clone() },
            &[&seeds],
        ),
        left,
    )?;
    Ok(left)
}

/// Move `lamports` from the buyer into the basket's wSOL ATA.
pub fn wrap_sol<'info>(
    payer: &AccountInfo<'info>,
    basket_wsol_ata: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    lamports: u64,
) -> Result<()> {
    if lamports == 0 {
        return Ok(());
    }
    system_program::transfer(
        CpiContext::new(
            system_program.key(),
            system_program::Transfer { from: payer.clone(), to: basket_wsol_ata.clone() },
        ),
        lamports,
    )?;
    token::sync_native(CpiContext::new(
        token_program.key(),
        token::SyncNative { account: basket_wsol_ata.clone() },
    ))
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

/// cp-amm fee config for a new pool: flat profile fee for `Open` baskets,
/// the launch scheduler (cliff → profile fee, linear) otherwise (D13).
pub fn pool_fee_parameters(config: &Config, basket: &Basket) -> Result<cp_amm::types::PoolFeeParameters> {
    let end = damm::bps_to_fee_numerator(basket.fees.pool_fee_bps as u64);
    let base_fee = if basket.uses_launch_scheduler() && config.scheduler.periods > 0 {
        let cliff = damm::bps_to_fee_numerator(config.scheduler.cliff_bps as u64);
        require!(cliff >= end, BasketError::FeeOutOfBounds);
        let reduction = (cliff - end) / config.scheduler.periods as u64;
        damm::time_scheduler_base_fee(cliff, config.scheduler.periods, config.scheduler.period_s, reduction)
    } else {
        damm::flat_base_fee(end)
    };
    Ok(cp_amm::types::PoolFeeParameters { base_fee, compounding_fee_bps: 0, padding: 0, dynamic_fee: None })
}

/// Split gross shares into (to buyer, fee) per the basket's mint fee (§4).
pub fn split_mint_fee(gross: u64, fee_bps: u16) -> Result<(u64, u64)> {
    let fee = math::bps(gross, fee_bps)?;
    Ok((gross - fee, fee))
}

/// Shares the basket's pool position currently holds (rounded up, see
/// `math::position_shares`).
pub fn position_share_amount(pool: &cp_amm::accounts::Pool, position: &cp_amm::accounts::Position) -> Result<u64> {
    let liquidity = position
        .unlocked_liquidity
        .checked_add(position.vested_liquidity)
        .and_then(|l| l.checked_add(position.permanent_locked_liquidity))
        .ok_or_else(|| error!(BasketError::MathOverflow))?;
    math::position_shares(liquidity, pool.sqrt_min_price, pool.sqrt_price, pool.sqrt_max_price)
}

/// `supply − shares in our pool position + shares mid-redemption`: the
/// denominator of every pro-rata rule (§2 NAV note).
pub fn holder_shares(
    supply: u64,
    pool: &cp_amm::accounts::Pool,
    position: &cp_amm::accounts::Position,
    pending_redeem_shares: u64,
) -> Result<u64> {
    let in_pool = position_share_amount(pool, position)?;
    supply
        .checked_sub(in_pool)
        .and_then(|h| h.checked_add(pending_redeem_shares))
        .ok_or_else(|| error!(BasketError::MathOverflow))
}

pub fn round_up_bps(amount: u64, bps: u16) -> Result<u64> {
    math::mul_div_u64(amount, bps as u64, BPS_TOTAL as u64, Rounding::Up)
}
