//! `crystallize` (D12, §4): Managed only, at most once per period. A keeper
//! attests the fund NAV (lamports per whole share). The management fee
//! accrues as share inflation to the creator for the elapsed time; then, if
//! the post-management NAV is above the high-water mark, the performance fee
//! is minted to the creator and the HWM moves to the resulting NAV. Nothing
//! is charged while underwater.

mod common;

use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

const DAY: i64 = 24 * 60 * 60;

fn seeded_managed(env: &mut Env, nonce: u64, perf_bps: u16) -> (Launched, SeededBasket) {
    let l = env.launch_managed(3, nonce, perf_bps);
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    (l, s)
}

#[test]
fn seed_sets_the_high_water_mark_to_the_opening_nav() {
    let mut env = Env::initialized();
    let (l, _s) = seeded_managed(&mut env, 1, 1_000);
    let b: Basket = env.load(&l.basket);
    // 0.3 SOL sleeve at r = 25% implies a 1.2 SOL fund over 1_000 shares.
    let plan = SeedPlan::default_for(&l);
    let fund = plan.sleeve_lamports * BPS_TOTAL as u64 / DEFAULT_STEP_R_BPS as u64;
    assert_eq!(fund, 1_200_000_000);
    assert_eq!(b.hwm_nav_lamports, fund * 1_000_000 / plan.initial_shares);
    assert_eq!(b.hwm_nav_lamports, 1_200_000);
    assert_eq!(b.last_crystallized_ts, env.now());
    assert_eq!(b.last_mgmt_accrual_ts, env.now());
    assert_eq!((b.fees.perf_fee_bps, b.fees.mgmt_fee_bps_per_year), (1_000, DEFAULT_MGMT_FEE_BPS_PER_YEAR));
}

#[test]
fn crystallize_mints_management_and_performance_fees_above_the_hwm() {
    let mut env = Env::initialized();
    let (l, s) = seeded_managed(&mut env, 2, 1_000);
    let creator_ata = Env::ata(&l.creator.pubkey(), &l.share_mint);
    let hwm0 = env.load::<Basket>(&l.basket).hwm_nav_lamports;
    env.warp(31 * DAY);

    // NAV doubled.
    let nav = 2 * hwm0;
    let q = env.crystallize_quote(&l, &s, nav);
    // 1%/yr for 31 days on 1_000 shares ≈ 0.85 shares.
    assert!(q.mgmt_shares > 800_000 && q.mgmt_shares < 900_000, "mgmt {}", q.mgmt_shares);
    // 10% of a 100% gain = 5% of the doubled fund → H·0.05/0.95 ≈ 52.6 shares
    // (dilution-correct: minted at the post-fee price).
    assert!(q.perf_shares > 52_000_000 && q.perf_shares < 54_000_000, "perf {}", q.perf_shares);
    // Creator ends up owning perf/(H+perf) ≈ 10% of the fund's gain in value:
    // their stake × post-fee NAV ≈ 0.1 × (nav − hwm) × H.
    let creator_value = q.perf_shares as u128 * q.hwm_after as u128;
    let owed = (q.nav_after_mgmt - hwm0) as u128 * (q.h_before + q.mgmt_shares) as u128 / 10;
    assert!(creator_value.abs_diff(owed) * 1_000 <= owed, "{creator_value} vs {owed}");

    let m = env.crystallize(&l, &s, nav);
    println!("crystallize CU: {}", m.compute_units_consumed);
    assert_eq!(env.token_amount(&creator_ata), q.mgmt_shares + q.perf_shares);
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.hwm_nav_lamports, q.hwm_after);
    assert!(b.hwm_nav_lamports < nav && b.hwm_nav_lamports > hwm0);
    assert_eq!(b.last_crystallized_ts, env.now());
    assert_eq!(b.last_mgmt_accrual_ts, env.now());
    assert_eq!(env.holder_shares(&l, &s), q.h_before + q.mgmt_shares + q.perf_shares);
}

#[test]
fn underwater_charges_only_the_management_fee() {
    let mut env = Env::initialized();
    let (l, s) = seeded_managed(&mut env, 3, 1_000);
    let creator_ata = Env::ata(&l.creator.pubkey(), &l.share_mint);
    let hwm0 = env.load::<Basket>(&l.basket).hwm_nav_lamports;
    env.warp(45 * DAY);

    let nav = hwm0 / 2;
    let q = env.crystallize_quote(&l, &s, nav);
    assert!(q.mgmt_shares > 0 && q.perf_shares == 0);
    env.crystallize(&l, &s, nav);
    assert_eq!(env.token_amount(&creator_ata), q.mgmt_shares);
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.hwm_nav_lamports, hwm0, "HWM does not move down");

    // Recovering to exactly the HWM is still nothing; above it pays only on
    // the part above.
    env.warp(31 * DAY);
    let q2 = env.crystallize_quote(&l, &s, hwm0);
    assert_eq!(q2.perf_shares, 0);
    env.crystallize(&l, &s, hwm0);
    env.warp(31 * DAY);
    let q3 = env.crystallize_quote(&l, &s, hwm0 + hwm0 / 10);
    assert!(q3.perf_shares > 0);
    env.crystallize(&l, &s, hwm0 + hwm0 / 10);
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.hwm_nav_lamports, q3.hwm_after);
}

#[test]
fn nothing_to_crystallize_when_no_fee_is_due() {
    let mut env = Env::initialized();
    // No management fee for baskets created from here on.
    let mut fees = FeeDefaults::default();
    fees.mgmt_fee_bps_per_year = 0;
    let ix = env.admin_ix(anchor_lang::InstructionData::data(&basket::instruction::UpdateConfig {
        update: ConfigUpdate { fees: Some(fees), ..Default::default() },
    }));
    let admin = env.admin.insecure_clone();
    env.send_ok(&[ix], &admin, &[]);

    let (l, s) = seeded_managed(&mut env, 4, 1_000);
    let hwm0 = env.load::<Basket>(&l.basket).hwm_nav_lamports;
    env.warp(31 * DAY);
    env.crystallize_expect_err(&l, &s, hwm0, err(BasketError::NothingToCrystallize));
    env.crystallize_expect_err(&l, &s, hwm0 - 1, err(BasketError::NothingToCrystallize));
    // A perf fee of zero still moves the HWM with the attested NAV.
    let (l0, s0) = seeded_managed(&mut env, 5, 0);
    env.warp(31 * DAY);
    env.crystallize(&l0, &s0, 3 * hwm0);
    let b: Basket = env.load(&l0.basket);
    assert_eq!(b.hwm_nav_lamports, 3 * hwm0);
    assert_eq!(env.token_amount(&Env::ata(&l0.creator.pubkey(), &l0.share_mint)), 0);
}

#[test]
fn crystallize_is_periodic_keeper_attested_and_managed_only() {
    let mut env = Env::initialized();
    let (l, s) = seeded_managed(&mut env, 6, 1_000);
    let hwm0 = env.load::<Basket>(&l.basket).hwm_nav_lamports;

    // Too soon (period is 30 days).
    env.warp(10 * DAY);
    env.crystallize_expect_err(&l, &s, 2 * hwm0, err(BasketError::CrystallizeTooSoon));
    env.warp(21 * DAY);

    // Only a configured keeper may attest.
    let stranger = env.fund(LAMPORTS);
    let ix = env.crystallize_ix(&l, &s, &stranger.pubkey(), 2 * hwm0);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    // NAV must be positive.
    env.crystallize_expect_err(&l, &s, 0, err(BasketError::ZeroAmount));

    // Fixed baskets have no performance or management fee.
    let lf = env.launch_fixed(2, 7);
    let sf = env.seed(&lf, &SeedPlan::default_for(&lf));
    env.warp(31 * DAY);
    env.crystallize_expect_err(&lf, &sf, 2 * hwm0, err(BasketError::InvalidBasketType));

    // Works right after the period, and then the clock restarts.
    env.crystallize(&l, &s, 2 * hwm0);
    env.crystallize_expect_err(&l, &s, 3 * hwm0, err(BasketError::CrystallizeTooSoon));
}

#[test]
fn creator_share_account_is_created_on_demand() {
    let mut env = Env::initialized();
    let (l, s) = seeded_managed(&mut env, 8, 1_000);
    let creator_ata = Env::ata(&l.creator.pubkey(), &l.share_mint);
    assert!(env.account(&creator_ata).is_none());
    env.warp(31 * DAY);
    let hwm0 = env.load::<Basket>(&l.basket).hwm_nav_lamports;
    env.crystallize(&l, &s, hwm0 * 3 / 2);
    assert!(env.token_amount(&creator_ata) > 0);
}
