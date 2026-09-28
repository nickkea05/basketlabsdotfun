//! Harness for Managed baskets: launch helper, `crystallize` builder and an
//! off-chain mirror of the fee arithmetic.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use litesvm::types::TransactionMetadata;
use solana_signer::Signer;

use basket::constants::*;
use basket::math;
use basket::state::*;

use super::*;

/// What one `crystallize` call will mint, from the current chain state.
pub struct CrystallizeQuote {
    pub h_before: u64,
    pub mgmt_shares: u64,
    pub nav_after_mgmt: u64,
    pub perf_shares: u64,
    pub hwm_after: u64,
}

impl Env {
    /// Managed basket: sleeve 12% (band 8–20%), open gate, given perf fee.
    pub fn launch_managed(&mut self, n: usize, nonce: u64, perf_fee_bps: u16) -> Launched {
        self.launch_with(n, nonce, |a| {
            a.basket_type = BASKET_TYPE_MANAGED;
            a.fixed_kind = 0;
            a.sleeve_r_bps = 1_200;
            a.perf_fee_bps = perf_fee_bps;
            a.name = "Alpha Managed".into();
            a.symbol = "ALPHA".into();
        })
    }

    pub fn crystallize_ix(&self, l: &Launched, s: &SeededBasket, keeper: &Pubkey, nav: u64) -> Instruction {
        let creator = l.creator.pubkey();
        let accounts = basket::accounts::Crystallize {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            share_auth: share_auth_pda(&l.share_mint),
            creator,
            creator_share_ata: Env::ata(&creator, &l.share_mint),
            pool: s.pool,
            pool_position: basket::damm::position(&s.position_nft_mint),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::Crystallize { nav_lamports_per_share: nav }.data(),
        }
    }

    pub fn crystallize(&mut self, l: &Launched, s: &SeededBasket, nav: u64) -> TransactionMetadata {
        let keeper = self.keeper.insecure_clone();
        let ix = self.crystallize_ix(l, s, &keeper.pubkey(), nav);
        self.send_ok(&[ix], &keeper, &[])
    }

    pub fn crystallize_expect_err(&mut self, l: &Launched, s: &SeededBasket, nav: u64, code: u32) {
        let keeper = self.keeper.insecure_clone();
        let ix = self.crystallize_ix(l, s, &keeper.pubkey(), nav);
        self.send_expect_err(&[ix], &keeper, &[], code);
    }

    /// Mirror of the program's crystallize arithmetic at the current clock.
    pub fn crystallize_quote(&self, l: &Launched, s: &SeededBasket, nav: u64) -> CrystallizeQuote {
        let b: Basket = self.load(&l.basket);
        let h = self.holder_shares(l, s);
        let elapsed = self.now() - b.last_mgmt_accrual_ts;
        let mgmt = math::mgmt_fee_shares(h, b.fees.mgmt_fee_bps_per_year, elapsed).unwrap();
        let h1 = h + mgmt;
        let nav1 = math::diluted_nav(nav, h, mgmt).unwrap();
        let perf = math::perf_fee_shares(h1, nav1, b.hwm_nav_lamports, b.fees.perf_fee_bps).unwrap();
        let hwm_after =
            if nav1 > b.hwm_nav_lamports { math::diluted_nav(nav1, h1, perf).unwrap() } else { b.hwm_nav_lamports };
        CrystallizeQuote { h_before: h, mgmt_shares: mgmt, nav_after_mgmt: nav1, perf_shares: perf, hwm_after }
    }
}
