//! Meteora DLMM (`lb_clmm`) bindings.
//!
//! Generated from the published IDL at `idls/lb_clmm.json` (dlmm-sdk
//! `idls/dlmm.json`, program v0.12.0) with `declare_program!`. The program is
//! closed source; this gives typed CPI (`lb_clmm::cpi::add_liquidity_by_strategy2`
//! etc.) and zero-copy account loaders (`lb_clmm::accounts::LbPair`,
//! `PositionV2`, `BinArray`). Replaces the DAMM v2 sleeve per
//! `docs/change-order-liquidity-and-fees.md`.

use anchor_lang::prelude::*;

declare_program!(lb_clmm);

pub use lb_clmm::ID as LB_CLMM_ID;

/// Seeds and constants, from the IDL `constants` section.
pub mod seeds {
    pub const BIN_ARRAY: &[u8] = b"bin_array";
    pub const BIN_ARRAY_BITMAP: &[u8] = b"bitmap";
    pub const ORACLE: &[u8] = b"oracle";
    pub const POSITION: &[u8] = b"position";
    pub const EVENT_AUTHORITY: &[u8] = b"__event_authority";
}

/// Base key used in every customizable-permissionless pool PDA
/// (`ILM_BASE` in the SDK).
pub const ILM_BASE: Pubkey = pubkey!("MFGQxwAmB91SwuYX36okv2Qmdc9aMuHTwWGUrp4AtB1");

pub const MAX_BIN_PER_ARRAY: i32 = 70;
pub const DEFAULT_BIN_PER_POSITION: i32 = 70;
pub const POSITION_MAX_LENGTH: i32 = 1400;
/// Largest growth per `increase_position_length*` call (10 KB realloc cap / 112 B per bin).
pub const MAX_RESIZE_LENGTH: i32 = 91;
pub const MAX_BIN_STEP: u16 = 400;
pub const BASIS_POINT_MAX: u64 = 10_000;
/// Fee rates are expressed against this denominator.
pub const FEE_DENOMINATOR: u64 = 1_000_000_000;
pub const MAX_BASE_FEE: u64 = 100_000_000;
pub const MIN_BASE_FEE: u64 = 100_000;
/// Protocol share of swap fees on customizable pools (bps).
pub const ILM_PROTOCOL_SHARE: u16 = 2000;

/// `ActivationType` wire values.
pub const ACTIVATION_TYPE_SLOT: u8 = 0;
pub const ACTIVATION_TYPE_TIMESTAMP: u8 = 1;
/// `StrategyType` wire values used by `add_liquidity_by_strategy2`.
pub const STRATEGY_SPOT_BALANCED: u8 = 3;
/// `CollectFeeMode`: 0 = input token only (both sides accrue), 1 = only Y.
pub const COLLECT_FEE_MODE_INPUT_ONLY: u8 = 0;

pub fn event_authority() -> Pubkey {
    Pubkey::find_program_address(&[seeds::EVENT_AUTHORITY], &LB_CLMM_ID).0
}

/// `[ILM_BASE, min(mint_x, mint_y), max(mint_x, mint_y)]` (min first — the
/// opposite order from cp-amm's `cpool` seeds).
pub fn customizable_lb_pair(mint_x: &Pubkey, mint_y: &Pubkey) -> Pubkey {
    let (lo, hi) = sort_mints(mint_x, mint_y);
    Pubkey::find_program_address(&[ILM_BASE.as_ref(), lo.as_ref(), hi.as_ref()], &LB_CLMM_ID).0
}

/// `[lb_pair, mint]`.
pub fn reserve(lb_pair: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[lb_pair.as_ref(), mint.as_ref()], &LB_CLMM_ID).0
}

pub fn oracle(lb_pair: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::ORACLE, lb_pair.as_ref()], &LB_CLMM_ID).0
}

pub fn bin_array_bitmap_extension(lb_pair: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::BIN_ARRAY_BITMAP, lb_pair.as_ref()], &LB_CLMM_ID).0
}

/// `["bin_array", lb_pair, index_le_i64]`.
pub fn bin_array(lb_pair: &Pubkey, index: i64) -> Pubkey {
    Pubkey::find_program_address(
        &[seeds::BIN_ARRAY, lb_pair.as_ref(), &index.to_le_bytes()],
        &LB_CLMM_ID,
    )
    .0
}

/// `["position", lb_pair, base, lower_bin_id_le_i32, width_le_i32]`
/// (`initialize_position_pda`).
pub fn position_pda(lb_pair: &Pubkey, base: &Pubkey, lower_bin_id: i32, width: i32) -> Pubkey {
    Pubkey::find_program_address(
        &[
            seeds::POSITION,
            lb_pair.as_ref(),
            base.as_ref(),
            &lower_bin_id.to_le_bytes(),
            &width.to_le_bytes(),
        ],
        &LB_CLMM_ID,
    )
    .0
}

/// Bin array index that holds `bin_id` (floor division).
pub const fn bin_array_index(bin_id: i32) -> i64 {
    let idx = bin_id / MAX_BIN_PER_ARRAY;
    if bin_id < 0 && bin_id % MAX_BIN_PER_ARRAY != 0 {
        (idx - 1) as i64
    } else {
        idx as i64
    }
}

/// Inclusive range of bin array indices covering `[lower_bin_id, upper_bin_id]`.
pub const fn bin_array_range(lower_bin_id: i32, upper_bin_id: i32) -> (i64, i64) {
    (bin_array_index(lower_bin_id), bin_array_index(upper_bin_id))
}

/// `base_factor` and `base_fee_power_factor` for a base fee in bps:
/// `fee_rate = base_factor × bin_step × 10 × 10^power / 1e9`.
/// Mirrors the SDK's `computeBaseFactorFromFeeBps`.
pub fn base_factor_for_fee_bps(bin_step: u16, fee_bps: u64) -> Option<(u16, u8)> {
    // fee_rate (1e9) = fee_bps * 1e5
    let fee_rate = fee_bps.checked_mul(100_000)?;
    let mut power: u8 = 0;
    let denom = (bin_step as u64).checked_mul(10)?;
    loop {
        let scale = 10u64.checked_pow(power as u32)?;
        let base_factor = fee_rate / (denom * scale);
        if base_factor <= u16::MAX as u64 {
            if base_factor * denom * scale != fee_rate {
                return None;
            }
            return Some((base_factor as u16, power));
        }
        power = power.checked_add(1)?;
    }
}

/// `(min, max)` by byte order.
fn sort_mints<'a>(a: &'a Pubkey, b: &'a Pubkey) -> (&'a Pubkey, &'a Pubkey) {
    if a.to_bytes() > b.to_bytes() {
        (b, a)
    } else {
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_array_index_floors_negatives() {
        assert_eq!(bin_array_index(0), 0);
        assert_eq!(bin_array_index(69), 0);
        assert_eq!(bin_array_index(70), 1);
        assert_eq!(bin_array_index(-1), -1);
        assert_eq!(bin_array_index(-70), -1);
        assert_eq!(bin_array_index(-71), -2);
    }

    #[test]
    fn base_factor_round_trips() {
        // 1 % at bin step 100: 10000 * 100 * 10 = 1e7 = 1 % of 1e9.
        assert_eq!(base_factor_for_fee_bps(100, 100), Some((10_000, 0)));
        // 0.5 % at bin step 50.
        assert_eq!(base_factor_for_fee_bps(50, 50), Some((10_000, 0)));
        // 10 % at bin step 1 overflows u16 without the power factor.
        let (bf, p) = base_factor_for_fee_bps(1, 1000).unwrap();
        assert_eq!(bf as u64 * 10 * 10u64.pow(p as u32), 100_000_000);
    }
}
