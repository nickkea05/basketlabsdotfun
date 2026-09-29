//! `apply_book` (§5): Managed only. Anyone may apply the pending book once
//! its timelock has passed; the weight-based turnover (Σ|Δw|/2 against the
//! current book, which the caller supplies and the program checks against
//! `book_hash`) must fit the per-window cap; the management fee accrues; the
//! rebalance window opens.

mod common;

use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn seeded_managed(env: &mut Env, nonce: u64) -> (Launched, SeededBasket) {
    let l = env.launch_managed(3, nonce, 1_000);
    let s = env.seed(&l, &SeedPlan::default_for(&l));
    let creator = l.creator.pubkey();
    env.svm.airdrop(&creator, LAMPORTS).unwrap();
    (l, s)
}

#[test]
fn applies_after_the_timelock_and_accounts_turnover() {
    let mut env = Env::initialized();
    let (l, s) = seeded_managed(&mut env, 1);
    let creator = l.creator.insecure_clone();
    // Equal thirds → 50/30/20: turnover = (|50−33.33| + |30−33.33| + |20−33.33|)/2 ≈ 16.67%.
    let target = book(&[l.mints[0], l.mints[1], l.mints[2]], &[5_000, 3_000, 2_000]);
    let expected_turnover = turnover_bps(&l.book, &target);
    assert!(expected_turnover >= 1_660 && expected_turnover <= 1_670, "{expected_turnover}");
    env.submit_book(&l, &creator, &target, &[]);

    let anyone = env.fund(LAMPORTS);
    // Too early.
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &l.book);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::TimelockActive));
    env.warp(DEFAULT_MANAGED_TIMELOCK_S);
    // Wrong current book.
    let wrong = book(&[l.mints[0], l.mints[1], l.mints[2]], &[3_400, 3_300, 3_300]);
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &wrong);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::BookHashMismatch));

    let creator_ata = Env::ata(&creator.pubkey(), &l.share_mint);
    let q = env.crystallize_quote(&l, &s, 1); // only the mgmt part is used
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &l.book);
    let m = env.send_ok(&[ix], &anyone, &[]);
    println!("apply_book CU: {}", m.compute_units_consumed);

    let b: Basket = env.load(&l.basket);
    assert!(b.rebalance.active);
    assert_eq!(b.rebalance.seq, 1);
    assert_eq!(b.rebalance.target_hash, chain_hash(&target));
    assert_eq!(b.rebalance.window_end_ts, env.now() + DEFAULT_REBALANCE_WINDOW_S);
    assert_eq!(b.rebalance.turnover_used_bps, expected_turnover);
    assert_eq!(b.rebalance.turnover_window_start_ts, env.now());
    assert_eq!(b.last_mgmt_accrual_ts, env.now());
    assert_eq!(env.token_amount(&creator_ata), q.mgmt_shares, "management fee accrued");
    assert!(q.mgmt_shares > 0);
    // The pending book stays as the rebalance target (finalize needs it).
    let pb: PendingBook = env.load(&pending_book_pda(&l.share_mint));
    assert_eq!(pb.book, target);
}

#[test]
fn turnover_cap_is_enforced_per_window() {
    let mut env = Env::initialized();
    let (l, s) = seeded_managed(&mut env, 2);
    let creator = l.creator.insecure_clone();
    let anyone = env.fund(LAMPORTS);

    // Default cap 30% per 7 days. All-in on one mint = 66.67% turnover.
    let all_in = book(&[l.mints[0], l.mints[1], l.mints[2]], &[9_800, 100, 100]);
    assert!(turnover_bps(&l.book, &all_in) > DEFAULT_TURNOVER_CAP_BPS);
    env.submit_book(&l, &creator, &all_in, &[]);
    env.warp(DEFAULT_MANAGED_TIMELOCK_S);
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &l.book);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::TurnoverExceeded));

    // Two 20% moves in one window: the second breaks the cap.
    let step1 = book(&[l.mints[0], l.mints[1], l.mints[2]], &[5_333, 3_334, 1_333]);
    let t1 = turnover_bps(&l.book, &step1);
    assert!(t1 > 1_900 && t1 <= 2_100, "{t1}");
    env.submit_book(&l, &creator, &step1, &[]);
    env.warp(DEFAULT_MANAGED_TIMELOCK_S);
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &l.book);
    env.send_ok(&[ix], &anyone, &[]);
    // Finish this rebalance (no swaps needed for the test: weights only).
    env.finish_rebalance(&l, &step1, &[]);
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.book_hash, chain_hash(&step1));

    let step2 = book(&[l.mints[0], l.mints[1], l.mints[2]], &[7_333, 1_334, 1_333]);
    let t2 = turnover_bps(&step1, &step2);
    assert!(t1 + t2 > DEFAULT_TURNOVER_CAP_BPS);
    env.submit_book(&l, &creator, &step2, &[]);
    env.warp(DEFAULT_MANAGED_TIMELOCK_S);
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &step1);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::TurnoverExceeded));

    // A new window resets the budget.
    env.warp(DEFAULT_TURNOVER_WINDOW_S);
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &step1);
    env.send_ok(&[ix], &anyone, &[]);
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.rebalance.turnover_used_bps, t2);
}

#[test]
fn apply_is_managed_only_and_respects_pause() {
    let mut env = Env::initialized();
    let (l, s) = seeded_managed(&mut env, 3);
    let creator = l.creator.insecure_clone();
    let anyone = env.fund(LAMPORTS);
    let target = book(&[l.mints[0], l.mints[1], l.mints[2]], &[4_000, 4_000, 2_000]);
    env.submit_book(&l, &creator, &target, &[]);
    env.warp(DEFAULT_MANAGED_TIMELOCK_S);
    env.set_paused(true);
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &l.book);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::Paused));
    env.set_paused(false);

    // Mirror baskets apply at submit; there is nothing pending to apply.
    let lm = env.launch_mirror(2, 4);
    let sm = env.seed(&lm, &SeedPlan::default_for(&lm));
    let ix = env.apply_book_ix(&lm, &sm, &anyone.pubkey(), &lm.book);
    assert!(env.send(&[ix], &anyone, &[]).is_err());

    // While the rebalance runs the target is locked: no resubmission, and
    // applying twice is impossible.
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &l.book);
    env.send_ok(&[ix], &anyone, &[]);
    let ix = env.submit_book_ix(&l, &creator.pubkey(), &target, &[]);
    env.send_expect_err(&[ix], &creator, &[], err(BasketError::RebalanceActive));
    let ix = env.apply_book_ix(&l, &s, &anyone.pubkey(), &l.book);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::RebalanceActive));
}
