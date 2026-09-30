//! Harness for the keeper's position instructions: `place_backstop`,
//! `fund_backstop`, `withdraw_backstop`, `close_backstop`, `recenter_tight`.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use solana_signer::Signer;

use basket::constants::*;
use basket::dlmm;
use basket::state::*;

use super::*;

impl Env {
    /// The `KeeperPool` account list. `backstop` and `new_position` default
    /// to the basket's current backstop (or the placeholder) and the
    /// placeholder respectively.
    pub fn keeper_pool_ix(
        &self,
        l: &Launched,
        keeper: &Pubkey,
        backstop: Option<Pubkey>,
        new_position: Option<Pubkey>,
        tail: Vec<AccountMeta>,
        data: Vec<u8>,
    ) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let mut accounts = basket::accounts::KeeperPool {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            share_auth: share_auth_pda(&l.share_mint),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            basket_wsol_ata: Env::ata(&l.basket, &WSOL),
            fee_vault,
            fee_vault_wsol_ata: Env::ata(&fee_vault, &WSOL),
            lb_pair: l.pool.lb_pair,
            reserve_x: l.pool.reserve_x,
            reserve_y: l.pool.reserve_y,
            tight_position: l.tight_position(self),
            backstop_position: backstop.unwrap_or_else(|| self.backstop_or_placeholder(l)),
            new_position: new_position.unwrap_or_else(|| l.placeholder()),
            dlmm_event_authority: dlmm::event_authority(),
            dlmm_program: LB_CLMM_ID,
            memo_program: MEMO_ID,
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
            rent: solana_sdk_ids::sysvar::rent::ID,
        }
        .to_account_metas(None);
        accounts.extend(tail);
        Instruction { program_id: basket::id(), accounts, data }
    }

    /// Where the keeper will place the backstop from the current state:
    /// `(lower, upper, lo_index, hi_index)`.
    pub fn backstop_placement(&self, l: &Launched) -> (i32, i32, i64, i64) {
        let b: Basket = self.load(&l.basket);
        let preset = b.pool.preset;
        let center = if b.backstop_state == BACKSTOP_UNPLACED { b.pool.launch_active_id } else { l.pool.active_id(self) };
        let (lo, hi) = dlmm::backstop_arrays(center, preset.backstop_half_width_bins as i32, preset.backstop_max_arrays);
        (dlmm::array_lower_bin(lo), dlmm::array_upper_bin(hi), lo, hi)
    }

    /// The backstop position PDA for the placement above (`base` = share_auth).
    pub fn backstop_position_for(&self, l: &Launched) -> Pubkey {
        let (lower, upper, _, _) = self.backstop_placement(l);
        dlmm::position_pda(&l.pool.lb_pair, &share_auth_pda(&l.share_mint), lower, upper - lower + 1)
    }

    pub fn place_backstop_ix(&self, l: &Launched, keeper: &Pubkey) -> Instruction {
        let (lower, upper, _, _) = self.backstop_placement(l);
        let position = self.backstop_position_for(l);
        let tail = l.pool.bin_array_metas(lower, upper);
        self.keeper_pool_ix(l, keeper, Some(position), None, tail, basket::instruction::PlaceBackstop {}.data())
    }

    pub fn place_backstop(&mut self, l: &Launched) -> TransactionMetadata {
        let keeper = self.keeper.insecure_clone();
        let ix = self.place_backstop_ix(l, &keeper.pubkey());
        self.send_ok(&[ix], &keeper, &[])
    }

    pub fn fund_backstop_ix(&self, l: &Launched, keeper: &Pubkey, array_index: i64) -> Instruction {
        let tail = vec![AccountMeta::new(l.pool.bin_array(array_index), false)];
        self.keeper_pool_ix(l, keeper, None, None, tail, basket::instruction::FundBackstop { array_index }.data())
    }

    /// Fund every bin array of the placed backstop (one tx each). Returns CU per call.
    pub fn fund_backstop_all(&mut self, l: &Launched) -> Vec<u64> {
        let keeper = self.keeper.insecure_clone();
        let b: Basket = self.load(&l.basket);
        let (lo, hi) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
        let mut cus = vec![];
        for idx in lo..=hi {
            let ix = self.fund_backstop_ix(l, &keeper.pubkey(), idx);
            let m = self.send_ok(&[ix], &keeper, &[]);
            cus.push(m.compute_units_consumed);
        }
        cus
    }

    /// `place_backstop` + all `fund_backstop` chunks.
    pub fn place_and_fund_backstop(&mut self, l: &Launched) -> Vec<u64> {
        self.place_backstop(l);
        self.fund_backstop_all(l)
    }

    pub fn withdraw_backstop_ix(&self, l: &Launched, keeper: &Pubkey, array_index: i64) -> Instruction {
        let tail = vec![AccountMeta::new(l.pool.bin_array(array_index), false)];
        self.keeper_pool_ix(l, keeper, None, None, tail, basket::instruction::WithdrawBackstop { array_index }.data())
    }

    pub fn withdraw_backstop_all(&mut self, l: &Launched) -> Vec<u64> {
        let keeper = self.keeper.insecure_clone();
        let b: Basket = self.load(&l.basket);
        let (lo, hi) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
        let mut cus = vec![];
        for idx in lo..=hi {
            let ix = self.withdraw_backstop_ix(l, &keeper.pubkey(), idx);
            let m = self.send_ok(&[ix], &keeper, &[]);
            cus.push(m.compute_units_consumed);
        }
        cus
    }

    pub fn close_backstop_ix(&self, l: &Launched, keeper: &Pubkey) -> Instruction {
        self.keeper_pool_ix(l, keeper, None, None, vec![], basket::instruction::CloseBackstop {}.data())
    }

    pub fn close_backstop(&mut self, l: &Launched) -> TransactionMetadata {
        let keeper = self.keeper.insecure_clone();
        let ix = self.close_backstop_ix(l, &keeper.pubkey());
        self.send_ok(&[ix], &keeper, &[])
    }

    /// The tight position `recenter_tight` will open around `active_id`.
    pub fn new_tight_for(&self, l: &Launched, active_id: i32) -> (Pubkey, i32, i32) {
        let (lower, upper) = self.tight_range_at(l, active_id);
        (dlmm::position_pda(&l.pool.lb_pair, &l.basket, lower, upper - lower + 1), lower, upper)
    }

    pub fn recenter_tight_ix(&self, l: &Launched, keeper: &Pubkey) -> Instruction {
        let b: Basket = self.load(&l.basket);
        let active = l.pool.active_id(self);
        let (new_position, lower, upper) = self.new_tight_for(l, active);
        let mut tail = l.pool.bin_array_metas(b.tight.lower_bin_id, b.tight.upper_bin_id);
        for m in l.pool.bin_array_metas(lower, upper) {
            if !tail.iter().any(|x| x.pubkey == m.pubkey) {
                tail.push(m);
            }
        }
        self.keeper_pool_ix(l, keeper, None, Some(new_position), tail, basket::instruction::RecenterTight {}.data())
    }

    pub fn recenter_tight(&mut self, l: &Launched) -> std::result::Result<TransactionMetadata, FailedTransactionMetadata> {
        let keeper = self.keeper.insecure_clone();
        let ix = self.recenter_tight_ix(l, &keeper.pubkey());
        self.send(&[ix], &keeper, &[])
    }

    pub fn recenter_tight_ok(&mut self, l: &Launched) -> TransactionMetadata {
        match self.recenter_tight(l) {
            Ok(m) => m,
            Err(e) => panic!("recenter_tight failed: {:?}\nlogs:\n{}", e.err, e.meta.logs.join("\n")),
        }
    }

    /// Whether the program would accept a re-centre right now.
    pub fn recenter_allowed(&self, l: &Launched) -> bool {
        let b: Basket = self.load(&l.basket);
        basket::instructions::keeper::recenter_allowed(&b, &self.config(), l.pool.active_id(self), self.now())
    }
}
