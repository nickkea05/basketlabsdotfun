//! Smaller per-basket accounts: `FeeVault`, `PendingBook`, `FrozenClaim`,
//! `RewardsRoot`, `CreatorLock`, `Redemption`.

use anchor_lang::prelude::*;

use crate::state::position::PositionArg;

/// `["fees", share_mint]`. Owns two ATAs: the share mint (mint/redeem fee
/// skims, D11) and wSOL (pool swap fees, D3). Fees arrive unsplit; `sweep_fees`
/// pays the creator and protocol lines out and leaves the holder line here
/// for `distribute_rewards`. `holder_reserve_*` is what is already
/// earmarked for holders and must not be split again.
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

/// `["rewards", share_mint, epoch_le]`. D15. `claimed` is a bitmap over
/// leaf indices; the leaf is `sha256(0x00 || index_le || wallet || amount_le)`.
#[account]
#[derive(Debug)]
pub struct RewardsRoot {
    pub bump: u8,
    pub basket: Pubkey,
    /// Keeper who paid the rent; refunded when the root closes.
    pub payer: Pubkey,
    pub epoch: u64,
    pub root: [u8; 32],
    /// The share mint (rewards in shares) or the native mint (rewards in
    /// SOL). Both are paid from the FeeVault's holder reserve.
    pub reward_mint: Pubkey,
    pub total_amount: u64,
    pub distributed_amount: u64,
    pub leaf_count: u32,
    pub posted_at: i64,
    pub claimed: Vec<u8>,
}

impl RewardsRoot {
    pub fn space_for(leaf_count: u32) -> usize {
        8 + 1 + 32 + 32 + 8 + 32 + 32 + 8 + 8 + 4 + 8 + 4 + leaf_count.div_ceil(8) as usize
    }

    pub fn is_native(&self) -> bool {
        self.reward_mint == anchor_spl::token::spl_token::native_mint::ID
    }

    pub fn is_claimed(&self, index: u32) -> bool {
        let (byte, bit) = ((index / 8) as usize, index % 8);
        self.claimed.get(byte).map(|b| b & (1 << bit) != 0).unwrap_or(true)
    }

    pub fn set_claimed(&mut self, index: u32) {
        let (byte, bit) = ((index / 8) as usize, index % 8);
        if let Some(b) = self.claimed.get_mut(byte) {
            *b |= 1 << bit;
        }
    }

    pub fn leaf(index: u32, wallet: &Pubkey, amount: u64) -> [u8; 32] {
        let index = index.to_le_bytes();
        let amount = amount.to_le_bytes();
        solana_sha256_hasher::hashv(&[&[0u8], &index[..], wallet.as_ref(), &amount[..]]).to_bytes()
    }

    /// Sorted-pair Merkle verification, `sha256(0x01 || min || max)`.
    pub fn verify(root: &[u8; 32], leaf: [u8; 32], proof: &[[u8; 32]]) -> bool {
        let mut node = leaf;
        for sibling in proof {
            let (a, b) = if node <= *sibling { (node, *sibling) } else { (*sibling, node) };
            node = solana_sha256_hasher::hashv(&[&[1u8], &a[..], &b[..]]).to_bytes();
        }
        node == *root
    }
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
