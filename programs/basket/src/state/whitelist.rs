//! `Whitelist` — allowed component mints and their liquidity tier (§2).
//! Kept sorted by mint so lookups are a binary search; grown with realloc.

use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, InitSpace, Debug, PartialEq, Eq)]
pub struct WhitelistEntry {
    pub mint: Pubkey,
    /// Liquidity tier, 0 = deepest. Informational for now.
    pub tier: u8,
}

#[account]
#[derive(Debug)]
pub struct Whitelist {
    pub bump: u8,
    pub entries: Vec<WhitelistEntry>,
}

impl Whitelist {
    pub const HEADER: usize = 8 + 1 + 4;
    pub const ENTRY: usize = 32 + 1;
    /// Account data is capped at 10 MiB; leave headroom.
    pub const MAX_ENTRIES: usize = 200_000;

    pub fn space_for(n: usize) -> usize {
        Self::HEADER + n * Self::ENTRY
    }

    pub fn find(&self, mint: &Pubkey) -> Option<&WhitelistEntry> {
        self.entries
            .binary_search_by(|e| e.mint.to_bytes().cmp(&mint.to_bytes()))
            .ok()
            .map(|i| &self.entries[i])
    }

    pub fn contains(&self, mint: &Pubkey) -> bool {
        self.find(mint).is_some()
    }

    /// Insert or update, keeping order. Returns true if a new entry was added.
    pub fn upsert(&mut self, mint: Pubkey, tier: u8) -> bool {
        match self.entries.binary_search_by(|e| e.mint.to_bytes().cmp(&mint.to_bytes())) {
            Ok(i) => {
                self.entries[i].tier = tier;
                false
            }
            Err(i) => {
                self.entries.insert(i, WhitelistEntry { mint, tier });
                true
            }
        }
    }

    pub fn remove(&mut self, mint: &Pubkey) -> bool {
        match self.entries.binary_search_by(|e| e.mint.to_bytes().cmp(&mint.to_bytes())) {
            Ok(i) => {
                self.entries.remove(i);
                true
            }
            Err(_) => false,
        }
    }
}
