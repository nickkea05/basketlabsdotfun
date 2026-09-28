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
    pub asset_count: u16,
    pub book_hash: [u8; 32],
    pub nonce: u64,
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
    pub pool: Pubkey,
    pub position_nft_mint: Pubkey,
    pub shares_to_buyer: u64,
    pub fee_shares: u64,
    pub treasury_shares: u64,
    pub sleeve_lamports: u64,
    pub creation_fee_lamports: u64,
    pub sqrt_price: u128,
}

#[event]
pub struct Minted {
    pub basket: Pubkey,
    pub buyer: Pubkey,
    pub shares_to_buyer: u64,
    pub fee_shares: u64,
    pub treasury_shares: u64,
    pub sleeve_lamports: u64,
    pub r_bps: u16,
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
    pub lamports: u64,
}

#[event]
pub struct FeesSwept {
    pub basket: Pubkey,
    pub creator_shares: u64,
    pub creator_lamports: u64,
    pub protocol_shares: u64,
    pub protocol_lamports: u64,
    pub holder_shares: u64,
    pub holder_lamports: u64,
}

#[event]
pub struct RewardsRootPosted {
    pub basket: Pubkey,
    pub epoch: u64,
    pub root: [u8; 32],
    pub reward_mint: Pubkey,
    pub total_amount: u64,
    pub leaf_count: u32,
}

#[event]
pub struct RewardDistributed {
    pub basket: Pubkey,
    pub epoch: u64,
    pub wallet: Pubkey,
    pub amount: u64,
    pub index: u32,
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
}
