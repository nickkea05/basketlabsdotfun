//! Harness for the book / rebalance instructions: `submit_book`,
//! `apply_book`, `execute_swap`, `close_position`, `finalize_rebalance`,
//! plus a DLMM venue pool between two mints that program PDAs (basket,
//! FeeVault, BUYBACK) can swap through (stands in for Jupiter in LiteSVM).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use litesvm::types::TransactionMetadata;
use solana_keypair::Keypair;
use solana_signer::Signer;

use basket::constants::*;
use basket::dlmm;
use basket::state::*;

use super::*;

pub fn pending_book_pda(share_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"pending", share_mint.as_ref()], &basket::id()).0
}

/// Weight-based turnover of moving from `old` to `new`: Σ|Δw| / 2.
pub fn turnover_bps(old: &[PositionArg], new: &[PositionArg]) -> u16 {
    let mut mints: Vec<Pubkey> = old.iter().chain(new).map(|p| p.mint).collect();
    mints.sort();
    mints.dedup();
    let w = |book: &[PositionArg], m: &Pubkey| book.iter().find(|p| p.mint == *m).map(|p| p.weight_bps).unwrap_or(0) as i32;
    let sum: i32 = mints.iter().map(|m| (w(new, m) - w(old, m)).abs()).sum();
    (sum / 2) as u16
}

/// A DLMM pool between two mints, admin-funded, one base unit of X per base
/// unit of Y at the active bin (bin 0), 70 bins of Spot liquidity either
/// side. Stands in for Jupiter as the allow-listed swap venue.
#[derive(Clone, Copy, Debug)]
pub struct VenuePool {
    pub pool: DlmmPool,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub lower: i32,
    pub upper: i32,
}

/// Kept for the old call sites: a component↔component venue.
pub type ComponentPool = VenuePool;

impl VenuePool {
    pub fn program_id(&self) -> Pubkey {
        LB_CLMM_ID
    }
}

impl Env {
    /// Mirror basket: keeper-managed book, sleeve 10% (band 5–15%).
    pub fn launch_mirror(&mut self, n: usize, nonce: u64) -> Launched {
        self.launch_with(n, nonce, |a| {
            a.basket_type = BASKET_TYPE_MIRROR;
            a.fixed_kind = 0;
            a.hosts_hash = [7u8; 32];
            a.host_weighting = HOST_WEIGHTING_EQUAL;
            a.sleeve_r_bps = 1_000;
            a.name = "Whale Mirror".into();
            a.symbol = "WHALE".into();
        })
    }

    /// Allow the DLMM program as a swap venue (tests only; mainnet = Jupiter).
    pub fn allow_dlmm_swaps(&mut self) {
        self.update_config(ConfigUpdate { swap_programs: Some(vec![LB_CLMM_ID]), ..Default::default() });
    }

    /// Open a 1:1 venue pool between `mint_a` (X) and `mint_b` (Y) with
    /// `amount_a` / `amount_b` of liquidity over 70 bins around bin 0,
    /// funded and owned by the admin. For wSOL legs the admin wraps SOL.
    pub fn create_venue_pool(&mut self, mint_a: &Pubkey, mint_b: &Pubkey, amount_a: u64, amount_b: u64) -> VenuePool {
        let admin = self.admin.insecure_clone();
        for (mint, amount) in [(mint_a, amount_a), (mint_b, amount_b)] {
            if *mint == WSOL {
                self.wrap_sol(&admin, amount);
            } else {
                self.mint_to(mint, &admin.pubkey(), amount);
            }
        }
        // Venue pools are their own thing: bin step 10, 0.3 % fee.
        let pool = self.create_dlmm_pool_xy(&admin, *mint_a, *mint_b, 10, 30, 0, dlmm::COLLECT_FEE_MODE_INPUT_ONLY);
        let (lower, upper) = (-35, 34);
        self.ensure_bin_arrays(&pool, &admin, lower, upper);
        let (position, _) = self.init_position_pda(&pool, &admin, &admin, lower, upper - lower + 1);
        let ix = self.add_liquidity_ix(
            &pool,
            &position,
            &admin.pubkey(),
            amount_a,
            amount_b,
            0,
            lower,
            upper,
            basket::dlmm::lb_clmm::types::StrategyType::SpotImBalanced,
        );
        self.send_ok(&[ix], &admin, &[]);
        VenuePool { pool, mint_a: *mint_a, mint_b: *mint_b, lower, upper }
    }

    /// Component↔component venue with `amount` of each (old call sites).
    pub fn create_component_pool(&mut self, mint_a: &Pubkey, mint_b: &Pubkey, amount: u64) -> ComponentPool {
        self.create_venue_pool(mint_a, mint_b, amount, amount)
    }

    /// Venue `swap2` with `user` (a program PDA) as owner of both token
    /// accounts. The PDA is passed unsigned; the program signs it in the CPI.
    pub fn venue_swap_ix(&self, p: &VenuePool, user: &Pubkey, in_mint: &Pubkey, amount_in: u64) -> Instruction {
        let x_to_y = *in_mint == p.mint_a;
        let mut ix = self.swap_ix(&p.pool, user, x_to_y, amount_in, 0, p.lower, p.upper);
        for m in ix.accounts.iter_mut() {
            if m.pubkey == *user {
                m.is_signer = false;
            }
        }
        ix
    }

    /// Venue swap with the basket as user (old call sites).
    pub fn component_swap_ix(&self, p: &ComponentPool, basket_key: &Pubkey, in_mint: &Pubkey, _out_mint: &Pubkey, amount_in: u64) -> Instruction {
        self.venue_swap_ix(p, basket_key, in_mint, amount_in)
    }

    pub fn submit_book_ix(&self, l: &Launched, signer: &Pubkey, book: &[PositionArg], new_mints: &[Pubkey]) -> Instruction {
        let mut accounts = basket::accounts::SubmitBook {
            signer: *signer,
            config: config_pda(),
            whitelist: whitelist_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            pending_book: pending_book_pda(&l.share_mint),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        for m in new_mints {
            accounts.push(AccountMeta::new_readonly(*m, false));
            accounts.push(AccountMeta::new(position_pda(&l.share_mint, m), false));
            accounts.push(AccountMeta::new(Env::ata(&l.basket, m), false));
        }
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::SubmitBook { book: book.to_vec() }.data(),
        }
    }

    pub fn submit_book(&mut self, l: &Launched, signer: &Keypair, book: &[PositionArg], new_mints: &[Pubkey]) -> TransactionMetadata {
        let ix = self.submit_book_ix(l, &signer.pubkey(), book, new_mints);
        self.send_ok(&[ix], signer, &[])
    }

    /// Permissionless: create the Position + vault for a pending-book mint
    /// that did not come with its accounts at `submit_book`.
    pub fn open_position_ix(&self, l: &Launched, payer: &Pubkey, mint: &Pubkey) -> Instruction {
        let accounts = basket::accounts::OpenPosition {
            payer: *payer,
            config: config_pda(),
            whitelist: whitelist_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            pending_book: pending_book_pda(&l.share_mint),
            mint: *mint,
            position: position_pda(&l.share_mint, mint),
            vault: Env::ata(&l.basket, mint),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::OpenPosition {}.data() }
    }

    pub fn apply_book_ix(&self, l: &Launched, caller: &Pubkey, current: &[PositionArg]) -> Instruction {
        let creator = l.creator.pubkey();
        let mut accounts = basket::accounts::ApplyBook {
            caller: *caller,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            share_auth: share_auth_pda(&l.share_mint),
            creator,
            creator_share_ata: Env::ata(&creator, &l.share_mint),
            pending_book: pending_book_pda(&l.share_mint),
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            lb_pair: l.pool.lb_pair,
            tight_position: l.tight_position(self),
            backstop_position: self.backstop_or_placeholder(l),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(self.pool_tail(l));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::ApplyBook { current_book: current.to_vec() }.data(),
        }
    }

    /// `execute_swap` wrapping an inner venue instruction.
    pub fn execute_swap_ix(
        &self,
        l: &Launched,
        keeper: &Pubkey,
        in_mint: &Pubkey,
        out_mint: &Pubkey,
        amount_in: u64,
        min_amount_out: u64,
        inner: &Instruction,
    ) -> Instruction {
        let mut accounts = basket::accounts::ExecuteSwap {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            pending_book: pending_book_pda(&l.share_mint),
            in_position: position_pda(&l.share_mint, in_mint),
            in_vault: Env::ata(&l.basket, in_mint),
            out_position: position_pda(&l.share_mint, out_mint),
            out_vault: Env::ata(&l.basket, out_mint),
            swap_program: inner.program_id,
        }
        .to_account_metas(None);
        // The basket PDA cannot sign the outer transaction; the program adds
        // its signature when it re-issues the inner instruction.
        accounts.extend(inner.accounts.iter().cloned().map(|mut m| {
            if m.pubkey == l.basket {
                m.is_signer = false;
            }
            m
        }));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::ExecuteSwap { amount_in, min_amount_out, data: inner.data.clone() }.data(),
        }
    }

    pub fn close_position_ix(&self, l: &Launched, keeper: &Pubkey, mint: &Pubkey) -> Instruction {
        let accounts = basket::accounts::ClosePosition {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            pending_book: pending_book_pda(&l.share_mint),
            position: position_pda(&l.share_mint, mint),
            vault: Env::ata(&l.basket, mint),
            mint: *mint,
            rent_payer: l.payer.pubkey(),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
        }
        .to_account_metas(None);
        Instruction { program_id: basket::id(), accounts, data: basket::instruction::ClosePosition {}.data() }
    }

    pub fn finalize_ix(&self, l: &Launched, keeper: &Pubkey, entries: &[PositionArg]) -> Instruction {
        let pb: PendingBook = self.load(&pending_book_pda(&l.share_mint));
        self.finalize_ix_with_payer(l, keeper, entries, &pb.payer)
    }

    /// For the "no pending book" negative case: any payer key.
    pub fn finalize_ix_without_pending(&self, l: &Launched, keeper: &Pubkey, entries: &[PositionArg]) -> Instruction {
        self.finalize_ix_with_payer(l, keeper, entries, keeper)
    }

    fn finalize_ix_with_payer(&self, l: &Launched, keeper: &Pubkey, entries: &[PositionArg], payer: &Pubkey) -> Instruction {
        let mut accounts = basket::accounts::FinalizeRebalance {
            keeper: *keeper,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            pending_book: pending_book_pda(&l.share_mint),
            pending_payer: *payer,
        }
        .to_account_metas(None);
        for e in entries {
            accounts.push(AccountMeta::new(position_pda(&l.share_mint, &e.mint), false));
        }
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::FinalizeRebalance { entries: entries.to_vec() }.data(),
        }
    }

    /// Full happy path for a keeper-run rebalance where all swaps are done:
    /// close removed positions, finalize in chunks of 8.
    pub fn finish_rebalance(&mut self, l: &Launched, target: &[PositionArg], removed: &[Pubkey]) {
        let keeper = self.keeper.insecure_clone();
        for m in removed {
            let ix = self.close_position_ix(l, &keeper.pubkey(), m);
            self.send_ok(&[ix], &keeper, &[]);
        }
        for chunk in target.chunks(POSITIONS_PER_TX) {
            let ix = self.finalize_ix(l, &keeper.pubkey(), chunk);
            self.send_ok(&[ix], &keeper, &[]);
        }
    }

    pub fn position(&self, l: &Launched, mint: &Pubkey) -> Option<Position> {
        self.account(&position_pda(&l.share_mint, mint)).map(|_| self.load(&position_pda(&l.share_mint, mint)))
    }
}
