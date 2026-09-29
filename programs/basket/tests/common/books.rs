//! Harness for the book / rebalance instructions: `submit_book`,
//! `apply_book`, `execute_swap`, `close_position`, `finalize_rebalance`,
//! plus a component↔component cp-amm pool the basket can swap through
//! (stands in for Jupiter in LiteSVM).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use litesvm::types::TransactionMetadata;
use solana_keypair::Keypair;
use solana_signer::Signer;

use basket::constants::*;
use basket::damm::{self, cp_amm};
use basket::math;
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

/// A cp-amm pool between two component mints, admin-funded, price 1:1.
pub struct ComponentPool {
    pub pool: Pubkey,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
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

    /// Allow the cp-amm program as a swap venue (tests only; mainnet = Jupiter).
    pub fn allow_cp_amm_swaps(&mut self) {
        let ix = self.admin_ix(
            basket::instruction::UpdateConfig {
                update: ConfigUpdate { swap_programs: Some(vec![CP_AMM_ID]), ..Default::default() },
            }
            .data(),
        );
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
    }

    /// Open a full-range 1:1 pool between two 6-dp component mints with
    /// `amount` of each, funded and owned by the admin.
    pub fn create_component_pool(&mut self, mint_a: &Pubkey, mint_b: &Pubkey, amount: u64) -> ComponentPool {
        let admin = self.admin.insecure_clone();
        self.mint_to(mint_a, &admin.pubkey(), amount);
        self.mint_to(mint_b, &admin.pubkey(), amount);
        let nft = Keypair::new();
        let pool = damm::customizable_pool(mint_a, mint_b);
        let sqrt_price: u128 = 1u128 << 64;
        let liquidity = math::liquidity_from_b(amount, damm::MIN_SQRT_PRICE, sqrt_price).unwrap();
        let accounts = cp_amm::client::accounts::InitializeCustomizablePool {
            creator: admin.pubkey(),
            position_nft_mint: nft.pubkey(),
            position_nft_account: damm::position_nft_account(&nft.pubkey()),
            payer: admin.pubkey(),
            pool_authority: damm::pool_authority(),
            pool,
            position: damm::position(&nft.pubkey()),
            token_a_mint: *mint_a,
            token_b_mint: *mint_b,
            token_a_vault: damm::token_vault(mint_a, &pool),
            token_b_vault: damm::token_vault(mint_b, &pool),
            payer_token_a: Env::ata(&admin.pubkey(), mint_a),
            payer_token_b: Env::ata(&admin.pubkey(), mint_b),
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            system_program: anchor_lang::system_program::ID,
            event_authority: damm::event_authority(),
            program: CP_AMM_ID,
        }
        .to_account_metas(None);
        let ix = Instruction {
            program_id: CP_AMM_ID,
            accounts,
            data: cp_amm::client::args::InitializeCustomizablePool {
                params: cp_amm::types::InitializeCustomizablePoolParameters {
                    pool_fees: cp_amm::types::PoolFeeParameters {
                        base_fee: damm::flat_base_fee(damm::bps_to_fee_numerator(30)),
                        compounding_fee_bps: 0,
                        padding: 0,
                        dynamic_fee: None,
                    },
                    sqrt_min_price: damm::MIN_SQRT_PRICE,
                    sqrt_max_price: damm::MAX_SQRT_PRICE,
                    has_alpha_vault: false,
                    liquidity,
                    sqrt_price,
                    activation_type: damm::ACTIVATION_TYPE_TIMESTAMP,
                    collect_fee_mode: damm::COLLECT_FEE_MODE_BOTH,
                    activation_point: None,
                },
            }
            .data(),
        };
        self.send_ok(&[ix], &admin, &[&nft]);
        ComponentPool { pool, mint_a: *mint_a, mint_b: *mint_b }
    }

    /// cp-amm `swap` with the basket as payer/owner of both token accounts.
    /// The basket key is passed unsigned; the program signs it in the CPI.
    pub fn component_swap_ix(&self, p: &ComponentPool, basket_key: &Pubkey, in_mint: &Pubkey, out_mint: &Pubkey, amount_in: u64) -> Instruction {
        let mut accounts = cp_amm::client::accounts::Swap {
            pool_authority: damm::pool_authority(),
            pool: p.pool,
            input_token_account: Env::ata(basket_key, in_mint),
            output_token_account: Env::ata(basket_key, out_mint),
            token_a_vault: damm::token_vault(&p.mint_a, &p.pool),
            token_b_vault: damm::token_vault(&p.mint_b, &p.pool),
            token_a_mint: p.mint_a,
            token_b_mint: p.mint_b,
            payer: *basket_key,
            token_a_program: spl_token::ID,
            token_b_program: spl_token::ID,
            referral_token_account: None,
            event_authority: damm::event_authority(),
            program: CP_AMM_ID,
        }
        .to_account_metas(None);
        for m in accounts.iter_mut() {
            if m.pubkey == *basket_key {
                m.is_signer = false;
            }
            if m.pubkey == CP_AMM_ID {
                m.is_writable = false;
            }
        }
        Instruction {
            program_id: CP_AMM_ID,
            accounts,
            data: cp_amm::client::args::Swap {
                _params: cp_amm::types::SwapParameters { amount_in, minimum_amount_out: 0 },
            }
            .data(),
        }
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

    pub fn apply_book_ix(&self, l: &Launched, s: &SeededBasket, caller: &Pubkey, current: &[PositionArg]) -> Instruction {
        let creator = l.creator.pubkey();
        let accounts = basket::accounts::ApplyBook {
            caller: *caller,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            share_auth: share_auth_pda(&l.share_mint),
            creator,
            creator_share_ata: Env::ata(&creator, &l.share_mint),
            pending_book: pending_book_pda(&l.share_mint),
            pool: s.pool,
            pool_position: damm::position(&s.position_nft_mint),
            token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
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
