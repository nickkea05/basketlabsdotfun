//! Meteora DAMM v2 (`cp-amm`) bindings.
//!
//! The client, account types and instruction builders are generated from the
//! published IDL at `idls/cp_amm.json` with `declare_program!`. This avoids
//! depending on the `cp-amm` crate, whose Anchor pin lags ours, while still
//! giving typed CPI (`cp_amm::cpi::initialize_customizable_pool` etc.) and
//! zero-copy account loaders (`cp_amm::accounts::Pool`, `Position`).

use anchor_lang::prelude::*;

declare_program!(cp_amm);

pub use cp_amm::ID as CP_AMM_ID;

/// Seeds, mirrored from `programs/cp-amm/src/constants.rs`.
pub mod seeds {
    pub const CUSTOMIZABLE_POOL: &[u8] = b"cpool";
    pub const TOKEN_VAULT: &[u8] = b"token_vault";
    pub const POOL_AUTHORITY: &[u8] = b"pool_authority";
    pub const POSITION: &[u8] = b"position";
    pub const POSITION_NFT_ACCOUNT: &[u8] = b"position_nft_account";
    pub const EVENT_AUTHORITY: &[u8] = b"__event_authority";
}

/// `MIN_SQRT_PRICE` / `MAX_SQRT_PRICE` from cp-amm: the full price range.
pub const MIN_SQRT_PRICE: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE: u128 = 79_226_673_521_066_979_257_578_248_091;

/// `FEE_DENOMINATOR` used by every cp-amm fee numerator.
pub const FEE_DENOMINATOR: u64 = 1_000_000_000;

/// `CollectFeeMode` wire values.
pub const COLLECT_FEE_MODE_BOTH: u8 = 0;
pub const COLLECT_FEE_MODE_ONLY_B: u8 = 1;

/// `ActivationType` wire values.
pub const ACTIVATION_TYPE_SLOT: u8 = 0;
pub const ACTIVATION_TYPE_TIMESTAMP: u8 = 1;

/// `BaseFeeMode` wire values (offset 26 of `BaseFeeParameters.data`).
pub const BASE_FEE_MODE_TIME_LINEAR: u8 = 0;
pub const BASE_FEE_MODE_TIME_EXPONENTIAL: u8 = 1;

pub fn pool_authority() -> Pubkey {
    Pubkey::find_program_address(&[seeds::POOL_AUTHORITY], &CP_AMM_ID).0
}

pub fn event_authority() -> Pubkey {
    Pubkey::find_program_address(&[seeds::EVENT_AUTHORITY], &CP_AMM_ID).0
}

/// `["cpool", max(mint_a, mint_b), min(mint_a, mint_b)]`.
pub fn customizable_pool(mint_a: &Pubkey, mint_b: &Pubkey) -> Pubkey {
    let (hi, lo) = if mint_a.to_bytes() > mint_b.to_bytes() {
        (mint_a, mint_b)
    } else {
        (mint_b, mint_a)
    };
    Pubkey::find_program_address(
        &[seeds::CUSTOMIZABLE_POOL, hi.as_ref(), lo.as_ref()],
        &CP_AMM_ID,
    )
    .0
}

pub fn token_vault(mint: &Pubkey, pool: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::TOKEN_VAULT, mint.as_ref(), pool.as_ref()], &CP_AMM_ID).0
}

pub fn position(position_nft_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::POSITION, position_nft_mint.as_ref()], &CP_AMM_ID).0
}

pub fn position_nft_account(position_nft_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[seeds::POSITION_NFT_ACCOUNT, position_nft_mint.as_ref()],
        &CP_AMM_ID,
    )
    .0
}

/// Borsh layout of `BorshFeeTimeScheduler`, packed into the 27-byte
/// `BaseFeeParameters.data`. A flat fee is a scheduler with zero periods.
pub fn flat_base_fee(fee_numerator: u64) -> cp_amm::types::BaseFeeParameters {
    time_scheduler_base_fee(fee_numerator, 0, 0, 0)
}

/// Linear time scheduler: starts at `cliff_fee_numerator`, drops by
/// `reduction_factor` every `period_frequency` seconds for `number_of_period`
/// periods. The end fee is `cliff - reduction * periods`.
pub fn time_scheduler_base_fee(
    cliff_fee_numerator: u64,
    number_of_period: u16,
    period_frequency: u64,
    reduction_factor: u64,
) -> cp_amm::types::BaseFeeParameters {
    let mut data = [0u8; 27];
    data[0..8].copy_from_slice(&cliff_fee_numerator.to_le_bytes());
    data[8..10].copy_from_slice(&number_of_period.to_le_bytes());
    data[10..18].copy_from_slice(&period_frequency.to_le_bytes());
    data[18..26].copy_from_slice(&reduction_factor.to_le_bytes());
    data[26] = BASE_FEE_MODE_TIME_LINEAR;
    cp_amm::types::BaseFeeParameters { data }
}

/// bps -> cp-amm fee numerator.
pub const fn bps_to_fee_numerator(bps: u64) -> u64 {
    bps * FEE_DENOMINATOR / 10_000
}
