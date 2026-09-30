//! Smaller accounts: `FeeVault`, `BuybackVault`, `PrizeVault`, `PendingBook`,
//! `FrozenClaim`, `CreatorLock`, `Redemption`.

use anchor_lang::prelude::*;

use crate::state::basket::FeeSchedule;
use crate::state::position::PositionArg;

/// `["fees", share_mint]`. Holds the mint / redeem fee shares (in its share
/// ATA) until `sweep_fees`, the creator's SOL line when it could not be paid
/// out, and, between `sweep_fees` and `settle_fees`, the in-kind
/// components of the buyback / team / prize lines in its component ATAs.
/// Pool swap fees arrive as SOL and are routed straight through.
#[account]
#[derive(InitSpace, Debug, Default)]
pub struct FeeVault {
    pub bump: u8,
    pub basket: Pubkey,
    /// Copied from the basket so `settle_fees` / `close_fee_vault` keep
    /// working after `close_basket`.
    pub creator: Pubkey,
    pub payer: Pubkey,
    pub fees: FeeSchedule,
    /// Creator SOL line held back because paying it would have left the
    /// creator wallet below rent exemption (a sweep must never revert on
    /// the creator's account state). Paid on a later sweep.
    pub creator_owed_lamports: u64,
    /// Pool fees the keeper claimed on the way through a re-centre or a
    /// backstop withdrawal (those instructions do not carry the routing
    /// accounts); routed by the next `claim_pool_fees`.
    pub unrouted_lamports: u64,
    /// Lifetime totals, for the indexer.
    pub swept_creator_shares: u64,
    pub swept_creator_lamports: u64,
    pub routed_buyback_lamports: u64,
    pub routed_team_lamports: u64,
    pub routed_prize_lamports: u64,
    /// Fee shares redeemed in kind and still waiting for `settle_fees`.
    pub unsettled_shares: u64,
}

/// `["buyback"]` (one per deployment). Accrues the buyback SOL line; it can
/// only ever be swapped into `Config.bskt_mint` and burned.
#[account]
#[derive(InitSpace, Debug, Default)]
pub struct BuybackVault {
    pub bump: u8,
    pub received_lamports: u64,
    pub spent_lamports: u64,
    pub burned_bskt: u64,
}

/// `["prize"]` (one per deployment). Accrues the deployer-prize SOL line;
/// paid out by `post_prize_payout`.
#[account]
#[derive(InitSpace, Debug, Default)]
pub struct PrizeVault {
    pub bump: u8,
    pub received_lamports: u64,
    pub paid_lamports: u64,
    /// Highest epoch already paid; each epoch is paid at most once.
    pub last_paid_epoch: u64,
    pub paid_any: bool,
}
/// `["pending", share_mint]`. Managed creator's timelocked book (§5).
#[account]
#[derive(InitSpace, Debug)]
pub struct PendingBook {
    pub bump: u8,
    pub basket: Pubkey,
    /// Who paid the rent (creator or keeper); refunded when the book is
    /// finalized or replaced.
    pub payer: Pubkey,
    pub submitted_at: i64,
    /// Managed: `submitted_at + timelock`. Others: `submitted_at`.
    pub ready_at: i64,
    pub book_hash: [u8; 32],
    #[max_len(PendingBook::MAX_BOOK)]
    pub book: Vec<PositionArg>,
}

impl PendingBook {
    pub const MAX_BOOK: usize = 64;

    pub fn weight_of(&self, mint: &Pubkey) -> Option<u16> {
        self.book.iter().find(|p| p.mint == *mint).map(|p| p.weight_bps)
    }
}

/// `["claim", share_mint, wallet, mint]`. D10.
#[account]
#[derive(InitSpace, Debug)]
pub struct FrozenClaim {
    pub bump: u8,
    pub basket: Pubkey,
    pub wallet: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
    pub created_at: i64,
}

/// `["lock", share_mint]`. Creator's locked self-position (leaderboard skin).
/// Shares sit in ATA(CreatorLock, share_mint).
#[account]
#[derive(InitSpace, Debug)]
pub struct CreatorLock {
    pub bump: u8,
    pub basket: Pubkey,
    pub creator: Pubkey,
    pub amount: u64,
    pub locked_at: i64,
    pub unlock_at: i64,
}

/// `["redemption", share_mint, holder]`. Multi-transaction redeem (D9,
/// "above ~12 assets, two transactions"). Shares are burned and the pool
/// leg paid in `redeem_begin`; components are paid in one or more
/// `redeem_components` calls; the account closes when every position has
/// been paid.
#[account]
#[derive(InitSpace, Debug)]
pub struct Redemption {
    pub bump: u8,
    pub basket: Pubkey,
    pub holder: Pubkey,
    /// Net shares (after the redeem fee) this redemption is worth.
    pub shares: u64,
    pub position_count: u16,
    pub paid_count: u16,
    #[max_len(64)]
    pub paid_mints: Vec<Pubkey>,
    pub created_at: i64,
}
