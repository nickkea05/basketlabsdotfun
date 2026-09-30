//! Harness for the Phase 7 instructions: the creator lock and the close crank
//! (`close_positions` + `close_basket` + `close_fee_vault`).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use solana_signer::Signer;

use basket::constants::*;
use basket::dlmm;
use basket::state::*;

use super::*;

pub fn creator_lock_pda(share_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::LOCK, share_mint.as_ref()], &basket::id()).0
}

impl Env {
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
    pub fn close_positions_ix(&self, l: &Launched, keeper: &Pubkey, mints: &[Pubkey]) -> Instruction {
        let treasury = self.treasury.pubkey();
        let b: Basket = self.load(&l.basket);
        let tight = if b.tight.is_set() { b.tight.key } else { l.placeholder() };
        let mut accounts = basket::accounts::ClosePositions {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            lb_pair: l.pool.lb_pair,
            tight_position: tight,
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
        accounts.extend(self.pool_tail(l));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::ClosePositions { count: mints.len() as u16 }.data(),
        }
    }

    /// Final step: drain the tight position to the treasury, burn what is
    /// left, refund the deposit, close the Basket.
    pub fn close_basket_ix(&self, l: &Launched, keeper: &Pubkey) -> Instruction {
        let treasury = self.treasury.pubkey();
        let fee_vault = fee_vault_pda(&l.share_mint);
        let b: Basket = self.load(&l.basket);
        let tight = if b.tight.is_set() { b.tight.key } else { l.placeholder() };
        let mut accounts = basket::accounts::CloseBasket {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault: if b.seeded { Some(fee_vault) } else { None },
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            creator: l.creator.pubkey(),
            treasury,
            treasury_wsol_ata: Env::ata(&treasury, &WSOL),
            payer: l.payer.pubkey(),
            pending_book: pending_book_pda(&l.share_mint),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            basket_wsol_ata: Env::ata(&l.basket, &WSOL),
            lb_pair: l.pool.lb_pair,
            reserve_x: l.pool.reserve_x,
            reserve_y: l.pool.reserve_y,
            tight_position: tight,
            dlmm_event_authority: dlmm::event_authority(),
            dlmm_program: LB_CLMM_ID,
            memo_program: MEMO_ID,
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        // Anchor renders `None` optional accounts as the program id, which
        // must not be marked writable.
        for m in accounts.iter_mut() {
            if m.pubkey == basket::id() {
                m.is_writable = false;
            }
        }
        accounts.extend(self.pool_tail(l));
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::CloseBasket {}.data() }
    }

    pub fn close_fee_vault_ix(&self, l: &Launched, keeper: &Pubkey) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let accounts = basket::accounts::CloseFeeVault {
            keeper: *keeper,
            config: config_pda(),
            share_mint: l.share_mint,
            fee_vault,
            basket: l.basket,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            payer: l.payer.pubkey(),
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::CloseFeeVault {}.data() }
    }
}
