//! Harness for `claim_pool_fees` / `sweep_fees`: a cp-amm `swap` so the
//! position accrues swap fees, instruction builders and FeeVault reads.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use solana_keypair::Keypair;
use solana_signer::Signer;

use basket::damm::{self, cp_amm};
use basket::state::*;

use super::*;

impl Env {
    /// Wrap `lamports` of `owner`'s SOL into their wSOL ATA.
    pub fn wrap_sol(&mut self, owner: &Keypair, lamports: u64) -> Pubkey {
        let ata = Env::ata(&owner.pubkey(), &WSOL);
        let ixs = [
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &owner.pubkey(),
                &owner.pubkey(),
                &WSOL,
                &spl_token::ID,
            ),
            anchor_lang::solana_program::system_instruction::transfer(&owner.pubkey(), &ata, lamports),
            spl_token::instruction::sync_native(&spl_token::ID, &ata).unwrap(),
        ];
        self.send_ok(&ixs, owner, &[]);
        ata
    }

    /// Trade through the basket's pool directly (not through our program).
    /// `a_to_b = false`: SOL in, shares out.
    pub fn swap(&mut self, l: &Launched, s: &SeededBasket, trader: &Keypair, amount_in: u64, a_to_b: bool) {
        let pool = self.pool_state(&s.pool);
        let trader_a = Env::ata(&trader.pubkey(), &l.share_mint);
        let trader_b = Env::ata(&trader.pubkey(), &WSOL);
        let (input, output) = if a_to_b { (trader_a, trader_b) } else { (trader_b, trader_a) };
        let mut accounts = cp_amm::client::accounts::Swap {
            pool_authority: damm::pool_authority(),
            pool: s.pool,
            input_token_account: input,
            output_token_account: output,
            token_a_vault: pool.token_a_vault,
            token_b_vault: pool.token_b_vault,
            token_a_mint: l.share_mint,
            token_b_mint: WSOL,
            payer: trader.pubkey(),
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            referral_token_account: None,
            event_authority: damm::event_authority(),
            program: CP_AMM_ID,
        }
        .to_account_metas(None);
        // Anchor renders `None` optional accounts as the program id.
        for m in accounts.iter_mut() {
            if m.pubkey == CP_AMM_ID && m.is_writable {
                m.is_writable = false;
            }
        }
        let ix = Instruction {
            program_id: CP_AMM_ID,
            accounts,
            data: cp_amm::client::args::Swap {
                _params: cp_amm::types::SwapParameters { amount_in, minimum_amount_out: 0 },
            }
            .data(),
        };
        let create = spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &trader.pubkey(),
            &trader.pubkey(),
            if a_to_b { &WSOL } else { &l.share_mint },
            &spl_token::ID,
        );
        self.send_ok(&[create, ix], trader, &[]);
    }

    /// Buy shares with `lamports` of SOL through the pool.
    pub fn buy_shares(&mut self, l: &Launched, s: &SeededBasket, trader: &Keypair, lamports: u64) {
        self.wrap_sol(trader, lamports);
        self.swap(l, s, trader, lamports, false);
    }

    /// Sell `shares` into the pool for SOL.
    pub fn sell_shares(&mut self, l: &Launched, s: &SeededBasket, trader: &Keypair, shares: u64) {
        self.swap(l, s, trader, shares, true);
    }

    pub fn claim_pool_fees_ix(&self, l: &Launched, s: &SeededBasket, caller: &Pubkey) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let accounts = basket::accounts::ClaimPoolFees {
            caller: *caller,
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            fee_vault_wsol_ata: Env::ata(&fee_vault, &WSOL),
            wsol_mint: WSOL,
            position_nft_account: damm::position_nft_account(&s.position_nft_mint),
            pool_authority: damm::pool_authority(),
            pool: s.pool,
            pool_position: damm::position(&s.position_nft_mint),
            token_a_vault: damm::token_vault(&l.share_mint, &s.pool),
            token_b_vault: damm::token_vault(&WSOL, &s.pool),
            cp_amm_program: CP_AMM_ID,
            event_authority: damm::event_authority(),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::ClaimPoolFees {}.data() }
    }

    /// Returns lamports claimed into the FeeVault.
    pub fn claim_pool_fees(&mut self, l: &Launched, s: &SeededBasket, caller: &Keypair) -> u64 {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let before = self.lamports(&fee_vault);
        let ix = self.claim_pool_fees_ix(l, s, &caller.pubkey());
        let m = self.send_ok(&[ix], caller, &[]);
        println!("claim_pool_fees CU: {}", m.compute_units_consumed);
        self.lamports(&fee_vault) - before
    }

    pub fn sweep_fees_ix(&self, l: &Launched, caller: &Pubkey) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let creator = l.creator.pubkey();
        let treasury = self.treasury.pubkey();
        let accounts = basket::accounts::SweepFees {
            caller: *caller,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            creator,
            creator_share_ata: Env::ata(&creator, &l.share_mint),
            treasury,
            treasury_share_ata: Env::ata(&treasury, &l.share_mint),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::SweepFees {}.data() }
    }

    pub fn sweep_fees(&mut self, l: &Launched, caller: &Keypair) -> u64 {
        let ix = self.sweep_fees_ix(l, &caller.pubkey());
        let m = self.send_ok(&[ix], caller, &[]);
        m.compute_units_consumed
    }

    pub fn fee_vault(&self, l: &Launched) -> FeeVault {
        self.load(&fee_vault_pda(&l.share_mint))
    }

    /// Lamports the FeeVault holds above its own rent.
    pub fn fee_vault_free_lamports(&self, l: &Launched) -> u64 {
        let key = fee_vault_pda(&l.share_mint);
        let acc = self.account(&key).unwrap();
        let rent = self.svm.minimum_balance_for_rent_exemption(acc.data.len());
        acc.lamports - rent
    }
}
