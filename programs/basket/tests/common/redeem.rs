//! Harness helpers for `redeem`, `redeem_begin` / `redeem_components` and
//! `claim_frozen`.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use solana_keypair::Keypair;
use solana_signer::Signer;

use basket::dlmm;
use basket::state::*;

use super::*;

pub struct RedeemResult {
    pub cu: u64,
    pub logs: Vec<String>,
}

pub fn frozen_claim_pda(share_mint: &Pubkey, wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[seeds::CLAIM, share_mint.as_ref(), wallet.as_ref(), mint.as_ref()],
        &basket::id(),
    )
    .0
}

pub fn redemption_pda(share_mint: &Pubkey, holder: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::REDEMPTION, share_mint.as_ref(), holder.as_ref()], &basket::id()).0
}

impl Env {
    fn redeem_accounts(&self, l: &Launched, holder: &Pubkey) -> basket::accounts::Redeem {
        let fee_vault = fee_vault_pda(&l.share_mint);
        basket::accounts::Redeem {
            holder: *holder,
            basket: l.basket,
            share_mint: l.share_mint,
            holder_share_ata: Env::ata(holder, &l.share_mint),
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            basket_wsol_ata: Env::ata(&l.basket, &WSOL),
            holder_wsol_ata: Env::ata(holder, &WSOL),
            lb_pair: l.pool.lb_pair,
            reserve_x: l.pool.reserve_x,
            reserve_y: l.pool.reserve_y,
            tight_position: l.tight_position(self),
            backstop_position: self.backstop_or_placeholder(l),
            dlmm_event_authority: dlmm::event_authority(),
            dlmm_program: LB_CLMM_ID,
            memo_program: MEMO_ID,
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
    }

    /// Single-transaction redeem over the full book; `claims` are FrozenClaim
    /// PDAs for components the holder expects to be frozen.
    pub fn redeem_ix(&self, l: &Launched, holder: &Pubkey, shares: u64, claims: &[Pubkey]) -> Instruction {
        let mut accounts = self.redeem_accounts(l, holder).to_account_metas(None);
        accounts.extend(component_metas(&l.basket, &l.share_mint, holder, &l.book));
        accounts.extend(self.pool_tail(l));
        accounts.extend(claims.iter().map(|c| AccountMeta::new(*c, false)));
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::Redeem { shares }.data() }
    }

    pub fn redeem(&mut self, l: &Launched, holder: &Keypair, shares: u64) -> RedeemResult {
        let ix = self.redeem_ix(l, &holder.pubkey(), shares, &[]);
        let meta = self.send_ok(&[ix], holder, &[]);
        RedeemResult { cu: meta.compute_units_consumed, logs: meta.logs }
    }

    pub fn redeem_begin_ix(&self, l: &Launched, holder: &Pubkey, shares: u64) -> Instruction {
        let m = self.redeem_accounts(l, holder);
        let mut accounts = basket::accounts::RedeemBegin {
            holder: m.holder,
            basket: m.basket,
            share_mint: m.share_mint,
            holder_share_ata: m.holder_share_ata,
            fee_vault: m.fee_vault,
            fee_vault_share_ata: m.fee_vault_share_ata,
            redemption: redemption_pda(&l.share_mint, holder),
            wsol_mint: m.wsol_mint,
            basket_share_ata: m.basket_share_ata,
            basket_wsol_ata: m.basket_wsol_ata,
            holder_wsol_ata: m.holder_wsol_ata,
            lb_pair: m.lb_pair,
            reserve_x: m.reserve_x,
            reserve_y: m.reserve_y,
            tight_position: m.tight_position,
            backstop_position: m.backstop_position,
            dlmm_event_authority: m.dlmm_event_authority,
            dlmm_program: m.dlmm_program,
            memo_program: m.memo_program,
            token_program: m.token_program,
            token_2022_program: m.token_2022_program,
            associated_token_program: m.associated_token_program,
            system_program: m.system_program,
        }
        .to_account_metas(None);
        accounts.extend(self.pool_tail(l));
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::RedeemBegin { shares }.data() }
    }

    pub fn redeem_components_ix(
        &self,
        l: &Launched,
        holder: &Pubkey,
        chunk: &[PositionArg],
        claims: &[Pubkey],
    ) -> Instruction {
        let mut accounts = basket::accounts::RedeemComponents {
            holder: *holder,
            basket: l.basket,
            share_mint: l.share_mint,
            redemption: redemption_pda(&l.share_mint, holder),
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            lb_pair: l.pool.lb_pair,
            tight_position: l.tight_position(self),
            backstop_position: self.backstop_or_placeholder(l),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(component_metas(&l.basket, &l.share_mint, holder, chunk));
        accounts.extend(self.pool_tail(l));
        accounts.extend(claims.iter().map(|c| AccountMeta::new(*c, false)));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::RedeemComponents { count: chunk.len() as u16 }.data(),
        }
    }

    pub fn claim_frozen_ix(&self, l: &Launched, wallet: &Pubkey, mint: &Pubkey) -> Instruction {
        let accounts = basket::accounts::ClaimFrozen {
            wallet: *wallet,
            basket: l.basket,
            share_mint: l.share_mint,
            claim: frozen_claim_pda(&l.share_mint, wallet, mint),
            position: position_pda(&l.share_mint, mint),
            vault: Env::ata(&l.basket, mint),
            wallet_ata: Env::ata(wallet, mint),
            mint: *mint,
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::ClaimFrozen {}.data() }
    }

    // ---- freeze authority helpers (admin is the freeze authority) ----

    pub fn freeze(&mut self, mint: &Pubkey, ata: &Pubkey) {
        let ix = spl_token::instruction::freeze_account(&spl_token::ID, ata, mint, &self.admin.pubkey(), &[]).unwrap();
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
    }

    pub fn thaw(&mut self, mint: &Pubkey, ata: &Pubkey) {
        let ix = spl_token::instruction::thaw_account(&spl_token::ID, ata, mint, &self.admin.pubkey(), &[]).unwrap();
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
    }
}
