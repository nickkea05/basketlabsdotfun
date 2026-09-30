//! Harness for the Phase 7 instructions: rewards roots (D15), the creator
//! lock, and the close crank (`close_positions` + `close_basket`).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use solana_signer::Signer;

use basket::constants::*;
use basket::damm;
use basket::state::*;

use super::*;

pub fn rewards_root_pda(share_mint: &Pubkey, epoch: u64) -> Pubkey {
    Pubkey::find_program_address(&[seeds::REWARDS, share_mint.as_ref(), &epoch.to_le_bytes()], &basket::id()).0
}

pub fn creator_lock_pda(share_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::LOCK, share_mint.as_ref()], &basket::id()).0
}

/// Sorted-pair Merkle tree over `RewardsRoot::leaf` hashes; an odd node is
/// carried up unchanged. Mirrors `RewardsRoot::verify`.
pub struct MerkleTree {
    layers: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    pub fn new(leaves: Vec<[u8; 32]>) -> Self {
        assert!(!leaves.is_empty());
        let mut layers = vec![leaves];
        while layers.last().unwrap().len() > 1 {
            let prev = layers.last().unwrap();
            let next: Vec<[u8; 32]> = prev
                .chunks(2)
                .map(|c| {
                    if c.len() == 2 {
                        let (a, b) = if c[0] <= c[1] { (c[0], c[1]) } else { (c[1], c[0]) };
                        solana_sha256_hasher::hashv(&[&[1u8], &a[..], &b[..]]).to_bytes()
                    } else {
                        c[0]
                    }
                })
                .collect();
            layers.push(next);
        }
        MerkleTree { layers }
    }

    /// Build from `(wallet, amount)` rows; leaf index = row index.
    pub fn from_rows(rows: &[(Pubkey, u64)]) -> Self {
        Self::new(rows.iter().enumerate().map(|(i, (w, a))| RewardsRoot::leaf(i as u32, w, *a)).collect())
    }

    pub fn root(&self) -> [u8; 32] {
        self.layers.last().unwrap()[0]
    }

    pub fn proof(&self, mut index: usize) -> Vec<[u8; 32]> {
        let mut proof = vec![];
        for layer in &self.layers[..self.layers.len() - 1] {
            let sibling = index ^ 1;
            if sibling < layer.len() {
                proof.push(layer[sibling]);
            }
            index /= 2;
        }
        proof
    }
}

impl Env {
    // ---- rewards ----

    #[allow(clippy::too_many_arguments)]
    pub fn post_rewards_root_ix(
        &self,
        l: &Launched,
        keeper: &Pubkey,
        epoch: u64,
        root: [u8; 32],
        reward_mint: Pubkey,
        total_amount: u64,
        leaf_count: u32,
    ) -> Instruction {
        let accounts = basket::accounts::PostRewardsRoot {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault: fee_vault_pda(&l.share_mint),
            rewards_root: rewards_root_pda(&l.share_mint, epoch),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::PostRewardsRoot { epoch, root, reward_mint, total_amount, leaf_count }.data(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn distribute_rewards_ix(
        &self,
        l: &Launched,
        caller: &Pubkey,
        epoch: u64,
        wallet: &Pubkey,
        index: u32,
        amount: u64,
        proof: Vec<[u8; 32]>,
    ) -> Instruction {
        let root_key = rewards_root_pda(&l.share_mint, epoch);
        let root: RewardsRoot = self.load(&root_key);
        let fee_vault = fee_vault_pda(&l.share_mint);
        let accounts = basket::accounts::DistributeRewards {
            caller: *caller,
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            rewards_root: root_key,
            root_payer: root.payer,
            wallet: *wallet,
            wallet_share_ata: Env::ata(wallet, &l.share_mint),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::DistributeRewards { index, amount, proof }.data(),
        }
    }

    pub fn close_rewards_root_ix(&self, l: &Launched, keeper: &Pubkey, epoch: u64) -> Instruction {
        let root_key = rewards_root_pda(&l.share_mint, epoch);
        let root: RewardsRoot = self.load(&root_key);
        let accounts = basket::accounts::CloseRewardsRoot {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault: fee_vault_pda(&l.share_mint),
            rewards_root: root_key,
            root_payer: root.payer,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::CloseRewardsRoot {}.data() }
    }

    // ---- creator lock ----

    pub fn lock_creator_shares_ix(&self, l: &Launched, signer: &Pubkey, amount: u64, duration_s: i64) -> Instruction {
        let lock = creator_lock_pda(&l.share_mint);
        let accounts = basket::accounts::LockCreatorShares {
            creator: *signer,
            basket: l.basket,
            share_mint: l.share_mint,
            creator_share_ata: Env::ata(signer, &l.share_mint),
            lock,
            lock_share_ata: Env::ata(&lock, &l.share_mint),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::LockCreatorShares { amount, duration_s }.data(),
        }
    }

    pub fn unlock_creator_shares_ix(&self, l: &Launched, signer: &Pubkey) -> Instruction {
        let lock = creator_lock_pda(&l.share_mint);
        let accounts = basket::accounts::UnlockCreatorShares {
            creator: *signer,
            basket: l.basket,
            share_mint: l.share_mint,
            creator_share_ata: Env::ata(signer, &l.share_mint),
            lock,
            lock_share_ata: Env::ata(&lock, &l.share_mint),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::UnlockCreatorShares {}.data() }
    }

    // ---- close crank ----

    /// Close the positions for `mints` (dust to the treasury, rent to the
    /// basket's payer). Chunk as needed.
    pub fn close_positions_ix(&self, l: &Launched, s: Option<&SeededBasket>, keeper: &Pubkey, mints: &[Pubkey]) -> Instruction {
        let treasury = self.treasury.pubkey();
        let fee_vault = fee_vault_pda(&l.share_mint);
        let mut accounts = basket::accounts::ClosePositions {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            pool: s.map(|s| s.pool),
            pool_position: s.map(|s| damm::position(&s.position_nft_mint)),
            pending_book: pending_book_pda(&l.share_mint),
            treasury,
            payer: l.payer.pubkey(),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        for m in mints {
            accounts.push(AccountMeta::new(position_pda(&l.share_mint, m), false));
            accounts.push(AccountMeta::new(Env::ata(&l.basket, m), false));
            accounts.push(AccountMeta::new_readonly(*m, false));
            accounts.push(AccountMeta::new(Env::ata(&treasury, m), false));
        }
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::ClosePositions {}.data() }
    }

    /// Final step: drain the sleeve to the treasury, burn what is left,
    /// close FeeVault + Basket. `s = None` for a basket that was never seeded.
    pub fn close_basket_ix(&self, l: &Launched, s: Option<&SeededBasket>, keeper: &Pubkey) -> Instruction {
        let treasury = self.treasury.pubkey();
        let fee_vault = fee_vault_pda(&l.share_mint);
        let accounts = basket::accounts::CloseBasket {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault: s.map(|_| fee_vault),
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            creator: l.creator.pubkey(),
            treasury,
            treasury_wsol_ata: Env::ata(&treasury, &WSOL),
            payer: l.payer.pubkey(),
            pending_book: pending_book_pda(&l.share_mint),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            position_nft_mint: s.map(|s| s.position_nft_mint),
            position_nft_account: s.map(|s| damm::position_nft_account(&s.position_nft_mint)),
            pool_authority: damm::pool_authority(),
            pool: s.map(|s| s.pool),
            pool_position: s.map(|s| damm::position(&s.position_nft_mint)),
            token_a_vault: s.map(|s| damm::token_vault(&l.share_mint, &s.pool)),
            token_b_vault: s.map(|s| damm::token_vault(&WSOL, &s.pool)),
            cp_amm_program: CP_AMM_ID,
            event_authority: damm::event_authority(),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::CloseBasket {}.data() }
    }
}
