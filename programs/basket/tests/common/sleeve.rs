//! Harness helpers for `seed` / `mint` / the DLMM sleeve: instruction
//! builders, pool/position readers and an off-chain mirror of the program's
//! share quote (what the frontend will compute before building a buy).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use litesvm::types::TransactionMetadata;
use solana_keypair::Keypair;
use solana_signer::Signer;

use basket::constants::*;
use basket::dlmm;
use basket::math;
use basket::state::*;

use super::*;

/// What the first buyer brings to `seed`.
#[derive(Clone, Debug)]
pub struct SeedPlan {
    pub deposits: Vec<u64>,
    pub initial_shares: u64,
    pub sleeve_lamports: u64,
}

impl SeedPlan {
    /// 1_000 units (6 dp) of every component, 1_000 shares, and the SOL leg
    /// the launch bin implies: Y = XÂ·r/(1âˆ’r) treasury shares at 1 lamport
    /// per base unit â†’ 0.333 SOL. At r = 25 % that implies 1.33 SOL of
    /// value, above the 1 SOL minimum.
    pub fn default_for(l: &Launched) -> Self {
        let initial_shares = 1_000_000_000;
        let y = math::treasury_shares(initial_shares, DEFAULT_STEP_R_BPS).unwrap();
        let price = dlmm::bin_price_q64(l.args.launch_active_id, l.pool.bin_step).unwrap();
        SeedPlan {
            deposits: vec![1_000_000_000; l.book.len()],
            initial_shares,
            sleeve_lamports: dlmm::lamports_for_x(y, price).unwrap(),
        }
    }

    pub fn treasury_shares(&self) -> u64 {
        math::treasury_shares(self.initial_shares, DEFAULT_STEP_R_BPS).unwrap()
    }
}

/// The basket right after `seed`.
pub struct SeededBasket {
    pub tight: Pubkey,
    pub tight_lower: i32,
    pub tight_upper: i32,
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
    pub treasury_shares: u64,
    pub sleeve_lamports: u64,
    /// The backstop slice, when the backstop is live and in range.
    pub backstop_x: u64,
    pub backstop_y: u64,
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

    /// The tight range `seed` will open: `[active âˆ’ w, active + w]`.
    pub fn tight_range_at(&self, l: &Launched, active_id: i32) -> (i32, i32) {
        let b: Basket = self.load(&l.basket);
        let w = b.pool.preset.tight_half_width_bins as i32;
        (active_id - w, active_id + w)
    }

    /// Bin arrays (existing or not) covering `[lower, upper]`.
    pub fn bin_array_metas(&self, l: &Launched, lower: i32, upper: i32) -> Vec<AccountMeta> {
        l.pool.bin_array_metas(lower, upper)
    }

    /// Bin arrays of both positions the basket holds (the tail every sleeve
    /// instruction wants): tight, plus backstop while it holds liquidity.
    pub fn pool_tail(&self, l: &Launched) -> Vec<AccountMeta> {
        let b: Basket = self.load(&l.basket);
        let mut metas: Vec<AccountMeta> = Vec::new();
        let mut push_range = |lower: i32, upper: i32| {
            for m in self.existing_bin_array_metas(&l.pool, lower, upper) {
                if !metas.iter().any(|x| x.pubkey == m.pubkey) {
                    metas.push(m);
                }
            }
        };
        if b.tight.is_set() {
            push_range(b.tight.lower_bin_id, b.tight.upper_bin_id);
        }
        if b.backstop.is_set() {
            push_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
        }
        metas
    }

    /// `backstop_position` account for the sleeve instructions.
    pub fn backstop_or_placeholder(&self, l: &Launched) -> Pubkey {
        let b: Basket = self.load(&l.basket);
        if b.backstop.is_set() { b.backstop.key } else { l.placeholder() }
    }

    pub fn seed_ix(&self, l: &Launched, plan: &SeedPlan) -> Instruction {
        self.seed_ix_for(l, plan, &l.book)
    }

    pub fn seed_ix_for(&self, l: &Launched, plan: &SeedPlan, chunk: &[PositionArg]) -> Instruction {
        let payer = l.payer.pubkey();
        let fee_vault = fee_vault_pda(&l.share_mint);
        let active = l.pool.active_id(self);
        let (lower, upper) = self.tight_range_at(l, active);
        let tight = dlmm::basket_position_pda(&l.pool.lb_pair, &l.basket, lower, upper);
        let mut accounts = basket::accounts::Seed {
            payer,
            config: config_pda(),
            basket: l.basket,
            share_mint: l.share_mint,
            share_auth: share_auth_pda(&l.share_mint),
            payer_share_ata: Env::ata(&payer, &l.share_mint),
            fee_vault,
            fee_vault_share_ata: Env::ata(&fee_vault, &l.share_mint),
            treasury: self.treasury.pubkey(),
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
            rent: solana_sdk_ids::sysvar::rent::ID,
        }
        .to_account_metas(None);
        accounts.extend(component_metas(&l.basket, &l.share_mint, &payer, chunk));
        accounts.extend(self.bin_array_metas(l, lower, upper));
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
        self.send_expect_err(&[ix], &l.payer, &[], code);
    }

    /// Funds the components and seeds; panics on failure.
    pub fn seed(&mut self, l: &Launched, plan: &SeedPlan) -> SeededBasket {
        self.fund_components(l, plan);
        let ix = self.seed_ix(l, plan);
        let meta: TransactionMetadata = self.send_ok(&[ix], &l.payer, &[]);
        // DLMM's JIT guard: a position cannot remove from the active bin in
        // the same second it added there. Later transactions land later.
        self.warp(1);
        let b: Basket = self.load(&l.basket);
        SeededBasket {
            tight: b.tight.key,
            tight_lower: b.tight.lower_bin_id,
            tight_upper: b.tight.upper_bin_id,
            cu: meta.compute_units_consumed,
        }
    }

    /// Launch + seed with defaults.
    pub fn launch_and_seed(&mut self, n: usize, nonce: u64) -> (Launched, SeededBasket) {
        let l = self.launch_fixed(n, nonce);
        let plan = SeedPlan::default_for(&l);
        let s = self.seed(&l, &plan);
        (l, s)
    }

    pub fn mint_ix(
        &self,
        l: &Launched,
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
        .to_account_metas(None);
        accounts.extend(component_metas(&l.basket, &l.share_mint, buyer, &l.book));
        accounts.extend(self.pool_tail(l));
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
        buyer: &Keypair,
        deposits: &[u64],
        min_shares_out: u64,
        max_sleeve_lamports: u64,
    ) -> MintResult {
        let ix = self.mint_ix(l, &buyer.pubkey(), deposits, min_shares_out, max_sleeve_lamports);
        let meta = self.send_ok(&[ix], buyer, &[]);
        self.warp(1); // see `seed`
        MintResult { cu: meta.compute_units_consumed, logs: meta.logs }
    }

    /// A funded buyer who mints `deposits` with no slippage bounds.
    pub fn mint_from_new_buyer(&mut self, l: &Launched, deposits: &[u64]) -> (Keypair, MintResult) {
        let buyer = self.fund(100 * LAMPORTS);
        self.fund_components_for(&buyer.pubkey(), l, deposits);
        let r = self.mint(l, &buyer, deposits, 0, u64::MAX);
        (buyer, r)
    }

    // ---- pool reads ----

    pub fn tight_amounts(&self, l: &Launched) -> PositionAmounts {
        let b: Basket = self.load(&l.basket);
        if !b.tight.is_set() {
            return PositionAmounts::default();
        }
        l.pool.position_amounts(self, &b.tight.key)
    }

    /// Backstop amounts while it holds liquidity (FUNDING / LIVE / WITHDRAWING).
    pub fn backstop_amounts(&self, l: &Launched) -> PositionAmounts {
        let b: Basket = self.load(&l.basket);
        let holds = matches!(b.backstop_state, BACKSTOP_FUNDING | BACKSTOP_LIVE | BACKSTOP_WITHDRAWING);
        if !holds || !b.backstop.is_set() {
            return PositionAmounts::default();
        }
        l.pool.position_amounts(self, &b.backstop.key)
    }

    pub fn idle_shares(&self, l: &Launched) -> u64 {
        self.token_amount(&Env::ata(&l.basket, &l.share_mint))
    }

    pub fn idle_sol(&self, l: &Launched) -> u64 {
        self.token_amount(&Env::ata(&l.basket, &WSOL))
    }

    /// Treasury shares in both positions.
    pub fn shares_in_positions(&self, l: &Launched) -> u64 {
        self.tight_amounts(l).amount_x + self.backstop_amounts(l).amount_x
    }

    /// SOL in both positions (excludes idle).
    pub fn sol_in_positions(&self, l: &Launched) -> u64 {
        self.tight_amounts(l).amount_y + self.backstop_amounts(l).amount_y
    }

    /// Holder shares outstanding: supply âˆ’ shares in positions âˆ’ idle shares
    /// (+ shares mid-redemption). The denominator of every pro-rata rule.
    pub fn holder_shares(&self, l: &Launched) -> u64 {
        let b: Basket = self.load(&l.basket);
        self.mint_supply(&l.share_mint) - self.shares_in_positions(l) - self.idle_shares(l) + b.pending_redeem_shares
    }

    /// Off-chain mirror of `mint`: creation-unit shares, step-rule `r`,
    /// treasury shares and the SOL the pool will take at the active bin.
    pub fn mint_quote(&self, l: &Launched, deposits: &[u64]) -> MintQuote {
        // Priced off `vault âˆ’ owed`: amounts owed to frozen claimants do not back holder shares.
        let vaults: Vec<u64> = l
            .mints
            .iter()
            .map(|m| {
                let p: Position = self.load(&position_pda(&l.share_mint, m));
                self.token_amount(&Env::ata(&l.basket, m)) - p.owed
            })
            .collect();
        let h = self.holder_shares(l);
        let gross_shares = math::shares_for_deposits(deposits, &vaults, h).unwrap();
        let b: Basket = self.load(&l.basket);
        let pool_sol = self.sol_in_positions(l) + self.idle_sol(l);
        let r_bps = if pool_sol < b.sleeve.step_threshold_lamports { b.sleeve.step_r_bps } else { b.sleeve.r_bps };
        let y = math::treasury_shares(gross_shares, r_bps).unwrap();
        let active = l.pool.active_id(self);
        let sleeve_lamports = dlmm::lamports_for_x(y, l.pool.price_q64(active)).unwrap();
        let config = self.config();
        let (mut backstop_x, mut backstop_y) = (0, 0);
        if b.backstop_live() && b.backstop.contains(active) {
            let lower = (active - dlmm::DEFAULT_BIN_PER_POSITION / 2).max(b.backstop.lower_bin_id);
            let upper = (lower + dlmm::DEFAULT_BIN_PER_POSITION - 1).min(b.backstop.upper_bin_id);
            let (xb, yb) = basket::instructions::sleeve::side_bins(lower, upper, active);
            if xb > 0 {
                backstop_x = math::bps(y, config.pools.backstop_slice_bps).unwrap();
            }
            if yb > 0 {
                backstop_y = math::bps(sleeve_lamports, config.pools.backstop_slice_bps).unwrap();
            }
        }
        MintQuote { gross_shares, r_bps, treasury_shares: y, sleeve_lamports, backstop_x, backstop_y }
    }

    /// Component vault balances in book order.
    pub fn vaults(&self, l: &Launched) -> Vec<u64> {
        l.mints.iter().map(|m| self.token_amount(&Env::ata(&l.basket, m))).collect()
    }
}
