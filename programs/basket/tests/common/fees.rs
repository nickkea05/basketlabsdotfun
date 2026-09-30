//! Harness for the fee path: trades through the basket's DLMM pool so the
//! positions accrue swap fees, and builders for `claim_pool_fees`,
//! `sweep_fees` / `sweep_fees_components` and `settle_fees`.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use solana_keypair::Keypair;
use solana_signer::Signer;

use basket::dlmm;
use basket::state::*;

use super::*;

impl Env {
    /// Trade through the basket's pool directly (not through our program).
    /// `x_to_y = false`: SOL in, shares out.
    pub fn swap(&mut self, l: &Launched, trader: &Keypair, amount_in: u64, x_to_y: bool) -> u64 {
        let b: Basket = self.load(&l.basket);
        // Cover every array the basket's positions span, so the swap can
        // walk as far as there is liquidity.
        let mut lower = b.tight.lower_bin_id;
        let mut upper = b.tight.upper_bin_id;
        if b.backstop.is_set() {
            lower = lower.min(b.backstop.lower_bin_id);
            upper = upper.max(b.backstop.upper_bin_id);
        }
        let mut ix = self.swap_ix(&l.pool, &trader.pubkey(), x_to_y, amount_in, 0, lower, upper);
        // Only arrays that exist can be passed; a Spot add never touches
        // arrays outside the positions' ranges.
        let fixed = ix.accounts.len() - l.pool.bin_array_metas(lower, upper).len();
        ix.accounts.truncate(fixed);
        ix.accounts.extend(self.existing_bin_array_metas(&l.pool, lower, upper));
        let out_mint = if x_to_y { WSOL } else { l.share_mint };
        let create = anchor_spl::associated_token::spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &trader.pubkey(),
            &trader.pubkey(),
            &out_mint,
            &spl_token::ID,
        );
        let out_ata = Env::ata(&trader.pubkey(), &out_mint);
        let before = self.token_amount(&out_ata);
        self.send_ok(&[create, ix], trader, &[]);
        self.token_amount(&out_ata) - before
    }

    /// Buy shares with `lamports` of SOL through the pool; returns shares out.
    pub fn buy_shares(&mut self, l: &Launched, trader: &Keypair, lamports: u64) -> u64 {
        self.wrap_sol(trader, lamports);
        self.swap(l, trader, lamports, false)
    }

    /// Sell `shares` into the pool for SOL; returns lamports out (as wSOL).
    pub fn sell_shares(&mut self, l: &Launched, trader: &Keypair, shares: u64) -> u64 {
        self.swap(l, trader, shares, true)
    }

    /// A trader with SOL who has bought `lamports` worth of shares.
    pub fn trader_who_bought(&mut self, l: &Launched, lamports: u64) -> (Keypair, u64) {
        let trader = self.fund(lamports + 5 * LAMPORTS);
        let got = self.buy_shares(l, &trader, lamports);
        (trader, got)
    }

    pub fn claim_pool_fees_ix(&self, l: &Launched, caller: &Pubkey, position: &Pubkey, min_bin_id: i32, max_bin_id: i32) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let config = self.config();
        let mut accounts = basket::accounts::ClaimPoolFees {
            caller: *caller,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault,
            fee_vault_wsol_ata: Env::ata(&fee_vault, &WSOL),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            basket_wsol_ata: Env::ata(&l.basket, &WSOL),
            position: *position,
            lb_pair: l.pool.lb_pair,
            reserve_x: l.pool.reserve_x,
            reserve_y: l.pool.reserve_y,
            dlmm_event_authority: dlmm::event_authority(),
            dlmm_program: LB_CLMM_ID,
            memo_program: MEMO_ID,
            creator: l.creator.pubkey(),
            buyback_vault: buyback_vault_pda(),
            team_wallet: config.team_wallet,
            prize_vault: prize_vault_pda(),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(self.bin_array_metas(l, min_bin_id, max_bin_id));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::ClaimPoolFees { min_bin_id, max_bin_id }.data(),
        }
    }

    /// Claim the whole tight position. Returns lamports routed (creator +
    /// buyback + team + prize deltas) and the CU.
    pub fn claim_tight_fees(&mut self, l: &Launched, caller: &Keypair) -> (u64, u64) {
        let b: Basket = self.load(&l.basket);
        let ix = self.claim_pool_fees_ix(l, &caller.pubkey(), &b.tight.key, b.tight.lower_bin_id, b.tight.upper_bin_id);
        let before = self.routed_total(l);
        let m = self.send_ok(&[ix], caller, &[]);
        (self.routed_total(l) - before, m.compute_units_consumed)
    }

    /// Claim the backstop one bin array at a time. Returns lamports routed.
    pub fn claim_backstop_fees(&mut self, l: &Launched, caller: &Keypair) -> u64 {
        let b: Basket = self.load(&l.basket);
        let (lo, hi) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
        let before = self.routed_total(l);
        for idx in lo..=hi {
            let lower = dlmm::array_lower_bin(idx).max(b.backstop.lower_bin_id);
            let upper = dlmm::array_upper_bin(idx).min(b.backstop.upper_bin_id);
            let ix = self.claim_pool_fees_ix(l, &caller.pubkey(), &b.backstop.key, lower, upper);
            self.send_ok(&[ix], caller, &[]);
        }
        self.routed_total(l) - before
    }

    /// Sum of the four fee destinations' balances (creator wallet, BUYBACK,
    /// team wallet, PRIZE) plus what the FeeVault still owes the creator.
    pub fn routed_total(&self, l: &Launched) -> u64 {
        let config = self.config();
        let fv = self.fee_vault(l);
        self.lamports(&l.creator.pubkey())
            + self.lamports(&buyback_vault_pda())
            + self.lamports(&config.team_wallet)
            + self.lamports(&prize_vault_pda())
            + fv.creator_owed_lamports
    }

    pub fn sweep_fees_ix(&self, l: &Launched, caller: &Pubkey) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let creator = l.creator.pubkey();
        let config = self.config();
        let mut accounts = basket::accounts::SweepFees {
            caller: *caller,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            fee_vault_wsol_ata: Env::ata(&fee_vault, &WSOL),
            creator,
            creator_share_ata: Env::ata(&creator, &l.share_mint),
            redemption: redemption_pda(&l.share_mint, &fee_vault),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            basket_wsol_ata: Env::ata(&l.basket, &WSOL),
            lb_pair: l.pool.lb_pair,
            reserve_x: l.pool.reserve_x,
            reserve_y: l.pool.reserve_y,
            tight_position: l.tight_position(self),
            backstop_position: self.backstop_or_placeholder(l),
            dlmm_event_authority: dlmm::event_authority(),
            dlmm_program: LB_CLMM_ID,
            memo_program: MEMO_ID,
            buyback_vault: buyback_vault_pda(),
            team_wallet: config.team_wallet,
            prize_vault: prize_vault_pda(),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(self.pool_tail(l));
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::SweepFees {}.data() }
    }

    pub fn sweep_fees_components_ix(&self, l: &Launched, caller: &Pubkey, chunk: &[PositionArg]) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let mut accounts = basket::accounts::SweepFeesComponents {
            caller: *caller,
            basket: l.basket,
            share_mint: l.share_mint,
            fee_vault,
            redemption: redemption_pda(&l.share_mint, &fee_vault),
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
        accounts.extend(component_metas(&l.basket, &l.share_mint, &fee_vault, chunk));
        accounts.extend(self.pool_tail(l));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::SweepFeesComponents { count: chunk.len() as u16 }.data(),
        }
    }

    /// `sweep_fees` + every component chunk. Returns the CU of the first tx.
    pub fn sweep_fees(&mut self, l: &Launched, caller: &Keypair) -> u64 {
        let ix = self.sweep_fees_ix(l, &caller.pubkey());
        let m = self.send_ok(&[ix], caller, &[]);
        for chunk in l.book.chunks(POSITIONS_PER_TX) {
            let ix = self.sweep_fees_components_ix(l, &caller.pubkey(), chunk);
            self.send_ok(&[ix], caller, &[]);
        }
        m.compute_units_consumed
    }

    /// `settle_fees` wrapping an inner venue swap of the FeeVault's `in_mint`
    /// ATA into its wSOL ATA. The FeeVault key is passed unsigned; the
    /// program signs it when it re-issues the inner instruction.
    pub fn settle_fees_ix(&self, l: &Launched, keeper: &Pubkey, in_mint: &Pubkey, amount_in: u64, min_amount_out: u64, inner: &Instruction) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let config = self.config();
        let mut accounts = basket::accounts::SettleFees {
            keeper: *keeper,
            config: config_pda(),
            share_mint: l.share_mint,
            fee_vault,
            in_ata: Env::ata(&fee_vault, in_mint),
            fee_vault_wsol_ata: Env::ata(&fee_vault, &WSOL),
            wsol_mint: WSOL,
            swap_program: inner.program_id,
            creator: l.creator.pubkey(),
            buyback_vault: buyback_vault_pda(),
            team_wallet: config.team_wallet,
            prize_vault: prize_vault_pda(),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(inner.accounts.iter().cloned().map(|mut m| {
            if m.pubkey == fee_vault {
                m.is_signer = false;
            }
            m
        }));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::SettleFees { amount_in, min_amount_out, data: inner.data.clone() }.data(),
        }
    }

    pub fn fee_vault(&self, l: &Launched) -> FeeVault {
        self.load(&fee_vault_pda(&l.share_mint))
    }

    /// Lamports the FeeVault holds above its own rent.
    pub fn fee_vault_free_lamports(&self, l: &Launched) -> u64 {
        self.free_lamports(&fee_vault_pda(&l.share_mint))
    }

    pub fn fee_shares(&self, l: &Launched) -> u64 {
        self.token_amount(&Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint))
    }
}
