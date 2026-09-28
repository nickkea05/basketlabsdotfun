//! Harness helpers for `seed` / `mint` / the DAMM v2 sleeve: instruction
//! builders, pool/position readers and an off-chain mirror of the program's
//! share quote (what the frontend will compute before building a buy).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use litesvm::types::TransactionMetadata;
use solana_keypair::Keypair;
use solana_signer::Signer;

use basket::damm::{self, cp_amm};
use basket::math;
use basket::state::*;

use super::*;

/// What the first buyer brings to `seed`.
pub struct SeedPlan {
    pub deposits: Vec<u64>,
    pub initial_shares: u64,
    pub sleeve_lamports: u64,
    pub position_nft_mint: Keypair,
}

impl Clone for SeedPlan {
    fn clone(&self) -> Self {
        SeedPlan {
            deposits: self.deposits.clone(),
            initial_shares: self.initial_shares,
            sleeve_lamports: self.sleeve_lamports,
            position_nft_mint: Keypair::new(),
        }
    }
}

impl SeedPlan {
    /// 1_000 units (6 dp) of every component, 1_000 shares, 0.3 SOL sleeve:
    /// at r = 25% that implies 1.2 SOL of value, above the 1 SOL minimum.
    pub fn default_for(l: &Launched) -> Self {
        SeedPlan {
            deposits: vec![1_000_000_000; l.book.len()],
            initial_shares: 1_000_000_000,
            sleeve_lamports: 300_000_000,
            position_nft_mint: Keypair::new(),
        }
    }

    /// Mirror of the program's seed geometry for a profile.
    pub fn quote(&self, r_bps: u16, range: PriceRange) -> SeedQuote {
        let nominal = math::treasury_shares(self.initial_shares, r_bps).unwrap();
        let sqrt_price = math::sqrt_price_from_amounts(self.sleeve_lamports, nominal).unwrap();
        let (lo, hi) = math::sqrt_price_bounds(sqrt_price, range).unwrap();
        let q = math::sleeve_quote(self.sleeve_lamports, nominal, lo, sqrt_price, hi).unwrap();
        SeedQuote {
            nominal_treasury_shares: nominal,
            treasury_shares: q.amount_a,
            sleeve_lamports: q.amount_b,
            sqrt_price,
            sqrt_min: lo,
            sqrt_max: hi,
            liquidity: q.liquidity,
        }
    }
}

pub struct SeedQuote {
    pub nominal_treasury_shares: u64,
    pub treasury_shares: u64,
    pub sleeve_lamports: u64,
    pub sqrt_price: u128,
    pub sqrt_min: u128,
    pub sqrt_max: u128,
    pub liquidity: u128,
}

pub struct SeededBasket {
    pub pool: Pubkey,
    pub position_nft_mint: Pubkey,
    pub cu: u64,
}

pub struct MintResult {
    pub cu: u64,
    pub logs: Vec<String>,
}

/// Mirror of the program's `mint` arithmetic.
pub struct MintQuote {
    pub gross_shares: u64,
    pub r_bps: u16,
    /// `X·r/(1−r)`: what the SOL leg is priced from.
    pub nominal_treasury_shares: u64,
    /// Shares the pool actually takes for that SOL leg (range geometry).
    pub treasury_shares: u64,
    pub sleeve_lamports: u64,
    pub liquidity: u128,
}

pub struct PoolState {
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub sqrt_price: u128,
    pub sqrt_min_price: u128,
    pub sqrt_max_price: u128,
    pub liquidity: u128,
    pub token_a_amount: u64,
    pub token_b_amount: u64,
    pub collect_fee_mode: u8,
    pub base_fee_cliff_numerator: u64,
    pub base_fee_periods: u16,
    pub base_fee_period_frequency: u64,
    pub base_fee_reduction_factor: u64,
}

pub struct PositionState {
    pub pool: Pubkey,
    pub unlocked_liquidity: u128,
}

pub fn pool_pda(share_mint: &Pubkey) -> Pubkey {
    damm::customizable_pool(share_mint, &WSOL)
}

/// `[position, vault, owner_ata, mint]` per book entry.
pub fn component_metas(basket_key: &Pubkey, share_mint: &Pubkey, owner: &Pubkey, chunk: &[PositionArg]) -> Vec<AccountMeta> {
    let mut metas = Vec::new();
    for p in chunk {
        metas.push(AccountMeta::new(position_pda(share_mint, &p.mint), false));
        metas.push(AccountMeta::new(Env::ata(basket_key, &p.mint), false));
        metas.push(AccountMeta::new(Env::ata(owner, &p.mint), false));
        metas.push(AccountMeta::new_readonly(p.mint, false));
    }
    metas
}

impl Env {
    pub fn pool_state(&self, pool: &Pubkey) -> PoolState {
        let acc = self.account(pool).expect("pool missing");
        assert_eq!(acc.owner, CP_AMM_ID);
        let p: cp_amm::accounts::Pool =
            bytemuck::pod_read_unaligned(&acc.data[8..8 + std::mem::size_of::<cp_amm::accounts::Pool>()]);
        let d = &p.pool_fees.base_fee.base_fee_info.data;
        PoolState {
            token_a_mint: p.token_a_mint,
            token_b_mint: p.token_b_mint,
            token_a_vault: p.token_a_vault,
            token_b_vault: p.token_b_vault,
            sqrt_price: p.sqrt_price,
            sqrt_min_price: p.sqrt_min_price,
            sqrt_max_price: p.sqrt_max_price,
            liquidity: p.liquidity,
            token_a_amount: p.token_a_amount,
            token_b_amount: p.token_b_amount,
            collect_fee_mode: p.collect_fee_mode,
            base_fee_cliff_numerator: u64::from_le_bytes(d[0..8].try_into().unwrap()),
            base_fee_periods: u16::from_le_bytes(d[14..16].try_into().unwrap()),
            base_fee_period_frequency: u64::from_le_bytes(d[16..24].try_into().unwrap()),
            base_fee_reduction_factor: u64::from_le_bytes(d[24..32].try_into().unwrap()),
        }
    }

    pub fn position_state(&self, position: &Pubkey) -> PositionState {
        let acc = self.account(position).expect("position missing");
        let p: cp_amm::accounts::Position =
            bytemuck::pod_read_unaligned(&acc.data[8..8 + std::mem::size_of::<cp_amm::accounts::Position>()]);
        PositionState { pool: p.pool, unlocked_liquidity: p.unlocked_liquidity }
    }

    /// Mint `deposits[i]` of component `i` into `owner`'s ATA.
    pub fn fund_components_for(&mut self, owner: &Pubkey, l: &Launched, deposits: &[u64]) {
        for (mint, amount) in l.mints.iter().zip(deposits) {
            self.mint_to(mint, owner, *amount);
        }
    }

    pub fn fund_components(&mut self, l: &Launched, plan: &SeedPlan) {
        let payer = l.payer.pubkey();
        self.fund_components_for(&payer, l, &plan.deposits);
    }

    pub fn seed_ix(&self, l: &Launched, plan: &SeedPlan) -> Instruction {
        self.seed_ix_for(l, plan, &l.book)
    }

    pub fn seed_ix_for(&self, l: &Launched, plan: &SeedPlan, chunk: &[PositionArg]) -> Instruction {
        let payer = l.payer.pubkey();
        let nft = plan.position_nft_mint.pubkey();
        let pool = pool_pda(&l.share_mint);
        let fee_vault = fee_vault_pda(&l.share_mint);
        let mut accounts = basket::accounts::Seed {
            payer,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            share_auth: share_auth_pda(&l.share_mint),
            payer_share_ata: Env::ata(&payer, &l.share_mint),
            payer_wsol_ata: Env::ata(&payer, &WSOL),
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            treasury: self.treasury.pubkey(),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            basket_wsol_ata: Env::ata(&l.basket, &WSOL),
            position_nft_mint: nft,
            position_nft_account: damm::position_nft_account(&nft),
            pool_authority: damm::pool_authority(),
            pool,
            pool_position: damm::position(&nft),
            token_a_vault: damm::token_vault(&l.share_mint, &pool),
            token_b_vault: damm::token_vault(&WSOL, &pool),
            cp_amm_program: CP_AMM_ID,
            event_authority: damm::event_authority(),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(component_metas(&l.basket, &l.share_mint, &payer, chunk));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::Seed {
                args: SeedArgs {
                    deposits: plan.deposits.clone(),
                    initial_shares: plan.initial_shares,
                    sleeve_lamports: plan.sleeve_lamports,
                },
            }
            .data(),
        }
    }

    /// Funds the components and expects `seed` to fail with `code`.
    pub fn seed_expect_err(&mut self, l: &Launched, plan: &SeedPlan, code: u32) {
        self.fund_components(l, plan);
        let ix = self.seed_ix(l, plan);
        self.send_expect_err(&[ix], &l.payer, &[&plan.position_nft_mint], code);
    }

    /// Funds the components and seeds; panics on failure.
    pub fn seed(&mut self, l: &Launched, plan: &SeedPlan) -> SeededBasket {
        self.fund_components(l, plan);
        let ix = self.seed_ix(l, plan);
        let meta: TransactionMetadata = self.send_ok(&[ix], &l.payer, &[&plan.position_nft_mint]);
        SeededBasket {
            pool: pool_pda(&l.share_mint),
            position_nft_mint: plan.position_nft_mint.pubkey(),
            cu: meta.compute_units_consumed,
        }
    }

    pub fn mint_ix(
        &self,
        l: &Launched,
        s: &SeededBasket,
        buyer: &Pubkey,
        deposits: &[u64],
        min_shares_out: u64,
        max_sleeve_lamports: u64,
    ) -> Instruction {
        let fee_vault = fee_vault_pda(&l.share_mint);
        let mut accounts = basket::accounts::MintShares {
            payer: *buyer,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            share_auth: share_auth_pda(&l.share_mint),
            payer_share_ata: Env::ata(buyer, &l.share_mint),
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            wsol_mint: WSOL,
            basket_share_ata: Env::ata(&l.basket, &l.share_mint),
            basket_wsol_ata: Env::ata(&l.basket, &WSOL),
            position_nft_account: damm::position_nft_account(&s.position_nft_mint),
            pool: s.pool,
            pool_position: damm::position(&s.position_nft_mint),
            token_a_vault: damm::token_vault(&l.share_mint, &s.pool),
            token_b_vault: damm::token_vault(&WSOL, &s.pool),
            cp_amm_program: CP_AMM_ID,
            event_authority: damm::event_authority(),
            token_program: spl_token::ID,
            token_2022_program: anchor_spl::token_2022::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(component_metas(&l.basket, &l.share_mint, buyer, &l.book));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::Mint {
                args: MintArgs { deposits: deposits.to_vec(), min_shares_out, max_sleeve_lamports },
            }
            .data(),
        }
    }

    pub fn mint(
        &mut self,
        l: &Launched,
        s: &SeededBasket,
        buyer: &Keypair,
        deposits: &[u64],
        min_shares_out: u64,
        max_sleeve_lamports: u64,
    ) -> MintResult {
        let ix = self.mint_ix(l, s, &buyer.pubkey(), deposits, min_shares_out, max_sleeve_lamports);
        let meta = self.send_ok(&[ix], buyer, &[]);
        MintResult { cu: meta.compute_units_consumed, logs: meta.logs }
    }

    /// Holder shares outstanding: supply − shares sitting in the basket's
    /// pool position (+ shares mid-redemption).
    pub fn holder_shares(&self, l: &Launched, s: &SeededBasket) -> u64 {
        let pool = self.pool_state(&s.pool);
        let pos = self.position_state(&damm::position(&s.position_nft_mint));
        let (a, _) =
            math::position_amounts(pos.unlocked_liquidity, pool.sqrt_min_price, pool.sqrt_price, pool.sqrt_max_price)
                .unwrap();
        let b: Basket = self.load(&l.basket);
        self.mint_supply(&l.share_mint) - a + b.pending_redeem_shares
    }

    /// Off-chain mirror of `mint`: creation-unit shares, step-rule `r`,
    /// treasury shares and the SOL the pool will pull.
    pub fn mint_quote(&self, l: &Launched, s: &SeededBasket, deposits: &[u64]) -> MintQuote {
        let vaults: Vec<u64> = l.mints.iter().map(|m| self.token_amount(&Env::ata(&l.basket, m))).collect();
        let h = self.holder_shares(l, s);
        let gross_shares = math::shares_for_deposits(deposits, &vaults, h).unwrap();
        let b: Basket = self.load(&l.basket);
        let pool = self.pool_state(&s.pool);
        let r_bps = if pool.token_b_amount < b.sleeve.step_threshold_lamports { b.sleeve.step_r_bps } else { b.sleeve.r_bps };
        let y = math::treasury_shares(gross_shares, r_bps).unwrap();
        let target_b = math::lamports_for_shares(y, pool.sqrt_price).unwrap();
        let q = math::sleeve_quote(target_b, y, pool.sqrt_min_price, pool.sqrt_price, pool.sqrt_max_price).unwrap();
        MintQuote {
            gross_shares,
            r_bps,
            nominal_treasury_shares: y,
            treasury_shares: q.amount_a,
            sleeve_lamports: q.amount_b,
            liquidity: q.liquidity,
        }
    }
}
