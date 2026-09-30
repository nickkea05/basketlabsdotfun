//! `seed`: the first buy. Deposits components, mints X to the buyer (less the
//! mint fee, taken as shares into the FeeVault), mints Y = XÂ·r/(1âˆ’r) treasury
//! shares, places the tight DLMM position around the launch bin with 80 % of
//! both legs and leaves 20 % idle for the keeper's backstop (D4, D5, D8, Â§4,
//! Â§5, change order Â§2, Q15).

mod common;

use basket::constants::*;
use basket::dlmm;
use basket::error::BasketError;
use basket::math;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

#[test]
fn seeds_places_tight_and_mints_shares() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(3, 1);
    let plan = SeedPlan::default_for(&l);
    let treasury_before = env.lamports(&env.treasury.pubkey());
    let payer_before = env.lamports(&l.payer.pubkey());
    let spent_before = l.basket_state(&env).deposit_spent_lamports;

    let s = env.seed(&l, &plan);
    eprintln!("seed(3 positions) CU: {}", s.cu);
    // DLMM's add over 63 bins dominates (~0.9M CU); must stay under the 1.4M cap.
    assert!(s.cu < 1_200_000, "seed CU {}", s.cu);

    // Components landed in the vaults.
    for (mint, dep) in l.mints.iter().zip(&plan.deposits) {
        assert_eq!(env.token_amount(&Env::ata(&l.basket, mint)), *dep);
    }

    // Shares: X to buyer less 1 % fee, fee to the FeeVault, Y treasury shares.
    let x = plan.initial_shares;
    let fee = x / 100;
    let y = plan.treasury_shares();
    assert_eq!(y, x * DEFAULT_STEP_R_BPS as u64 / (BPS_TOTAL - DEFAULT_STEP_R_BPS) as u64);
    assert_eq!(env.token_amount(&Env::ata(&l.payer.pubkey(), &l.share_mint)), x - fee);
    assert_eq!(env.fee_shares(&l), fee);
    assert_eq!(env.mint_supply(&l.share_mint), x + y, "supply = X + treasury shares");
    assert_eq!(env.holder_shares(&l), x);

    // Tight: [launch âˆ’ w, launch + w], owned by the basket, 80 % of both legs.
    let b: Basket = env.load(&l.basket);
    let w = b.pool.preset.tight_half_width_bins as i32;
    assert_eq!((s.tight_lower, s.tight_upper), (LAUNCH_ACTIVE_ID - w, LAUNCH_ACTIVE_ID + w));
    assert_eq!(s.tight, dlmm::basket_position_pda(&l.pool.lb_pair, &l.basket, s.tight_lower, s.tight_upper));
    let p = load_position(&env, &s.tight);
    assert_eq!(p.owner, l.basket);
    assert_eq!((p.lower_bin_id, p.upper_bin_id), (s.tight_lower, s.tight_upper));
    let slice = env.config().pools.backstop_slice_bps;
    let x_tight = y - math::bps(y, slice).unwrap();
    let y_tight = plan.sleeve_lamports - math::bps(plan.sleeve_lamports, slice).unwrap();
    let t = env.tight_amounts(&l);
    assert!(t.amount_x.abs_diff(x_tight) <= 70, "tight X {} vs {x_tight}", t.amount_x);
    assert!(t.amount_y.abs_diff(y_tight) <= 70, "tight Y {} vs {y_tight}", t.amount_y);
    // DLMM rounds each bin's deposit down: a few base units short per leg.
    let (rx, ry) = l.pool.reserves(&env);
    assert!(rx <= x_tight && x_tight - rx <= 70, "pool X {rx} vs {x_tight}");
    assert!(ry <= y_tight && y_tight - ry <= 70, "pool Y {ry} vs {y_tight}");
    // Asks above the active bin, bids below (Spot around the launch bin).
    let above = l.pool.bin_amounts(&env, LAUNCH_ACTIVE_ID + 1, s.tight_upper);
    let below = l.pool.bin_amounts(&env, s.tight_lower, LAUNCH_ACTIVE_ID - 1);
    assert!(above.0 > 0 && above.1 == 0, "ask bin {above:?}");
    assert!(below.0 == 0 && below.1 > 0, "bid bin {below:?}");
    assert_eq!(l.pool.active_id(&env), LAUNCH_ACTIVE_ID, "adding liquidity does not move the price");

    // 20 % of both legs (plus the rounding) idle for the keeper's backstop.
    assert_eq!(env.idle_shares(&l), y - rx);
    assert_eq!(env.idle_sol(&l), plan.sleeve_lamports - ry);

    // Basket bookkeeping.
    assert!(b.seeded);
    assert_eq!(b.tight.key, s.tight);
    assert_eq!((b.tight.lower_bin_id, b.tight.upper_bin_id), (s.tight_lower, s.tight_upper));
    assert!(!b.backstop.is_set());
    assert_eq!(b.backstop_state, BACKSTOP_UNPLACED);
    assert_eq!(b.last_recenter_ts, env.now());
    let opening_value = math::implied_total_lamports(plan.sleeve_lamports, DEFAULT_STEP_R_BPS).unwrap();
    assert_eq!(b.hwm_nav_lamports, math::nav_per_share(opening_value, x).unwrap());
    let fv = env.fee_vault(&l);
    assert_eq!(fv.basket, l.basket);
    assert_eq!(fv.creator, l.creator.pubkey());
    assert_eq!(fv.fees, b.fees);
    assert_eq!(fv.unsettled_shares, 0);

    // Creation fee to the treasury; bin-array rent out of the deposit;
    // position + FeeVault + ATA rent from the buyer (refundable at close).
    assert_eq!(env.lamports(&env.treasury.pubkey()) - treasury_before, DEFAULT_CREATION_FEE_LAMPORTS);
    let array_rent = b.deposit_spent_lamports - spent_before;
    assert!(array_rent > 0, "tight bin arrays are paid from the deposit");
    let spent = payer_before - env.lamports(&l.payer.pubkey());
    let rent_and_fees = spent - plan.sleeve_lamports - DEFAULT_CREATION_FEE_LAMPORTS;
    eprintln!("seed: bin array rent {array_rent} (deposit), buyer rent + tx fees {rent_and_fees} lamports");
    // Tight position (~8 KB, refundable at close) + FeeVault + 4 ATAs.
    assert!(rent_and_fees < 80_000_000, "buyer rent {rent_and_fees} lamports (>0.08 SOL)");
    let position_rent = env.lamports(&s.tight);
    eprintln!("tight position rent {position_rent} lamports");
    assert!(rent_and_fees - position_rent < 10_000_000, "non-position buyer rent {}", rent_and_fees - position_rent);
}

#[test]
fn sleeve_must_agree_with_the_launch_bin() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 1);
    let plan = SeedPlan::default_for(&l);
    let y = plan.treasury_shares();
    let low = dlmm::lamports_for_x(y, l.pool.price_q64(LAUNCH_ACTIVE_ID - 1)).unwrap();
    let high = dlmm::lamports_for_x(y, l.pool.price_q64(LAUNCH_ACTIVE_ID + 1)).unwrap();
    assert!(low < plan.sleeve_lamports && plan.sleeve_lamports < high);

    // One bin step either way is the tolerance.
    let mut bad = plan.clone();
    bad.sleeve_lamports = low - 1;
    env.seed_expect_err(&l, &bad, err(BasketError::LaunchPriceMismatch));
    bad.sleeve_lamports = high + 1;
    env.seed_expect_err(&l, &bad, err(BasketError::LaunchPriceMismatch));
    bad.sleeve_lamports = high;
    env.seed(&l, &bad);

    // A basket launched at a higher bin wants proportionally more SOL.
    let l2 = env.launch_with(2, 2, |a| a.launch_active_id = 400);
    assert_eq!(l2.pool.active_id(&env), 400);
    let plan2 = SeedPlan::default_for(&l2);
    assert!(plan2.sleeve_lamports > plan.sleeve_lamports * 2, "{} vs {}", plan2.sleeve_lamports, plan.sleeve_lamports);
    let mut bad2 = plan2.clone();
    bad2.sleeve_lamports = plan.sleeve_lamports;
    env.seed_expect_err(&l2, &bad2, err(BasketError::LaunchPriceMismatch));
    let s2 = env.seed(&l2, &plan2);
    let w = l2.basket_state(&env).pool.preset.tight_half_width_bins as i32;
    assert_eq!((s2.tight_lower, s2.tight_upper), (400 - w, 400 + w));
}

#[test]
fn enforces_minimum_and_completeness() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 3);
    let plan = SeedPlan::default_for(&l);

    // Implied total = sleeve / r + creation fee must reach 1 SOL: a small
    // buy at the launch bin (fewer shares) is below the minimum.
    let mut small = plan.clone();
    small.initial_shares = plan.initial_shares / 4;
    small.sleeve_lamports = plan.sleeve_lamports / 4; // 0.083 SOL / 25 % + 0.1 < 1 SOL
    env.seed_expect_err(&l, &small, err(BasketError::BelowMinimum));

    // Zero shares or zero deposits are rejected.
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
    env.send_expect_err(&[ix], &l2.payer, &[], err(BasketError::BasketIncomplete));

    // Seeding succeeds, then a second seed fails: the FeeVault `init` trips
    // (system program "already in use") before the handler's AlreadySeeded
    // check can run.
    env.seed(&l, &plan);
    env.fund_components(&l, &plan);
    let ix = env.seed_ix(&l, &plan);
    assert!(env.send(&[ix], &l.payer, &[]).is_err(), "second seed must fail");
}

#[test]
fn gate_and_pause_apply_to_seed() {
    let mut env = Env::initialized();
    let close_ts = env.now() + DEFAULT_MIN_MINT_WINDOW_S;
    let l = env.launch_with(2, 5, |a| a.gate = MintGate::WindowUntil { close_ts });
    let plan = SeedPlan::default_for(&l);
    env.warp(DEFAULT_MIN_MINT_WINDOW_S + 1);
    env.seed_expect_err(&l, &plan, err(BasketError::MintGateClosed));

    // A Closed gate with a schedule: seed only inside a window.
    let anchor_ts = env.now() + 3_600;
    let l3 = env.launch_with(2, 7, |a| {
        a.gate = MintGate::Closed;
        a.schedule = Some(ReopenSchedule { anchor_ts, period_s: 7 * 86_400, open_s: 86_400 });
    });
    let plan3 = SeedPlan::default_for(&l3);
    env.seed_expect_err(&l3, &plan3, err(BasketError::MintGateClosed));
    env.warp(3_600);
    env.seed(&l3, &plan3);

    let l2 = env.launch_fixed(2, 6);
    let plan2 = SeedPlan::default_for(&l2);
    env.set_paused(true);
    env.seed_expect_err(&l2, &plan2, err(BasketError::Paused));
    env.set_paused(false);
    env.seed(&l2, &plan2);
}

/// 23 fixed accounts + 4 per component + the tight range's bin arrays
/// against the 64-account lock limit: N â‰¤ 9 in one transaction (larger
/// books need the split flow; see README).
#[test]
fn seed_account_counts_and_cu_by_size() {
    let mut env = Env::initialized();
    for (i, n) in [2usize, 5, 8, 9].iter().enumerate() {
        let l = env.launch_fixed(*n, 10 + i as u64);
        let plan = SeedPlan::default_for(&l);
        let ix = env.seed_ix(&l, &plan);
        let accounts = ix.accounts.len();
        assert!(accounts <= 64, "N={n}: {accounts} accounts");
        let s = env.seed(&l, &plan);
        eprintln!("seed N={n}: {accounts} accounts, {} CU", s.cu);
    }
    let l = env.launch_fixed(10, 20);
    let plan = SeedPlan::default_for(&l);
    env.fund_components(&l, &plan);
    let ix = env.seed_ix(&l, &plan);
    let r = env.send(&[ix], &l.payer, &[]);
    assert!(format!("{:?}", r.err().map(|e| e.err)).contains("TooManyAccountLocks"));
}
