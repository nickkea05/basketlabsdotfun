//! `seed`: the first buy. Deposits components, mints X to the buyer (less the
//! mint fee, taken as shares into the FeeVault), mints Y = X·r/(1−r) treasury
//! shares, opens the DAMM v2 pool at price = sleeve_SOL / Y, and parks the
//! position NFT with the basket (D4, D5, D8, §4, §5).

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::damm;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

#[test]
fn seeds_opens_pool_and_mints_shares() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(3, 1);
    let plan = SeedPlan::default_for(&l);
    let treasury_before = env.lamports(&env.treasury.pubkey());
    let payer_before = env.lamports(&l.payer.pubkey());

    let s = env.seed(&l, &plan);
    println!("seed(3 positions) CU: {}", s.cu);

    // Components landed in the vaults.
    for (mint, dep) in l.mints.iter().zip(&plan.deposits) {
        assert_eq!(env.token_amount(&Env::ata(&l.basket, mint)), *dep);
    }

    // Shares: X to buyer less 1% fee, fee to the FeeVault, the geometry's
    // share side to the pool (≈2.2 × nominal Y for the [0.5×, 8×] range).
    let x = plan.initial_shares;
    let fee = x / 100;
    let q = plan.quote(DEFAULT_STEP_R_BPS, PriceRange::Bounded);
    let y = x * DEFAULT_STEP_R_BPS as u64 / (BPS_TOTAL - DEFAULT_STEP_R_BPS) as u64;
    assert_eq!(q.nominal_treasury_shares, y);
    assert!(q.treasury_shares > 2 * y && q.treasury_shares < 3 * y, "a={} y={y}", q.treasury_shares);
    assert_eq!(env.token_amount(&Env::ata(&l.payer.pubkey(), &l.share_mint)), x - fee);
    assert_eq!(env.token_amount(&Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint)), fee);
    let supply = env.mint_supply(&l.share_mint);
    assert_eq!(supply, x + q.treasury_shares, "supply = X + pool shares (dust burned)");

    // Pool: token_a = shares, token_b = wSOL, marginal price = sleeve / nominal Y.
    let pool = env.pool_state(&s.pool);
    assert_eq!(pool.token_a_mint, l.share_mint);
    assert_eq!(pool.token_b_mint, NATIVE_MINT);
    assert_eq!(pool.sqrt_price, q.sqrt_price);
    assert_eq!((pool.sqrt_min_price, pool.sqrt_max_price), (q.sqrt_min, q.sqrt_max));
    assert_eq!(pool.collect_fee_mode, damm::COLLECT_FEE_MODE_ONLY_B);
    assert_eq!(pool.liquidity, q.liquidity);
    let a_in_pool = env.token_amount(&pool.token_a_vault);
    let b_in_pool = env.token_amount(&pool.token_b_vault);
    assert_eq!(a_in_pool, q.treasury_shares);
    assert_eq!(b_in_pool, q.sleeve_lamports);
    assert!(b_in_pool <= plan.sleeve_lamports && b_in_pool + 2 >= plan.sleeve_lamports, "b_in_pool {b_in_pool}");
    // NAV check: pool prices the holder shares at exactly the components' implied value.
    let nav_of_holders = basket::math::lamports_for_shares(x, pool.sqrt_price).unwrap();
    let implied_components = plan.sleeve_lamports * (BPS_TOTAL - DEFAULT_STEP_R_BPS) as u64 / DEFAULT_STEP_R_BPS as u64;
    assert!(nav_of_holders.abs_diff(implied_components) <= 2, "{nav_of_holders} vs {implied_components}");
    // Open gate: flat 0.30% base fee, no scheduler.
    assert_eq!(pool.base_fee_cliff_numerator, damm::bps_to_fee_numerator(30));
    assert_eq!(pool.base_fee_periods, 0);

    // Position NFT is owned by the basket.
    assert_eq!(env.token_amount(&damm::position_nft_account(&s.position_nft_mint)), 1);
    let position = env.position_state(&damm::position(&s.position_nft_mint));
    assert_eq!(position.pool, s.pool);
    assert_eq!(position.unlocked_liquidity, pool.liquidity);

    // Basket bookkeeping.
    let b: Basket = env.load(&l.basket);
    assert!(b.seeded);
    assert_eq!(b.pool, s.pool);
    assert_eq!(b.position_nft_mint, s.position_nft_mint);
    assert_eq!(b.pool_position, damm::position(&s.position_nft_mint));
    let fv: FeeVault = env.load(&fee_vault_pda(&l.share_mint));
    assert_eq!(fv.basket, l.basket);

    // Creation fee to the treasury; pool + FeeVault rent came from the buyer.
    assert_eq!(env.lamports(&env.treasury.pubkey()) - treasury_before, DEFAULT_CREATION_FEE_LAMPORTS);
    let spent = payer_before - env.lamports(&l.payer.pubkey());
    let rent_and_fees = spent - plan.sleeve_lamports - DEFAULT_CREATION_FEE_LAMPORTS;
    println!("seed rent + tx fees: {rent_and_fees} lamports");
    assert!(rent_and_fees < 40_000_000, "rent+tx fees {rent_and_fees} lamports (>0.04 SOL)");
    // The buyer's temporary wSOL account was closed again.
    assert!(env.account(&Env::ata(&l.payer.pubkey(), &NATIVE_MINT)).is_none());
}

#[test]
fn windowed_gate_gets_the_launch_fee_scheduler() {
    let mut env = Env::initialized();
    let close_ts = env.now() + 3600;
    let l = env.launch_with(3, 2, |a| {
        a.basket_type = BASKET_TYPE_FIXED;
        a.fixed_kind = FIXED_KIND_MEME;
        a.sleeve_r_bps = 2000;
        a.gate = MintGate::WindowUntil { close_ts };
    });
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    let pool = env.pool_state(&s.pool);
    // Meme profile: full range, 1% end fee, 50% -> 1% linear over 60 x 60 s.
    assert_eq!((pool.sqrt_min_price, pool.sqrt_max_price), (damm::MIN_SQRT_PRICE, damm::MAX_SQRT_PRICE));
    assert_eq!(pool.base_fee_cliff_numerator, damm::bps_to_fee_numerator(DEFAULT_SCHEDULER_CLIFF_BPS as u64));
    assert_eq!(pool.base_fee_periods, DEFAULT_SCHEDULER_PERIODS);
    assert_eq!(pool.base_fee_period_frequency, DEFAULT_SCHEDULER_PERIOD_S);
    let end = damm::bps_to_fee_numerator(100);
    let cliff = damm::bps_to_fee_numerator(DEFAULT_SCHEDULER_CLIFF_BPS as u64);
    assert_eq!(pool.base_fee_reduction_factor, (cliff - end) / DEFAULT_SCHEDULER_PERIODS as u64);
}

#[test]
fn enforces_minimum_and_completeness() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 3);
    let mut plan = SeedPlan::default_for(&l);

    // Implied total = sleeve / r + creation fee must reach 1 SOL.
    plan.sleeve_lamports = 100_000_000; // 0.1 SOL / 25% = 0.4 SOL + 0.1 < 1 SOL
    env.seed_expect_err(&l, &plan, err(BasketError::BelowMinimum));

    // Zero shares or zero deposits are rejected.
    plan.sleeve_lamports = 300_000_000;
    let mut bad = plan.clone();
    bad.initial_shares = 0;
    env.seed_expect_err(&l, &bad, err(BasketError::ZeroAmount));
    let mut bad = plan.clone();
    bad.deposits[1] = 0;
    env.seed_expect_err(&l, &bad, err(BasketError::ZeroAmount));

    // Wrong number of deposits.
    let mut bad = plan.clone();
    bad.deposits.pop();
    env.seed_expect_err(&l, &bad, err(BasketError::ComponentCountMismatch));

    // Incomplete basket (only one of two positions delivered) cannot be seeded.
    let l2 = env.launch_partial(2, 4, 1);
    let mut plan2 = SeedPlan::default_for(&l2);
    plan2.deposits.truncate(1);
    let ix = env.seed_ix_for(&l2, &plan2, &l2.book[..1]);
    env.fund_components(&l2, &plan2);
    env.send_expect_err(&[ix], &l2.payer, &[&plan2.position_nft_mint], err(BasketError::BasketIncomplete));

    // Seeding succeeds, then a second seed fails: the FeeVault `init` trips
    // (system program "already in use") before the handler's AlreadySeeded
    // check can run.
    env.seed(&l, &plan);
    let plan3 = SeedPlan::default_for(&l);
    env.fund_components(&l, &plan3);
    let ix = env.seed_ix(&l, &plan3);
    let r = env.send(&[ix], &l.payer, &[&plan3.position_nft_mint]);
    assert!(r.is_err(), "second seed must fail");
}

#[test]
fn gate_and_pause_apply_to_seed() {
    let mut env = Env::initialized();
    let close_ts = env.now() + 60;
    let l = env.launch_with(2, 5, |a| a.gate = MintGate::WindowUntil { close_ts });
    let plan = SeedPlan::default_for(&l);
    env.warp(120);
    env.seed_expect_err(&l, &plan, err(BasketError::MintGateClosed));

    let l2 = env.launch_fixed(2, 6);
    let plan2 = SeedPlan::default_for(&l2);
    env.set_paused(true);
    env.seed_expect_err(&l2, &plan2, err(BasketError::Paused));
    env.set_paused(false);
    env.seed(&l2, &plan2);
}

/// 26 fixed accounts + 4 per component against the 64-account lock limit
/// (`increase_tx_account_lock_limit` is not active on mainnet): N ≤ 9 in
/// one transaction. Larger books need the split flow (open question for
/// Nick; see README).
#[test]
fn seed_account_counts_and_cu_by_size() {
    let mut env = Env::initialized();
    for (i, n) in [2usize, 5, 8, 9].iter().enumerate() {
        let l = env.launch_fixed(*n, 10 + i as u64);
        let plan = SeedPlan::default_for(&l);
        let ix = env.seed_ix(&l, &plan);
        let accounts = ix.accounts.len();
        let s = env.seed(&l, &plan);
        println!("seed N={n}: {accounts} accounts, {} CU", s.cu);
    }
    let l = env.launch_fixed(10, 20);
    let plan = SeedPlan::default_for(&l);
    env.fund_components(&l, &plan);
    let ix = env.seed_ix(&l, &plan);
    let r = env.send(&[ix], &l.payer, &[&plan.position_nft_mint]);
    assert!(format!("{:?}", r.err().map(|e| e.err)).contains("TooManyAccountLocks"));
}
