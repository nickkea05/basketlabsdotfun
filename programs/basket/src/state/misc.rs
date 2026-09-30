//! Smaller per-basket accounts: `FeeVault`, `PendingBook`, `FrozenClaim`,
//! `CreatorLock`, `Redemption`.

use anchor_lang::prelude::*;

use crate::state::position::PositionArg;

/// `["fees", share_mint]`. Owns two ATAs: the share mint (mint/redeem fee
/// skims, D11) and wSOL (pool swap fees, D3). Fees arrive unsplit; `sweep_fees`
/// pays the creator and protocol lines out. `holder_reserve_*` is the legacy
/// holder line (rewards removed by the change order); it is re-routed to the
/// buyback when the fee split is rebuilt (progress §6.3 step 5).
#[account]
#[derive(InitSpace, Debug, Default)]
pub struct FeeVault {
    pub bump: u8,
    pub basket: Pubkey,
    pub holder_reserve_shares: u64,
    pub holder_reserve_lamports: u64,
    /// Creator SOL line held back because paying it would have left the
    /// creator wallet below rent exemption (a sweep must never revert on
    /// the creator's account state). Paid on a later sweep.
    pub creator_owed_lamports: u64,
    /// Lifetime totals, for the indexer.
    pub swept_creator_shares: u64,
    pub swept_creator_lamports: u64,
    pub swept_protocol_shares: u64,
    pub swept_protocol_lamports: u64,
    pub distributed_shares: u64,
    pub distributed_lamports: u64,
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
