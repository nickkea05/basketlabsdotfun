//! Protocol constants. Anything marked `#[constant]` is exported in the IDL
//! and must agree with `packages/core/src/constants.js`.
//!
//! Numbers that the confirmation doc calls "defaults in `Config`, changeable
//! for new baskets only" live in `Config`, not here; the values below are the
//! initial values `initialize_config` writes when the admin passes none.

use anchor_lang::prelude::*;

/// Weights are expressed in basis points and must sum to exactly this.
#[constant]
pub const BPS_TOTAL: u16 = 10_000;

/// Basket kinds, matching `BasketType` in `packages/core`.
#[constant]
pub const BASKET_TYPE_FIXED: u8 = 0;
#[constant]
pub const BASKET_TYPE_MIRROR: u8 = 1;
#[constant]
pub const BASKET_TYPE_MANAGED: u8 = 2;
#[constant]
pub const BASKET_TYPE_STRATEGY: u8 = 3;

/// Sleeve / fee profile (§3 of the confirmation doc). Fixed baskets pick one
/// of two profiles in the signed payload (`fixed_kind`); the other types map
/// 1:1.
#[constant]
pub const PROFILE_FIXED_INDEX: u8 = 0;
#[constant]
pub const PROFILE_MIRROR: u8 = 1;
#[constant]
pub const PROFILE_STRATEGY: u8 = 2;
#[constant]
pub const PROFILE_MANAGED: u8 = 3;
#[constant]
pub const PROFILE_FIXED_MEME: u8 = 4;
pub const PROFILE_COUNT: usize = 5;

/// `fixed_kind` in `CreateBasketArgs`, Fixed baskets only.
#[constant]
pub const FIXED_KIND_INDEX: u8 = 0;
#[constant]
pub const FIXED_KIND_MEME: u8 = 1;

/// Host weighting, Mirror only. Matches `HostWeighting` in `packages/core`.
#[constant]
pub const HOST_WEIGHTING_EQUAL: u8 = 0;
#[constant]
pub const HOST_WEIGHTING_VALUE: u8 = 1;

/// Only Fixed baskets cap the asset count (D20).
#[constant]
pub const MAX_ASSETS_FIXED: u16 = 20;

/// String limits (bytes). `usize` is not an IDL type, so these are mirrored
/// in `packages/core/src/constants.js` by hand.
pub const NAME_MAX: usize = 32;
pub const SYMBOL_MAX: usize = 10;
pub const URI_MAX: usize = 200;

/// Share mint decimals (D1: classic SPL).
#[constant]
pub const SHARE_DECIMALS: u8 = 6;

/// Keeper set size cap (D17: "small rotatable signer set").
pub const MAX_KEEPERS: usize = 8;

/// Fee split, holders / creator / protocol (§4). Initial `Config` values.
pub const DEFAULT_HOLDER_SPLIT_BPS: u16 = 4_000;
pub const DEFAULT_CREATOR_SPLIT_BPS: u16 = 3_000;
pub const DEFAULT_PROTOCOL_SPLIT_BPS: u16 = 3_000;

pub const DEFAULT_MINT_FEE_BPS: u16 = 100;
pub const DEFAULT_REDEEM_FEE_BPS: u16 = 25;
pub const DEFAULT_MGMT_FEE_BPS_PER_YEAR: u16 = 100;
pub const DEFAULT_PERF_FEE_BPS: u16 = 1_000;
pub const DEFAULT_PERF_FEE_MAX_BPS: u16 = 2_000;

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
pub const DEFAULT_CREATION_FEE_LAMPORTS: u64 = LAMPORTS_PER_SOL / 10;
pub const DEFAULT_MIN_SEED_LAMPORTS: u64 = LAMPORTS_PER_SOL;
pub const DEFAULT_MIN_MINT_LAMPORTS: u64 = LAMPORTS_PER_SOL / 10;

/// D5: front-loaded sleeve, 25% until the pool's SOL side reaches 20 SOL.
pub const DEFAULT_STEP_R_BPS: u16 = 2_500;
pub const DEFAULT_STEP_THRESHOLD_LAMPORTS: u64 = 20 * LAMPORTS_PER_SOL;

/// D16: `Open` baskets graduate at 50 SOL on the pool's SOL side.
pub const DEFAULT_GRADUATION_LAMPORTS: u64 = 50 * LAMPORTS_PER_SOL;

/// Managed rules (§9 defaults): 24 h timelock, 30% turnover per 7 d,
/// monthly crystallization.
pub const DEFAULT_MANAGED_TIMELOCK_S: i64 = 24 * 60 * 60;
pub const DEFAULT_TURNOVER_CAP_BPS: u16 = 3_000;
pub const DEFAULT_TURNOVER_WINDOW_S: i64 = 7 * 24 * 60 * 60;
pub const DEFAULT_CRYSTALLIZE_PERIOD_S: i64 = 30 * 24 * 60 * 60;

/// Rebalance window after a Mirror/Strategy `submit_book`.
pub const DEFAULT_REBALANCE_WINDOW_S: i64 = 6 * 60 * 60;

/// `close_basket`: zero supply and no activity for this long.
pub const DEFAULT_CLOSE_IDLE_S: i64 = 14 * 24 * 60 * 60;

/// D13 launch fee scheduler for Window/Closed baskets: starts at
/// `cliff`, linear decay to the profile's pool fee over `periods × freq`.
pub const DEFAULT_SCHEDULER_CLIFF_BPS: u16 = 5_000;
pub const DEFAULT_SCHEDULER_PERIODS: u16 = 60;
pub const DEFAULT_SCHEDULER_PERIOD_S: u64 = 60;

pub const SECONDS_PER_YEAR: i64 = 365 * 24 * 60 * 60;

/// PDA seeds.
pub mod seeds {
    pub const CONFIG: &[u8] = b"config";
    pub const WHITELIST: &[u8] = b"whitelist";
    pub const SHARE_MINT: &[u8] = b"mint";
    pub const BASKET: &[u8] = b"basket";
    pub const SHARE_AUTH: &[u8] = b"share_auth";
    pub const POSITION: &[u8] = b"position";
    pub const FEES: &[u8] = b"fees";
    pub const PENDING: &[u8] = b"pending";
    pub const CLAIM: &[u8] = b"claim";
    pub const REWARDS: &[u8] = b"rewards";
    pub const LOCK: &[u8] = b"lock";
    pub const REDEMPTION: &[u8] = b"redemption";
}
