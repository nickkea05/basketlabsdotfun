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

/// Sleeve / pool-preset profile (change order §2.1, progress §6.4a). Fixed
/// baskets pick one of two profiles in the payload (`fixed_kind`); the
/// other types map 1:1.
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
/// Swap venues `execute_swap` / `settle_fees` / `execute_buyback` may route through.
pub const MAX_SWAP_PROGRAMS: usize = 4;
/// Jupiter v6 aggregator.
pub const JUPITER_V6_ID: Pubkey = Pubkey::from_str_const("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");
/// Default slack on the weight-derived per-position sell cap (5%).
pub const DEFAULT_REBALANCE_TOLERANCE_BPS: u16 = 500;

/// Fee split, creator / buyback / team / deployer prizes (change order §4).
/// Initial `Config` values; every fee event is split this way.
pub const DEFAULT_CREATOR_SPLIT_BPS: u16 = 2_000;
pub const DEFAULT_BUYBACK_SPLIT_BPS: u16 = 5_000;
pub const DEFAULT_TEAM_SPLIT_BPS: u16 = 2_500;
pub const DEFAULT_PRIZE_SPLIT_BPS: u16 = 500;

pub const DEFAULT_MINT_FEE_BPS: u16 = 100;
pub const DEFAULT_REDEEM_FEE_BPS: u16 = 25;
pub const DEFAULT_MGMT_FEE_BPS_PER_YEAR: u16 = 100;
pub const DEFAULT_PERF_FEE_BPS: u16 = 1_000;
pub const DEFAULT_PERF_FEE_MAX_BPS: u16 = 2_000;

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;
pub const DEFAULT_CREATION_FEE_LAMPORTS: u64 = LAMPORTS_PER_SOL / 10;
pub const DEFAULT_MIN_SEED_LAMPORTS: u64 = LAMPORTS_PER_SOL;
pub const DEFAULT_MIN_MINT_LAMPORTS: u64 = LAMPORTS_PER_SOL / 10;

/// Change order §2.2: the deployer's refundable launch deposit. Pool and
/// bin-array rent (non-refundable) is spent from it; the rest comes back at
/// `close_basket`.
pub const DEFAULT_DEPOSIT_LAMPORTS: u64 = LAMPORTS_PER_SOL;

/// Change order §2.3 / progress §6.4a Q12: mint window default and bounds.
pub const DEFAULT_MINT_WINDOW_S: i64 = 48 * 60 * 60;
pub const DEFAULT_MIN_MINT_WINDOW_S: i64 = 24 * 60 * 60;
pub const DEFAULT_MAX_MINT_WINDOW_S: i64 = 72 * 60 * 60;

/// D5: front-loaded sleeve, 25% until the pool's SOL side reaches 20 SOL.
pub const DEFAULT_STEP_R_BPS: u16 = 2_500;
pub const DEFAULT_STEP_THRESHOLD_LAMPORTS: u64 = 20 * LAMPORTS_PER_SOL;

/// Progress §6.4a: tight re-centre trigger (70 % of the half-width from the
/// centre) and the minimum interval between triggered re-centres of one
/// basket; an out-of-range active bin may always be re-centred.
pub const DEFAULT_RECENTER_TRIGGER_BPS: u16 = 7_000;
pub const DEFAULT_RECENTER_MIN_INTERVAL_S: i64 = 5 * 60;
/// Share of every sleeve deposit that goes to the backstop (Q5/Q6).
pub const DEFAULT_BACKSTOP_SLICE_BPS: u16 = 2_000;
/// Backstop width cap in bin arrays (rent stays under ~0.3 SOL).
pub const MAX_BACKSTOP_ARRAYS: u8 = 4;

/// Change order §5: deployer prize pool.
pub const DEFAULT_PRIZE_EPOCH_S: i64 = 14 * 24 * 60 * 60;
pub const DEFAULT_PRIZE_MAX_RECIPIENTS: u8 = 3;
pub const MAX_PRIZE_RECIPIENTS: usize = 10;
pub const DEFAULT_PRIZE_RANK_SPLIT_BPS: [u16; MAX_PRIZE_RECIPIENTS] = [5_000, 3_000, 2_000, 0, 0, 0, 0, 0, 0, 0];
/// Q9 guardrail: a basket must have claimed at least this much pool fee SOL
/// in the epoch for its deployer to be paid (on-chain proxy for volume).
pub const DEFAULT_MIN_EPOCH_POOL_FEES: u64 = LAMPORTS_PER_SOL / 10;

/// Creator tiers (Q10): keeper-attested leaderboard rank at create.
pub const TIER_COUNT: usize = 4;

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
/// Holder shares (base units, 1e-6 share) above the FeeVault's own stash
/// that still count as "nobody holds anything" for the close crank; covers
/// liquidity→amount rounding in the pool positions.
pub const CLOSE_DUST_SHARES: u64 = 10;

/// Creator lock duration bounds.
pub const MIN_CREATOR_LOCK_S: i64 = 24 * 60 * 60;
pub const MAX_CREATOR_LOCK_S: i64 = 4 * 365 * 24 * 60 * 60;

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
    pub const LOCK: &[u8] = b"lock";
    pub const REDEMPTION: &[u8] = b"redemption";
    /// Global SOL accrual for the $BSKT buyback (change order §4).
    pub const BUYBACK: &[u8] = b"buyback";
    /// Global SOL accrual for deployer prizes (change order §5).
    pub const PRIZE: &[u8] = b"prize";
}
