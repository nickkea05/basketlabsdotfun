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

/// SPL Memo v2, required by the `*2` remove/claim instructions.
pub const MEMO_PROGRAM_ID: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

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
pub const STRATEGY_SPOT_IMBALANCED: u8 = 6;
/// `CollectFeeMode`: 0 = input token only (both sides accrue), 1 = only Y.
pub const COLLECT_FEE_MODE_INPUT_ONLY: u8 = 0;
pub const COLLECT_FEE_MODE_ONLY_Y: u8 = 1;

/// Q64.64 one.
pub const ONE_Q64: u128 = 1u128 << 64;

// ---------------------------------------------------------------------------
// Raw account layouts (read by offset: a BinArray is 10 KB and a PositionV2
// 8 KB+, far too big to copy onto the SBF stack).
// ---------------------------------------------------------------------------

/// `PositionV2`: after the 8-byte discriminator: lb_pair(32) owner(32)
/// liquidity_shares[70]u128 reward_infos[70]×48 fee_infos[70]×48
/// lower_bin_id(i32) upper_bin_id(i32) … = 8112 bytes. Positions wider than
/// 70 bins append one `PositionBinData` (112 bytes, `liquidity_share: u128`
/// first) per extra bin (measured against the mainnet binary, progress
/// §6.1b).
pub const POSITION_FIXED_LEN: usize = 8112;
const POSITION_LB_PAIR: usize = 8;
const POSITION_OWNER: usize = 40;
const POSITION_SHARES: usize = 72;
const POSITION_LOWER: usize = 8 + 32 + 32 + 70 * 16 + 70 * 48 + 70 * 48;
const POSITION_UPPER: usize = POSITION_LOWER + 4;
pub const POSITION_BIN_DATA_LEN: usize = 112;

/// `BinArray`: index(i64) version(u8) pad(7) lb_pair(32) bins[70] after the
/// discriminator; each `Bin` is 144 bytes with amount_x(u64) amount_y(u64)
/// price(u128) liquidity_supply(u128) first.
pub const BIN_ARRAY_HEADER: usize = 8 + 8 + 1 + 7 + 32;
pub const BIN_LEN: usize = 144;
pub const BIN_ARRAY_LEN: usize = BIN_ARRAY_HEADER + 70 * BIN_LEN;

fn read_u64(data: &[u8], at: usize) -> Option<u64> {
    data.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
}

fn read_u128(data: &[u8], at: usize) -> Option<u128> {
    data.get(at..at + 16).map(|b| u128::from_le_bytes(b.try_into().unwrap()))
}

fn read_i32(data: &[u8], at: usize) -> Option<i32> {
    data.get(at..at + 4).map(|b| i32::from_le_bytes(b.try_into().unwrap()))
}

fn read_pubkey(data: &[u8], at: usize) -> Option<Pubkey> {
    data.get(at..at + 32).map(|b| Pubkey::try_from(b).unwrap())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionHeader {
    pub lb_pair: Pubkey,
    pub owner: Pubkey,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
}

pub fn position_header(data: &[u8]) -> Option<PositionHeader> {
    if data.len() < 8 + POSITION_FIXED_LEN || data[..8] != lb_clmm::accounts::PositionV2::DISCRIMINATOR[..] {
        return None;
    }
    Some(PositionHeader {
        lb_pair: read_pubkey(data, POSITION_LB_PAIR)?,
        owner: read_pubkey(data, POSITION_OWNER)?,
        lower_bin_id: read_i32(data, POSITION_LOWER)?,
        upper_bin_id: read_i32(data, POSITION_UPPER)?,
    })
}

/// Liquidity share of the position's `offset`-th bin (0 = `lower_bin_id`).
pub fn position_bin_share(data: &[u8], offset: usize) -> Option<u128> {
    if offset < DEFAULT_BIN_PER_POSITION as usize {
        read_u128(data, POSITION_SHARES + offset * 16)
    } else {
        let at = 8 + POSITION_FIXED_LEN + (offset - DEFAULT_BIN_PER_POSITION as usize) * POSITION_BIN_DATA_LEN;
        read_u128(data, at)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BinArrayHeader {
    pub index: i64,
    pub lb_pair: Pubkey,
}

pub fn bin_array_header(data: &[u8]) -> Option<BinArrayHeader> {
    if data.len() < BIN_ARRAY_LEN || data[..8] != lb_clmm::accounts::BinArray::DISCRIMINATOR[..] {
        return None;
    }
    Some(BinArrayHeader {
        index: read_u64(data, 8)? as i64,
        lb_pair: read_pubkey(data, 8 + 16)?,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BinAmounts {
    pub amount_x: u64,
    pub amount_y: u64,
    pub liquidity_supply: u128,
}

/// The `slot`-th bin (0..70) of a bin array.
pub fn bin_at(data: &[u8], slot: usize) -> Option<BinAmounts> {
    let at = BIN_ARRAY_HEADER + slot * BIN_LEN;
    Some(BinAmounts {
        amount_x: read_u64(data, at)?,
        amount_y: read_u64(data, at + 8)?,
        liquidity_supply: read_u128(data, at + 32)?,
    })
}

/// `LbPair.active_id` and `bin_step` from the raw account.
pub fn lb_pair_active_id(data: &[u8]) -> Option<(i32, u16)> {
    // StaticParameters (32) + VariableParameters (32) + bump(1) + bin_step_seed(2) + pair_type(1)
    let at = 8 + 32 + 32 + 1 + 2 + 1;
    let active_id = read_i32(data, at)?;
    let bin_step = data.get(at + 4..at + 6).map(|b| u16::from_le_bytes(b.try_into().unwrap()))?;
    Some((active_id, bin_step))
}

// ---------------------------------------------------------------------------
// Bin geometry
// ---------------------------------------------------------------------------

pub const fn array_lower_bin(index: i64) -> i32 {
    (index as i32) * MAX_BIN_PER_ARRAY
}

pub const fn array_upper_bin(index: i64) -> i32 {
    (index as i32) * MAX_BIN_PER_ARRAY + MAX_BIN_PER_ARRAY - 1
}

/// Whole bin arrays covering `[center − half_width, center + half_width]`,
/// at most `cap` of them; when the span needs more, the side farther from
/// the centre is dropped first (progress §6.4a: narrow the width, never
/// raise the cap).
pub fn backstop_arrays(center: i32, half_width: i32, cap: u8) -> (i64, i64) {
    let mut lo = bin_array_index(center - half_width);
    let mut hi = bin_array_index(center + half_width);
    while hi - lo + 1 > cap as i64 {
        let slack_lo = center - array_lower_bin(lo);
        let slack_hi = array_upper_bin(hi) - center;
        if slack_lo >= slack_hi {
            lo += 1;
        } else {
            hi -= 1;
        }
    }
    (lo, hi)
}

/// `(1 + bin_step / 10⁴)^bin_id` in Q64.64: lamports per X base unit at
/// `bin_id`. Same binary exponentiation the DLMM program uses.
pub fn bin_price_q64(bin_id: i32, bin_step: u16) -> Option<u128> {
    let bps = (bin_step as u128) << 64;
    let base = ONE_Q64.checked_add(bps / BASIS_POINT_MAX as u128)?;
    pow_q64(base, bin_id)
}

fn mul_shr_64(a: u128, b: u128) -> u128 {
    let p = ruint::aliases::U256::from(a) * ruint::aliases::U256::from(b);
    u128::try_from(p >> 64).unwrap_or(u128::MAX)
}

fn pow_q64(base: u128, exp: i32) -> Option<u128> {
    let mut invert = exp.is_negative();
    if exp == 0 {
        return Some(ONE_Q64);
    }
    let exp = exp.unsigned_abs();
    if exp >= 0x100000 {
        return None;
    }
    let mut squared_base = base;
    let mut result = ONE_Q64;
    if squared_base >= result {
        squared_base = u128::MAX / squared_base;
        invert = !invert;
    }
    for bit in 0..20 {
        if exp & (1u32 << bit) != 0 {
            result = mul_shr_64(result, squared_base);
        }
        squared_base = mul_shr_64(squared_base, squared_base);
    }
    if result == 0 {
        return None;
    }
    if invert {
        result = u128::MAX / result;
    }
    Some(result)
}

/// Lamports `amount_x` base units are worth at `price_q64`, rounded up.
pub fn lamports_for_x(amount_x: u64, price_q64: u128) -> Option<u64> {
    let p = ruint::aliases::U256::from(amount_x) * ruint::aliases::U256::from(price_q64);
    let (q, r) = p.div_rem(ruint::aliases::U256::from(ONE_Q64));
    let q = if r.is_zero() { q } else { q + ruint::aliases::U256::from(1u8) };
    u64::try_from(q).ok()
}

/// X base units `lamports` buy at `price_q64`, rounded down.
pub fn x_for_lamports(lamports: u64, price_q64: u128) -> Option<u64> {
    if price_q64 == 0 {
        return None;
    }
    let n = ruint::aliases::U256::from(lamports) << 64;
    u64::try_from(n / ruint::aliases::U256::from(price_q64)).ok()
}

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
/// (`initialize_position_pda`). `width` is the width the position is
/// created with (≤ `DEFAULT_BIN_PER_POSITION`), not its final width after
/// `increase_position_length2`; see `basket_position_pda`.
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

/// The PDA of a basket position over `[lower, upper]`: created one bin
/// array wide (or narrower) and extended, so the seed width is capped.
pub fn basket_position_pda(lb_pair: &Pubkey, base: &Pubkey, lower_bin_id: i32, upper_bin_id: i32) -> Pubkey {
    let width = (upper_bin_id - lower_bin_id + 1).min(DEFAULT_BIN_PER_POSITION);
    position_pda(lb_pair, base, lower_bin_id, width)
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
    fn raw_layout_offsets_match_generated_types() {
        assert_eq!(std::mem::size_of::<lb_clmm::accounts::PositionV2>(), POSITION_FIXED_LEN);
        assert_eq!(std::mem::size_of::<lb_clmm::accounts::BinArray>() + 8, BIN_ARRAY_LEN);
        assert_eq!(std::mem::size_of::<lb_clmm::types::Bin>(), BIN_LEN);
        assert_eq!(std::mem::size_of::<lb_clmm::types::PositionBinData>(), POSITION_BIN_DATA_LEN);
        assert_eq!(std::mem::offset_of!(lb_clmm::accounts::PositionV2, lower_bin_id) + 8, POSITION_LOWER);
        assert_eq!(std::mem::offset_of!(lb_clmm::accounts::PositionV2, liquidity_shares) + 8, POSITION_SHARES);
        assert_eq!(std::mem::offset_of!(lb_clmm::accounts::BinArray, bins) + 8, BIN_ARRAY_HEADER);
        assert_eq!(std::mem::offset_of!(lb_clmm::types::Bin, liquidity_supply), 32);
        assert_eq!(std::mem::offset_of!(lb_clmm::accounts::LbPair, active_id) + 8, 8 + 32 + 32 + 1 + 2 + 1);
        assert_eq!(std::mem::offset_of!(lb_clmm::accounts::LbPair, bin_step), std::mem::offset_of!(lb_clmm::accounts::LbPair, active_id) + 4);
    }

    #[test]
    fn bin_price_matches_float() {
        for (id, step) in [(0, 25), (1, 25), (100, 25), (-100, 25), (644, 25), (53, 100), (-53, 100), (2000, 50)] {
            let p = bin_price_q64(id, step).unwrap() as f64 / ONE_Q64 as f64;
            let want = (1.0 + step as f64 / 10_000.0).powi(id);
            assert!((p / want - 1.0).abs() < 1e-9, "id {id} step {step}: {p} vs {want}");
        }
        assert_eq!(bin_price_q64(0, 100), Some(ONE_Q64));
        assert_eq!(lamports_for_x(1_000, ONE_Q64), Some(1_000));
        assert_eq!(x_for_lamports(1_000, ONE_Q64), Some(1_000));
    }

    #[test]
    fn backstop_arrays_respect_cap() {
        // ±135 around 0 needs arrays −2..=1 (4) — fits.
        assert_eq!(backstop_arrays(0, 135, 4), (-2, 1));
        // ±135 around 60 needs −2..=2 (5) — drop the farther side.
        let (lo, hi) = backstop_arrays(60, 135, 4);
        assert_eq!(hi - lo + 1, 4);
        assert!(array_lower_bin(lo) <= 60 && array_upper_bin(hi) >= 60);
        assert_eq!((lo, hi), (-1, 2));
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
