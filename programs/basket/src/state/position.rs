//! `Position` — one per component mint in a basket (D20).

use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace, Debug)]
pub struct Position {
    pub bump: u8,
    pub basket: Pubkey,
    pub mint: Pubkey,
    pub weight_bps: u16,
    /// Token or Token-2022 program that owns `mint`.
    pub token_program: Pubkey,
    pub decimals: u8,
    /// ATA(basket, mint) under `token_program`.
    pub vault: Pubkey,
    /// Component amount owed to holders through `FrozenClaim`s (D10). The
    /// vault balance minus this is what pro-rata math uses.
    pub owed: u64,
    /// Order in which the position entered the book; the chain hash follows
    /// this order.
    pub index: u16,
}

impl Position {
    pub fn effective_vault_amount(&self, vault_amount: u64) -> u64 {
        vault_amount.saturating_sub(self.owed)
    }
}

/// One component in `add_positions` / `finalize_rebalance`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, InitSpace)]
pub struct PositionArg {
    pub mint: Pubkey,
    pub weight_bps: u16,
}
