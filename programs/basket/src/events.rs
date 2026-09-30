//! Events on every state change (§5). The indexer builds follower PnL,
//! leaderboard and portfolio from these plus SPL transfers. Every event
//! carries the basket and the acting wallet; the slot comes from the log.

use anchor_lang::prelude::*;

#[event]
pub struct ConfigInitialized {
    pub admin: Pubkey,
    pub treasury: Pubkey,
    pub cluster: u8,
}

#[event]
pub struct ConfigUpdated {
    pub admin: Pubkey,
}

#[event]
pub struct KeepersRotated {
    pub keepers: Vec<Pubkey>,
}

#[event]
pub struct WhitelistUpdated {
    pub added: u32,
    pub removed: u32,
    pub total: u32,
}

#[event]
pub struct PauseChanged {
    pub paused: bool,
}

#[event]
pub struct BasketCreated {
    pub basket: Pubkey,
    pub share_mint: Pubkey,
    pub creator: Pubkey,
    pub payer: Pubkey,
    pub basket_type: u8,
    pub profile: u8,
    pub creator_tier: u8,
    pub asset_count: u16,
    pub book_hash: [u8; 32],
    pub nonce: u64,
    pub lb_pair: Pubkey,
    pub bin_step: u16,
    pub launch_active_id: i32,
    pub deposit_lamports: u64,
    pub pool_rent_lamports: u64,
}
#[event]
pub struct PositionsAdded {
    pub basket: Pubkey,
    pub from_index: u16,
    pub count: u16,
    pub complete: bool,
}

#[event]
pub struct Seeded {
    pub basket: Pubkey,
    pub buyer: Pubkey,
    pub tight_position: Pubkey,
    pub tight_lower_bin_id: i32,
    pub tight_upper_bin_id: i32,
    pub shares_to_buyer: u64,
    pub fee_shares: u64,
    pub treasury_shares: u64,
    pub sleeve_lamports: u64,
    pub creation_fee_lamports: u64,
    pub bin_array_rent_lamports: u64,
}

#[event]
pub struct Minted {
    pub basket: Pubkey,
    pub buyer: Pubkey,
    pub shares_to_buyer: u64,
    pub fee_shares: u64,
    pub treasury_shares: u64,
    pub sleeve_lamports: u64,
    pub backstop_lamports: u64,
    pub r_bps: u16,
    pub active_id: i32,
}
#[event]
pub struct Redeemed {
    pub basket: Pubkey,
    pub holder: Pubkey,
    pub shares_burned: u64,
    pub fee_shares: u64,
    pub treasury_shares_burned: u64,
    pub lamports_out: u64,
    pub components_paid: u16,
    pub claims_created: u16,
    /// False when components are still to be paid through `Redemption`.
    pub complete: bool,
}

#[event]
pub struct RedemptionComponentsPaid {
    pub basket: Pubkey,
    pub holder: Pubkey,
    pub count: u16,
    pub complete: bool,
}

#[event]
pub struct FrozenClaimCreated {
    pub basket: Pubkey,
    pub wallet: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
}

#[event]
pub struct FrozenClaimPaid {
    pub basket: Pubkey,
    pub wallet: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
}

#[event]
pub struct BookSubmitted {
    pub basket: Pubkey,
    pub by: Pubkey,
    pub book_hash: [u8; 32],
    pub count: u16,
    /// Managed: when the pending book can be applied.
    pub ready_at: i64,
}

#[event]
pub struct BookApplied {
    pub basket: Pubkey,
    pub book_hash: [u8; 32],
    pub window_end_ts: i64,
    pub mgmt_fee_shares: u64,
}

#[event]
pub struct SwapExecuted {
    pub basket: Pubkey,
    pub keeper: Pubkey,
    pub in_mint: Pubkey,
    pub out_mint: Pubkey,
    pub amount_in: u64,
    pub amount_out: u64,
}

#[event]
pub struct RebalanceFinalized {
    pub basket: Pubkey,
    pub book_hash: [u8; 32],
    pub position_count: u16,
}

#[event]
pub struct Crystallized {
    pub basket: Pubkey,
    pub nav_lamports: u64,
    pub hwm_before: u64,
    pub perf_fee_shares: u64,
    pub mgmt_fee_shares: u64,
}

#[event]
pub struct PoolFeesClaimed {
    pub basket: Pubkey,
    pub position: Pubkey,
    pub lamports: u64,
    pub creator_lamports: u64,
    pub buyback_lamports: u64,
    pub team_lamports: u64,
    pub prize_lamports: u64,
    pub epoch: u64,
}

#[event]
pub struct FeesSwept {
    pub basket: Pubkey,
    pub creator_shares: u64,
    pub creator_lamports: u64,
    /// The 80 % redeemed in kind into the FeeVault's component ATAs.
    pub settled_shares: u64,
    pub pool_lamports: u64,
    pub buyback_lamports: u64,
    pub team_lamports: u64,
    pub prize_lamports: u64,
    pub components_paid: u16,
    pub complete: bool,
}

#[event]
pub struct FeesSettled {
    pub basket: Pubkey,
    pub mint: Pubkey,
    pub amount_in: u64,
    pub lamports_out: u64,
    pub buyback_lamports: u64,
    pub team_lamports: u64,
    pub prize_lamports: u64,
}

#[event]
pub struct BackstopPlaced {
    pub basket: Pubkey,
    pub position: Pubkey,
    pub lower_bin_id: i32,
    pub upper_bin_id: i32,
    pub bin_array_rent_lamports: u64,
    pub position_rent_lamports: u64,
}

#[event]
pub struct BackstopFunded {
    pub basket: Pubkey,
    pub array_index: i64,
    pub amount_x: u64,
    pub amount_y: u64,
    pub complete: bool,
}

#[event]
pub struct BackstopWithdrawn {
    pub basket: Pubkey,
    pub array_index: i64,
    pub amount_x: u64,
    pub amount_y: u64,
    pub fee_lamports: u64,
    pub complete: bool,
}

#[event]
pub struct BackstopClosed {
    pub basket: Pubkey,
    pub position: Pubkey,
}

#[event]
pub struct TightRecentered {
    pub basket: Pubkey,
    pub keeper: Pubkey,
    pub active_id: i32,
    pub old_position: Pubkey,
    pub old_lower_bin_id: i32,
    pub old_upper_bin_id: i32,
    pub new_position: Pubkey,
    pub new_lower_bin_id: i32,
    pub new_upper_bin_id: i32,
    pub amount_x: u64,
    pub amount_y: u64,
    pub topped_up_shares: u64,
    pub fee_lamports: u64,
    pub bin_array_rent_lamports: u64,
}

#[event]
pub struct DepositSpent {
    pub basket: Pubkey,
    pub lamports: u64,
    pub reason: u8,
    pub spent_total: u64,
}

#[event]
pub struct BuybackExecuted {
    pub lamports_in: u64,
    pub bskt_burned: u64,
}

#[event]
pub struct BsktMintSet {
    pub mint: Pubkey,
}

#[event]
pub struct PrizesPaid {
    pub epoch: u64,
    pub total_lamports: u64,
    pub recipients: Vec<Pubkey>,
    pub amounts: Vec<u64>,
}
#[event]
pub struct CreatorSharesLocked {
    pub basket: Pubkey,
    pub creator: Pubkey,
    pub amount: u64,
    pub unlock_at: i64,
}

#[event]
pub struct CreatorSharesUnlocked {
    pub basket: Pubkey,
    pub creator: Pubkey,
    pub amount: u64,
}

#[event]
pub struct BasketClosed {
    pub basket: Pubkey,
    pub share_mint: Pubkey,
    pub refunded_to: Pubkey,
    pub lamports: u64,
    pub deposit_refunded: u64,
    pub deposit_spent: u64,
}
