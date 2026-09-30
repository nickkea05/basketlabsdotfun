//! `Basket` — one per launch — and the deployer's launch payload.

use anchor_lang::prelude::*;

use crate::constants::*;
use crate::error::BasketError;
use crate::state::config::{ManagedRules, PoolPreset};

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

/// What the deployer submits at "Launch" (frontend `buildLaunchParams()`).
/// The deployer signs `create_basket` directly (change order §2.2), so
/// there is no signed payload any more; `nonce` only salts the share mint
/// PDA so one wallet can launch more than once.
///
/// The asset list itself is not in the payload; `book_hash` commits to it
/// (chain hash, see `Basket::chain_hash`) and `add_positions` delivers it.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateBasketArgs {
    pub nonce: u64,
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
    /// DLMM active bin at launch: one base share unit costs
    /// `(1 + bin_step/10⁴)^launch_active_id` lamports. The client derives it
    /// from the intended NAV per share; `seed` checks the first buy against it.
    pub launch_active_id: i32,
}

/// Q10: keeper-signed leaderboard tier, verified by the ed25519 precompile
/// over `borsh(TierMessage)`. Missing or expired → tier 0.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierAttestation {
    pub keeper: Pubkey,
    pub tier: u8,
    pub expiry_slot: u64,
}

/// The bytes the keeper signs for a tier attestation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierMessage {
    pub program_id: Pubkey,
    pub cluster: u8,
    pub creator: Pubkey,
    pub tier: u8,
    pub expiry_slot: u64,
}

/// `seed` (§5): the first buy. `deposits[i]` is the amount of position `i`
/// (book order) delivered in kind; `initial_shares` is the gross X the buyer
/// is paying for (the program never prices, D4); `sleeve_lamports` is the
/// SOL leg, `r·D` by the client's split. It must agree with the launch bin:
/// `sleeve_lamports ≈ Y × price(launch_active_id)` within one bin step.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct SeedArgs {
    pub deposits: Vec<u64>,
    pub initial_shares: u64,
    pub sleeve_lamports: u64,
}

/// `mint` (§5). Shares follow the creation-unit rule over `deposits`; the
/// active bin then prices the SOL leg. `min_shares_out` and
/// `max_sleeve_lamports` are the buyer's slippage bounds.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct MintArgs {
    pub deposits: Vec<u64>,
    pub min_shares_out: u64,
    pub max_sleeve_lamports: u64,
}

/// Per-basket fee schedule, copied from `Config` at creation (D11, §4,
/// change order §4). The four split lines sum to `BPS_TOTAL`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct FeeSchedule {
    pub mint_fee_bps: u16,
    pub redeem_fee_bps: u16,
    pub creator_split_bps: u16,
    pub buyback_split_bps: u16,
    pub team_split_bps: u16,
    pub prize_split_bps: u16,
    /// Managed only, else 0.
    pub mgmt_fee_bps_per_year: u16,
    /// Managed only, else 0.
    pub perf_fee_bps: u16,
}

/// One fee amount split four ways; the creator line is exact, the rest is
/// shared pro rata by the config lines so the pieces always add up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeeSplit {
    pub creator: u64,
    pub buyback: u64,
    pub team: u64,
    pub prize: u64,
}

impl FeeSchedule {
    pub fn split(&self, amount: u64) -> Result<FeeSplit> {
        let creator = crate::math::bps(amount, self.creator_split_bps)?;
        let rest = amount - creator;
        let others = self.buyback_split_bps as u64 + self.team_split_bps as u64 + self.prize_split_bps as u64;
        if others == 0 {
            return Ok(FeeSplit { creator: amount, buyback: 0, team: 0, prize: 0 });
        }
        let buyback = crate::math::mul_div_u64(rest, self.buyback_split_bps as u64, others, crate::math::Rounding::Down)?;
        let team = crate::math::mul_div_u64(rest, self.team_split_bps as u64, others, crate::math::Rounding::Down)?;
        let prize = rest - buyback - team;
        Ok(FeeSplit { creator, buyback, team, prize })
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct SleeveParams {
    pub step_r_bps: u16,
    pub step_threshold_lamports: u64,
    /// After the step phase.
    pub r_bps: u16,
}

/// The basket's DLMM pool (created in `create_basket`) and its preset.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct PoolParams {
    pub lb_pair: Pubkey,
    pub preset: PoolPreset,
    pub launch_active_id: i32,
}

/// One DLMM position the basket PDA owns. `key` is the PDA
/// `["position", lb_pair, base, lower, width]` with base = basket (tight)
/// or share_auth (backstop). Zero until placed.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct PositionRef {
    pub key: Pubkey,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
}

impl PositionRef {
    pub fn is_set(&self) -> bool {
        self.key != Pubkey::default()
    }

    pub fn width(&self) -> i32 {
        self.upper_bin_id - self.lower_bin_id + 1
    }

    pub fn contains(&self, bin_id: i32) -> bool {
        self.is_set() && bin_id >= self.lower_bin_id && bin_id <= self.upper_bin_id
    }
}

/// Backstop lifecycle (progress §6.4a; funding and withdrawal are chunked
/// per bin array because a whole-range add or remove does not fit one tx).
pub const BACKSTOP_UNPLACED: u8 = 0;
/// Position and bin arrays exist; `fund_backstop` still has arrays to fill
/// (`backstop_mask` = arrays funded so far).
pub const BACKSTOP_FUNDING: u8 = 1;
/// Fully funded; mints add their slice; the keeper never moves it.
pub const BACKSTOP_LIVE: u8 = 2;
/// `withdraw_backstop` in progress (`backstop_mask` = arrays emptied).
pub const BACKSTOP_WITHDRAWING: u8 = 3;
/// Emptied and closed; awaiting `reset_backstop` (re-place) or `close_basket`.
pub const BACKSTOP_CLOSED: u8 = 4;

/// State of an in-flight Mirror/Strategy/Managed rebalance.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, Default, PartialEq, Eq)]
pub struct Rebalance {
    pub active: bool,
    /// Incremented every time a rebalance starts; scopes `Position.sold`.
    pub seq: u16,
    /// Chain hash of the target book (the `PendingBook` holds the list).
    pub target_hash: [u8; 32],
    pub target_count: u16,
    /// `execute_swap` is allowed until here; `finalize_rebalance` any time.
    pub window_end_ts: i64,
    /// `finalize_rebalance` progress: entries processed so far, running
    /// chain hash and weight sum over them.
    pub acc_count: u16,
    pub acc_hash: [u8; 32],
    pub acc_weight: u16,
    /// Managed turnover accounting: bps of the book turned over in the window.
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
    /// Attested leaderboard tier at create (Q10).
    pub creator_tier: u8,
    /// The deployer: signs `create_basket`, makes the first buy, gets the
    /// deposit and rents back at close. Also `payer` (kept separate for the
    /// indexer and for events).
    pub creator: Pubkey,
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
    /// Commitment to the book (`CreateBasketArgs.book_hash`).
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

    pub pool: PoolParams,
    /// Re-centred by the keeper; where fills happen. Set at `seed`.
    pub tight: PositionRef,
    /// Placed once by the keeper after `seed`, whole bin arrays around the
    /// launch bin; withdrawn only by `withdraw_backstop`.
    pub backstop: PositionRef,
    pub backstop_state: u8,
    /// Bit per backstop bin array (from `backstop.lower_bin_id`'s array):
    /// funded (FUNDING) or emptied (WITHDRAWING).
    pub backstop_mask: u8,
    pub last_recenter_ts: i64,
    pub recenter_count: u32,

    /// Change order §2.2: deployer's deposit held on this account, and the
    /// non-refundable part (pool + bin-array rent) spent from it so far.
    pub deposit_lamports: u64,
    pub deposit_spent_lamports: u64,

    /// Q9: pool-fee SOL claimed in the current and previous prize epochs.
    pub fee_epoch: u64,
    pub fee_epoch_lamports: u64,
    pub fee_prev_epoch_lamports: u64,

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
        solana_sha256_hasher::hashv(&[acc, mint.as_ref(), &weight_bps.to_le_bytes()]).to_bytes()
    }

    pub fn is_managed(&self) -> bool {
        self.basket_type == BASKET_TYPE_MANAGED
    }

    /// Mirror, Managed and Strategy books can change; Fixed cannot (D20).
    pub fn book_is_mutable(&self) -> bool {
        self.basket_type != BASKET_TYPE_FIXED
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

    pub fn backstop_live(&self) -> bool {
        self.backstop_state == BACKSTOP_LIVE
    }

    /// Pool-fee SOL this basket claimed during `epoch`, as far as the
    /// two-epoch window remembers.
    pub fn epoch_pool_fees(&self, epoch: u64) -> u64 {
        if epoch == self.fee_epoch {
            self.fee_epoch_lamports
        } else if epoch + 1 == self.fee_epoch {
            self.fee_prev_epoch_lamports
        } else {
            0
        }
    }

    /// Record `lamports` of claimed pool fees in `epoch`, rolling the window.
    pub fn record_pool_fees(&mut self, epoch: u64, lamports: u64) {
        if epoch != self.fee_epoch {
            self.fee_prev_epoch_lamports = if epoch == self.fee_epoch + 1 { self.fee_epoch_lamports } else { 0 };
            self.fee_epoch = epoch;
            self.fee_epoch_lamports = 0;
        }
        self.fee_epoch_lamports = self.fee_epoch_lamports.saturating_add(lamports);
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
