//! `Basket` — one per launch — and the creator-signed payload that creates it.

use anchor_lang::prelude::*;

use crate::constants::*;
use crate::error::BasketError;
use crate::state::config::{ManagedRules, PriceRange};

/// D6. Time-based, immutable after creation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub enum MintGate {
    Open,
    WindowUntil { close_ts: i64 },
    Closed,
}

/// D6. Re-opens the gate for `open_s` seconds every `period_s` seconds
/// starting at `anchor_ts`. Only consulted when the gate is not `Open`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct ReopenSchedule {
    pub anchor_ts: i64,
    pub period_s: i64,
    pub open_s: i64,
}

impl ReopenSchedule {
    pub fn validate(&self) -> bool {
        self.period_s > 0 && self.open_s > 0 && self.open_s < self.period_s
    }

    pub fn is_open_at(&self, now: i64) -> bool {
        if now < self.anchor_ts {
            return false;
        }
        (now - self.anchor_ts) % self.period_s < self.open_s
    }
}

/// D19 domain separator: binds the signed payload to this program, this
/// cluster and one creator nonce. The nonce also seeds the share mint PDA,
/// so replaying a payload derives an existing mint and fails.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain {
    pub program_id: Pubkey,
    pub cluster: u8,
    pub nonce: u64,
}

/// What the creator signs at "Launch" (frontend `buildLaunchParams()` plus
/// the fields the confirmation doc added). The first buyer submits it
/// verbatim inside `create_basket`; the ed25519 precompile instruction in the
/// same transaction covers exactly `borsh(CreateBasketArgs)`.
///
/// The asset list itself is not in the payload; `book_hash` commits to it
/// (chain hash, see `Basket::chain_hash`) and `add_positions` delivers it.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateBasketArgs {
    pub domain: Domain,
    pub creator: Pubkey,
    pub basket_type: u8,
    /// `FIXED_KIND_*`, Fixed only; 0 otherwise.
    pub fixed_kind: u8,
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub asset_count: u16,
    /// Chain hash over the ordered `(mint, weight_bps)` list.
    pub book_hash: [u8; 32],
    /// Mirror: sha256 over the ordered host wallet list. Zero otherwise.
    pub hosts_hash: [u8; 32],
    /// Mirror: `HOST_WEIGHTING_*`. 0 otherwise.
    pub host_weighting: u8,
    /// Strategy: sha256 of the assembled module. Zero otherwise.
    pub strategy_hash: [u8; 32],
    /// Sleeve ratio after the step phase, within the profile band (§3).
    pub sleeve_r_bps: u16,
    /// Managed: performance fee within `[0, perf_fee_max_bps]`. 0 otherwise.
    pub perf_fee_bps: u16,
    pub gate: MintGate,
    pub schedule: Option<ReopenSchedule>,
}

/// `seed` (§5): the first buy. `deposits[i]` is the amount of position `i`
/// (book order) delivered in kind; `initial_shares` is the gross X the buyer
/// is paying for (the program never prices, D4); `sleeve_lamports` is the
/// SOL leg, `r·D` by the client's split. Pool opens at `sleeve / Y`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SeedArgs {
    pub deposits: Vec<u64>,
    pub initial_shares: u64,
    pub sleeve_lamports: u64,
}

/// `mint` (§5). Shares follow the creation-unit rule over `deposits`; the
/// pool then dictates the SOL leg. `min_shares_out` and
/// `max_sleeve_lamports` are the buyer's slippage bounds.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct MintArgs {
    pub deposits: Vec<u64>,
    pub min_shares_out: u64,
    pub max_sleeve_lamports: u64,
}

/// Per-basket fee schedule, copied from `Config` at creation (D11, §4).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct FeeSchedule {
    pub mint_fee_bps: u16,
    pub redeem_fee_bps: u16,
    pub holder_split_bps: u16,
    pub creator_split_bps: u16,
    pub protocol_split_bps: u16,
    /// Managed only, else 0.
    pub mgmt_fee_bps_per_year: u16,
    /// Managed only, else 0.
    pub perf_fee_bps: u16,
    /// DAMM v2 base fee after any launch scheduler.
    pub pool_fee_bps: u16,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct SleeveParams {
    pub step_r_bps: u16,
    pub step_threshold_lamports: u64,
    /// After the step phase.
    pub r_bps: u16,
    pub price_range: PriceRange,
}

impl Default for PriceRange {
    fn default() -> Self {
        PriceRange::Full
    }
}

/// State of an in-flight Mirror/Strategy/Managed rebalance.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct Rebalance {
    pub active: bool,
    /// Chain hash of the target book.
    pub target_hash: [u8; 32],
    pub target_count: u16,
    pub window_end_ts: i64,
    /// Managed turnover accounting: bps of NAV turned over in the window.
    pub turnover_window_start_ts: i64,
    pub turnover_used_bps: u16,
}

#[account]
#[derive(InitSpace, Debug)]
pub struct Basket {
    pub bump: u8,
    pub share_auth_bump: u8,
    pub basket_type: u8,
    /// `PROFILE_*`
    pub profile: u8,
    pub host_weighting: u8,
    pub complete: bool,
    pub seeded: bool,
    pub creator: Pubkey,
    /// Paid the rent; refunded by `close_basket`.
    pub payer: Pubkey,
    pub share_mint: Pubkey,
    pub nonce: u64,
    #[max_len(NAME_MAX)]
    pub name: String,
    #[max_len(SYMBOL_MAX)]
    pub symbol: String,
    #[max_len(URI_MAX)]
    pub uri: String,

    pub asset_count: u16,
    pub position_count: u16,
    /// Signed commitment to the book (`CreateBasketArgs.book_hash`).
    pub book_hash: [u8; 32],
    /// Running chain hash while positions are being added.
    pub book_acc: [u8; 32],
    /// Running weight sum while positions are being added.
    pub weight_acc: u16,
    pub hosts_hash: [u8; 32],
    pub strategy_hash: [u8; 32],

    pub fees: FeeSchedule,
    pub managed: ManagedRules,
    pub sleeve: SleeveParams,
    pub gate: MintGate,
    pub schedule: Option<ReopenSchedule>,

    /// DAMM v2 pool, position NFT mint and position account. Zero until
    /// `seed`.
    pub pool: Pubkey,
    pub position_nft_mint: Pubkey,
    pub pool_position: Pubkey,

    /// Shares burned by `redeem_begin` whose components have not all been
    /// paid out yet. Added back to the holder-share denominator so the
    /// per-share vault ratio stays exact during a multi-tx redemption.
    pub pending_redeem_shares: u64,

    /// D12: fund-level high-water mark in lamports per whole share
    /// (`SHARE_DECIMALS`), and the last crystallization / management accrual.
    pub hwm_nav_lamports: u64,
    pub last_crystallized_ts: i64,
    pub last_mgmt_accrual_ts: i64,

    pub rebalance: Rebalance,

    pub created_at: i64,
    pub last_activity_at: i64,
}

impl Basket {
    /// `acc' = sha256(acc || mint || weight_bps_le)`; `acc0 = [0; 32]`.
    pub fn chain_hash(acc: &[u8; 32], mint: &Pubkey, weight_bps: u16) -> [u8; 32] {
        solana_sha256_hasher::hashv(&[acc, mint.as_ref(), &weight_bps.to_le_bytes()])
            .to_bytes()
    }

    pub fn is_managed(&self) -> bool {
        self.basket_type == BASKET_TYPE_MANAGED
    }

    /// Mirror, Managed and Strategy books can change; Fixed cannot (D20).
    pub fn book_is_mutable(&self) -> bool {
        self.basket_type != BASKET_TYPE_FIXED
    }

    /// D13: launch fee scheduler only for non-Open gates.
    pub fn uses_launch_scheduler(&self) -> bool {
        self.gate != MintGate::Open
    }

    pub fn gate_open_at(&self, now: i64) -> bool {
        let base = match self.gate {
            MintGate::Open => true,
            MintGate::WindowUntil { close_ts } => now < close_ts,
            MintGate::Closed => false,
        };
        base || self.schedule.map(|s| s.is_open_at(now)).unwrap_or(false)
    }

    pub fn touch(&mut self, now: i64) {
        self.last_activity_at = now;
    }
}

pub fn profile_for(basket_type: u8, fixed_kind: u8) -> Result<u8> {
    Ok(match basket_type {
        BASKET_TYPE_FIXED => match fixed_kind {
            FIXED_KIND_INDEX => PROFILE_FIXED_INDEX,
            FIXED_KIND_MEME => PROFILE_FIXED_MEME,
            _ => return err!(BasketError::InvalidBasketType),
        },
        BASKET_TYPE_MIRROR => PROFILE_MIRROR,
        BASKET_TYPE_MANAGED => PROFILE_MANAGED,
        BASKET_TYPE_STRATEGY => PROFILE_STRATEGY,
        _ => return err!(BasketError::InvalidBasketType),
    })
}
