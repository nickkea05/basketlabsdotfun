//! Integer math: creation-unit share rule (D4), sleeve split (D5), fee and
//! NAV arithmetic. Bin-price math for the DLMM sleeve lives in `dlmm.rs`.

use anchor_lang::prelude::*;
use ruint::aliases::U256;

use crate::constants::{BPS_TOTAL, SECONDS_PER_YEAR, SHARE_DECIMALS};
use crate::error::BasketError;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rounding {
    Up,
    Down,
}

fn overflow() -> anchor_lang::error::Error {
    error!(BasketError::MathOverflow)
}

pub fn u256(x: u128) -> U256 {
    U256::from(x)
}

pub fn to_u128(x: U256) -> Result<u128> {
    u128::try_from(x).map_err(|_| overflow())
}

pub fn to_u64(x: U256) -> Result<u64> {
    u64::try_from(x).map_err(|_| overflow())
}

/// `a * b / den` in 256 bits.
pub fn mul_div_u256(a: U256, b: U256, den: U256, round: Rounding) -> Result<U256> {
    require!(den != U256::ZERO, BasketError::MathOverflow);
    let prod = a.checked_mul(b).ok_or_else(overflow)?;
    let (q, r) = prod.div_rem(den);
    Ok(if round == Rounding::Up && r != U256::ZERO {
        q.checked_add(U256::from(1)).ok_or_else(overflow)?
    } else {
        q
    })
}

/// `a * b / den` for u64 with u128 intermediates.
pub fn mul_div_u64(a: u64, b: u64, den: u64, round: Rounding) -> Result<u64> {
    require!(den != 0, BasketError::MathOverflow);
    let prod = (a as u128).checked_mul(b as u128).ok_or_else(overflow)?;
    let q = prod / den as u128;
    let q = if round == Rounding::Up && prod % den as u128 != 0 { q + 1 } else { q };
    u64::try_from(q).map_err(|_| overflow())
}

pub fn bps(amount: u64, bps: u16) -> Result<u64> {
    mul_div_u64(amount, bps as u64, BPS_TOTAL as u64, Rounding::Down)
}

pub fn bps_up(amount: u64, bps: u16) -> Result<u64> {
    mul_div_u64(amount, bps as u64, BPS_TOTAL as u64, Rounding::Up)
}

// ---------------------------------------------------------------------------
// Creation-unit rule (D4)
// ---------------------------------------------------------------------------

/// `shares = min_i floor(deposit_i * outstanding / vault_i)`. Every vault
/// must be non-empty; a basket with an empty vault for a weighted position
/// cannot be priced in kind and must be seeded or rebalanced first.
pub fn shares_for_deposits(deposits: &[u64], vaults: &[u64], outstanding: u64) -> Result<u64> {
    require!(!deposits.is_empty() && deposits.len() == vaults.len(), BasketError::InvalidArgument);
    let mut best: Option<u64> = None;
    for (d, v) in deposits.iter().zip(vaults) {
        require!(*v > 0, BasketError::ZeroAmount);
        let s = mul_div_u64(*d, outstanding, *v, Rounding::Down)?;
        best = Some(best.map_or(s, |b| b.min(s)));
    }
    Ok(best.unwrap_or(0))
}

/// Component amount that `shares` out of `outstanding` is worth, rounded in
/// the basket's favour.
pub fn component_for_shares(shares: u64, vault: u64, outstanding: u64, round: Rounding) -> Result<u64> {
    if outstanding == 0 {
        return Ok(0);
    }
    mul_div_u64(shares, vault, outstanding, round)
}

// ---------------------------------------------------------------------------
// Sleeve (D5)
// ---------------------------------------------------------------------------

/// Treasury shares for `holder_shares` at sleeve ratio `r`:
/// `Y = X * r / (1 - r)`.
pub fn treasury_shares(holder_shares: u64, r_bps: u16) -> Result<u64> {
    require!(r_bps > 0 && r_bps < BPS_TOTAL, BasketError::SleeveOutOfBand);
    mul_div_u64(holder_shares, r_bps as u64, (BPS_TOTAL - r_bps) as u64, Rounding::Down)
}

/// Total value implied by the SOL leg: `D = sleeve_sol / r`.
pub fn implied_total_lamports(sleeve_lamports: u64, r_bps: u16) -> Result<u64> {
    require!(r_bps > 0, BasketError::SleeveOutOfBand);
    mul_div_u64(sleeve_lamports, BPS_TOTAL as u64, r_bps as u64, Rounding::Down)
}

/// NAV per whole share (lamports) of a fund worth `value_lamports` with
/// `shares` base units outstanding.
pub fn nav_per_share(value_lamports: u64, shares: u64) -> Result<u64> {
    if shares == 0 {
        return Ok(0);
    }
    mul_div_u64(value_lamports, 10u64.pow(SHARE_DECIMALS as u32), shares, Rounding::Down)
}

/// Management fee as share inflation (§4): mint `s` to the creator so they
/// hold fraction `f = bps · elapsed / year` of the enlarged fund,
/// `s = H·f / (1 − f)`, rounded down.
pub fn mgmt_fee_shares(holder_shares: u64, bps_per_year: u16, elapsed_s: i64) -> Result<u64> {
    if bps_per_year == 0 || elapsed_s <= 0 || holder_shares == 0 {
        return Ok(0);
    }
    let num = U256::from(bps_per_year) * U256::from(elapsed_s as u64);
    let den = U256::from(BPS_TOTAL) * U256::from(SECONDS_PER_YEAR as u64);
    if num >= den {
        return err!(BasketError::FeeOutOfBounds);
    }
    to_u64(mul_div_u256(U256::from(holder_shares), num, den - num, Rounding::Down)?)
}

/// Performance fee as share inflation (D12): on `nav > hwm` (both lamports
/// per whole share) the creator is owed `perf · (nav − hwm) · H` of value;
/// minting `s` shares at the post-dilution price gives
/// `s = perf·(nav − hwm)·H / (nav − perf·(nav − hwm))`, rounded down.
pub fn perf_fee_shares(holder_shares: u64, nav: u64, hwm: u64, perf_bps: u16) -> Result<u64> {
    if perf_bps == 0 || nav <= hwm || holder_shares == 0 {
        return Ok(0);
    }
    let profit = U256::from(nav - hwm) * U256::from(perf_bps); // × BPS_TOTAL
    let den = U256::from(nav) * U256::from(BPS_TOTAL) - profit;
    to_u64(mul_div_u256(U256::from(holder_shares), profit, den, Rounding::Down)?)
}

/// NAV per share after minting `minted` new shares against the same value.
pub fn diluted_nav(nav: u64, holder_shares: u64, minted: u64) -> Result<u64> {
    if minted == 0 {
        return Ok(nav);
    }
    let total = holder_shares.checked_add(minted).ok_or_else(|| error!(BasketError::MathOverflow))?;
    mul_div_u64(nav, holder_shares, total, Rounding::Down)
}

/// `liquidity * num / den`, rounded down.
pub fn liquidity_share(liquidity: u128, num: u64, den: u64) -> Result<u128> {
    if den == 0 {
        return Ok(0);
    }
    to_u128(mul_div_u256(u256(liquidity), U256::from(num), U256::from(den), Rounding::Down)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_unit_rule_takes_the_min_ratio() {
        // vaults 100 / 200, outstanding 1_000. Deposit 10 / 40 -> min ratio 10%.
        assert_eq!(shares_for_deposits(&[10, 40], &[100, 200], 1_000).unwrap(), 100);
        assert_eq!(component_for_shares(100, 100, 1_000, Rounding::Up).unwrap(), 10);
        assert_eq!(component_for_shares(100, 200, 1_000, Rounding::Up).unwrap(), 20);
    }

    #[test]
    fn sleeve_split() {
        // r = 25%: Y = X / 3
        assert_eq!(treasury_shares(3_000, 2_500).unwrap(), 1_000);
        assert_eq!(implied_total_lamports(250, 2_500).unwrap(), 1_000);
    }

}
