//! Integer math: creation-unit share rule (D4), sleeve split (D5) and the
//! DAMM v2 liquidity <-> amount conversions (Q64.64 sqrt prices, 256-bit
//! intermediates). The pool formulas mirror
//! `cp-amm/src/liquidity_handler/concentrated_liquidity.rs` so the amounts we
//! predict are the amounts the pool will pull.

use anchor_lang::prelude::*;
use ruint::aliases::U256;

use crate::constants::{BPS_TOTAL, SECONDS_PER_YEAR, SHARE_DECIMALS};
use crate::damm::{MAX_SQRT_PRICE, MIN_SQRT_PRICE};
use crate::error::BasketError;
use crate::state::PriceRange;

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

// ---------------------------------------------------------------------------
// DAMM v2 concentrated-liquidity conversions
// ---------------------------------------------------------------------------

/// `Δa = L * (√P_upper - √P_lower) / (√P_upper * √P_lower)`
pub fn delta_a(lower: u128, upper: u128, liquidity: u128, round: Rounding) -> Result<u64> {
    require!(upper >= lower && lower > 0, BasketError::MathOverflow);
    let den = u256(lower).checked_mul(u256(upper)).ok_or_else(overflow)?;
    let r = mul_div_u256(u256(liquidity), u256(upper - lower), den, round)?;
    to_u64(r)
}

/// `Δb = L * (√P_upper - √P_lower) / 2^128`
pub fn delta_b(lower: u128, upper: u128, liquidity: u128, round: Rounding) -> Result<u64> {
    require!(upper >= lower, BasketError::MathOverflow);
    let prod = u256(liquidity).checked_mul(u256(upper - lower)).ok_or_else(overflow)?;
    let den = U256::from(1u8) << 128;
    let (q, rem) = prod.div_rem(den);
    let q = if round == Rounding::Up && rem != U256::ZERO { q + U256::from(1u8) } else { q };
    to_u64(q)
}

/// Largest `L` such that `delta_b(min, price, L) <= amount_b`.
pub fn liquidity_from_b(amount_b: u64, sqrt_min: u128, sqrt_price: u128) -> Result<u128> {
    require!(sqrt_price > sqrt_min, BasketError::MathOverflow);
    let num = U256::from(amount_b) << 128;
    to_u128(num / u256(sqrt_price - sqrt_min))
}

/// Largest `L` such that `delta_a(price, max, L) <= amount_a`.
pub fn liquidity_from_a(amount_a: u64, sqrt_price: u128, sqrt_max: u128) -> Result<u128> {
    require!(sqrt_max > sqrt_price, BasketError::MathOverflow);
    let num = U256::from(amount_a)
        .checked_mul(u256(sqrt_price))
        .ok_or_else(overflow)?
        .checked_mul(u256(sqrt_max))
        .ok_or_else(overflow)?;
    to_u128(num / u256(sqrt_max - sqrt_price))
}

/// Integer square root.
pub fn isqrt(n: U256) -> U256 {
    if n < U256::from(2u8) {
        return n;
    }
    let mut x = U256::from(1u8) << (n.bit_len().div_ceil(2));
    loop {
        let y = (x + n / x) >> 1;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Q64.64 `sqrt(amount_b / amount_a)`: the pool price that makes `amount_a`
/// shares worth `amount_b` lamports.
pub fn sqrt_price_from_amounts(amount_b: u64, amount_a: u64) -> Result<u128> {
    require!(amount_a > 0 && amount_b > 0, BasketError::ZeroAmount);
    let ratio = (U256::from(amount_b) << 128) / U256::from(amount_a);
    let p = to_u128(isqrt(ratio))?;
    require!(p >= MIN_SQRT_PRICE && p <= MAX_SQRT_PRICE, BasketError::MathOverflow);
    Ok(p)
}

/// `sqrt(0.5)` and `sqrt(8)` as `x / 1e16` multipliers.
const SQRT_HALF_E16: u128 = 7_071_067_811_865_476;
const SQRT_EIGHT_E16: u128 = 28_284_271_247_461_903;
const E16: u128 = 10_000_000_000_000_000;

/// Pool sqrt price bounds for a profile, relative to the seed sqrt price.
pub fn sqrt_price_bounds(sqrt_price: u128, range: PriceRange) -> Result<(u128, u128)> {
    let scale = |mult: u128| -> Result<u128> {
        to_u128(mul_div_u256(u256(sqrt_price), u256(mult), u256(E16), Rounding::Down)?)
    };
    let (lo, hi) = match range {
        PriceRange::Full => (MIN_SQRT_PRICE, MAX_SQRT_PRICE),
        PriceRange::FloorOnly => (scale(SQRT_HALF_E16)?, MAX_SQRT_PRICE),
        PriceRange::Bounded => (scale(SQRT_HALF_E16)?, scale(SQRT_EIGHT_E16)?),
    };
    let lo = lo.clamp(MIN_SQRT_PRICE, MAX_SQRT_PRICE);
    let hi = hi.clamp(MIN_SQRT_PRICE, MAX_SQRT_PRICE);
    require!(lo < sqrt_price && sqrt_price < hi, BasketError::MathOverflow);
    Ok((lo, hi))
}

/// Given a share amount `a` to deposit at the pool's current state, the
/// liquidity delta and the exact `(a, b)` the pool will pull (rounded up,
/// as `add_liquidity` does).
pub struct AddQuote {
    pub liquidity: u128,
    pub amount_a: u64,
    pub amount_b: u64,
}

pub fn quote_add_for_a(
    amount_a: u64,
    sqrt_min: u128,
    sqrt_price: u128,
    sqrt_max: u128,
) -> Result<AddQuote> {
    let liquidity = liquidity_from_a(amount_a, sqrt_price, sqrt_max)?;
    require!(liquidity > 0, BasketError::ZeroAmount);
    let a = delta_a(sqrt_price, sqrt_max, liquidity, Rounding::Up)?;
    let b = delta_b(sqrt_min, sqrt_price, liquidity, Rounding::Up)?;
    Ok(AddQuote { liquidity, amount_a: a, amount_b: b })
}

/// Lamports `shares` are worth at the pool's marginal price
/// (`price = sqrt_price² / 2^128`), rounded up.
pub fn lamports_for_shares(shares: u64, sqrt_price: u128) -> Result<u64> {
    let p2 = u256(sqrt_price).checked_mul(u256(sqrt_price)).ok_or_else(overflow)?;
    to_u64(mul_div_u256(U256::from(shares), p2, U256::from(1u8) << 128, Rounding::Up)?)
}

/// Sleeve deposit quote (D5). The SOL leg `target_b` (= r·D) is the primary
/// quantity: liquidity is sized so the position absorbs exactly that much
/// SOL at the current marginal price, and the treasury-share side follows
/// from the range geometry (≈2.2× the nominal `X·r/(1−r)` for `[0.5×, 8×]`,
/// 1× for full range). That keeps marginal price = NAV regardless of range.
///
/// Edges cp-amm lets the price sit on: at the ceiling the position is all
/// SOL (no share side); at the floor it is all shares, so the nominal share
/// count is added with no SOL leg.
pub fn sleeve_quote(
    target_b: u64,
    nominal_a: u64,
    sqrt_min: u128,
    sqrt_price: u128,
    sqrt_max: u128,
) -> Result<AddQuote> {
    if sqrt_price <= sqrt_min {
        if nominal_a == 0 {
            return Ok(AddQuote { liquidity: 0, amount_a: 0, amount_b: 0 });
        }
        let liquidity = liquidity_from_a(nominal_a, sqrt_min, sqrt_max)?;
        let a = delta_a(sqrt_min, sqrt_max, liquidity, Rounding::Up)?;
        return Ok(AddQuote { liquidity, amount_a: a, amount_b: 0 });
    }
    if target_b == 0 {
        return Ok(AddQuote { liquidity: 0, amount_a: 0, amount_b: 0 });
    }
    let p = sqrt_price.min(sqrt_max);
    let liquidity = liquidity_from_b(target_b, sqrt_min, p)?;
    require!(liquidity > 0, BasketError::ZeroAmount);
    let a = if p >= sqrt_max { 0 } else { delta_a(p, sqrt_max, liquidity, Rounding::Up)? };
    let b = delta_b(sqrt_min, p, liquidity, Rounding::Up)?;
    Ok(AddQuote { liquidity, amount_a: a, amount_b: b })
}

/// Amounts a position of `liquidity` holds at the current price (rounded
/// down, what `remove_liquidity` pays out).
pub fn position_amounts(
    liquidity: u128,
    sqrt_min: u128,
    sqrt_price: u128,
    sqrt_max: u128,
) -> Result<(u64, u64)> {
    if liquidity == 0 {
        return Ok((0, 0));
    }
    let a = delta_a(sqrt_price.max(sqrt_min), sqrt_max, liquidity, Rounding::Down)?;
    let b = delta_b(sqrt_min, sqrt_price.min(sqrt_max), liquidity, Rounding::Down)?;
    Ok((a, b))
}

/// Shares a position of `liquidity` holds, rounded up: cp-amm pulls deposits
/// rounded up, so this is the exact count after `add_liquidity`, and it keeps
/// `supply − in_pool` from over-counting holder shares by a unit.
pub fn position_shares(liquidity: u128, sqrt_min: u128, sqrt_price: u128, sqrt_max: u128) -> Result<u64> {
    if liquidity == 0 || sqrt_price >= sqrt_max {
        return Ok(0);
    }
    delta_a(sqrt_price.max(sqrt_min), sqrt_max, liquidity, Rounding::Up)
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

    #[test]
    fn pool_quote_round_trips() {
        // 1 SOL against 1_000 shares (6 dp) -> price 1e9 / 1e9 = 1 lamport/unit
        let a = 1_000_000_000u64;
        let b = 1_000_000_000u64;
        let p = sqrt_price_from_amounts(b, a).unwrap();
        assert_eq!(p, 1u128 << 64);
        let (lo, hi) = sqrt_price_bounds(p, PriceRange::Bounded).unwrap();
        let q = quote_add_for_a(a, lo, p, hi).unwrap();
        assert!(q.amount_a <= a && q.amount_a + 2 >= a, "a={} q.a={}", a, q.amount_a);
        assert!(q.amount_b > 0);
        let (pa, pb) = position_amounts(q.liquidity, lo, p, hi).unwrap();
        assert!(pa <= q.amount_a && pa + 2 >= q.amount_a);
        assert!(pb <= q.amount_b && pb + 2 >= q.amount_b);
    }

    #[test]
    fn full_range_quote() {
        let p = sqrt_price_from_amounts(500_000_000, 2_000_000_000).unwrap();
        let (lo, hi) = sqrt_price_bounds(p, PriceRange::Full).unwrap();
        assert_eq!((lo, hi), (MIN_SQRT_PRICE, MAX_SQRT_PRICE));
        let q = quote_add_for_a(2_000_000_000, lo, p, hi).unwrap();
        // At full range with price sqrt(b/a), b side ≈ a * price
        assert!((q.amount_b as i128 - 500_000_000).abs() < 1_000, "b={}", q.amount_b);
    }

    #[test]
    fn sleeve_quote_absorbs_the_sol_leg_at_nav_price() {
        // X = 3_000 shares, r = 25% -> nominal Y = 1_000; SOL leg 0.3 SOL.
        let y = treasury_shares(3_000_000_000, 2_500).unwrap();
        let b = 300_000_000u64;
        let p = sqrt_price_from_amounts(b, y).unwrap();
        for (range, ratio) in [(PriceRange::Full, 1.0), (PriceRange::Bounded, 2.207), (PriceRange::FloorOnly, 3.414)] {
            let (lo, hi) = sqrt_price_bounds(p, range).unwrap();
            let q = sleeve_quote(b, y, lo, p, hi).unwrap();
            assert!(q.amount_b <= b && q.amount_b + 2 >= b, "{range:?}: b={}", q.amount_b);
            let got = q.amount_a as f64 / y as f64;
            assert!((got - ratio).abs() < 0.01, "{range:?}: a/y = {got}");
            // Marginal price unchanged: shares valued at p equal the nominal leg.
            assert!(lamports_for_shares(y, p).unwrap().abs_diff(b) <= 2);
        }
    }

    #[test]
    fn isqrt_exact() {
        assert_eq!(isqrt(U256::from(0u8)), U256::from(0u8));
        assert_eq!(isqrt(U256::from(1u8)), U256::from(1u8));
        assert_eq!(isqrt(U256::from(15u8)), U256::from(3u8));
        assert_eq!(isqrt(U256::from(16u8)), U256::from(4u8));
        assert_eq!(isqrt(U256::from(1u8) << 128), U256::from(1u8) << 64);
    }
}
