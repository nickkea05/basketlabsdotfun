//! `submit_book` (§5, D17, D20): Mirror/Strategy keepers and Managed
//! creators propose a new book. The list lives in `PendingBook`; new mints
//! get their `Position` + vault created right away (so redeem keeps paying
//! everything the basket holds), and for keeper-run types the rebalance
//! window opens immediately. Managed books wait for the timelock and
//! `apply_book`. Fixed books are immutable.

mod common;

use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn seeded_mirror(env: &mut Env, n: usize, nonce: u64) -> (Launched, SeededBasket) {
    let l = env.launch_mirror(n, nonce);
    let s = env.seed(&l, &SeedPlan::default_for(&l));
    (l, s)
}

#[test]
fn keeper_opens_a_rebalance_for_mirror_baskets() {
    let mut env = Env::initialized();
    let (l, _s) = seeded_mirror(&mut env, 3, 1);
    let keeper = env.keeper.insecure_clone();
    // Drop mint 2, add a new whitelisted mint at 40%.
    let new_mint = env.create_mint(6, &env.admin.pubkey());
    env.whitelist(&[new_mint]);
    let target = book(&[l.mints[0], l.mints[1], new_mint], &[3_000, 3_000, 4_000]);

    let before = env.now();
    let m = env.submit_book(&l, &keeper, &target, &[new_mint]);
    println!("submit_book CU: {}", m.compute_units_consumed);

    let b: Basket = env.load(&l.basket);
    assert!(b.rebalance.active);
    assert_eq!(b.rebalance.seq, 1);
    assert_eq!(b.rebalance.target_hash, chain_hash(&target));
    assert_eq!(b.rebalance.target_count, 3);
    assert_eq!(b.rebalance.window_end_ts, before + DEFAULT_REBALANCE_WINDOW_S);
    assert_eq!((b.rebalance.acc_count, b.rebalance.acc_weight), (0, 0));
    assert_eq!(b.book_hash, chain_hash(&l.book), "live book unchanged until finalize");

    let pb: PendingBook = env.load(&pending_book_pda(&l.share_mint));
    assert_eq!(pb.book, target);
    assert_eq!(pb.book_hash, chain_hash(&target));
    assert_eq!(pb.ready_at, before);
    assert_eq!(pb.payer, keeper.pubkey());

    // The new mint has a Position and an empty vault; the leaving one is untouched.
    let p = env.position(&l, &new_mint).expect("new position");
    assert_eq!((p.weight_bps, p.index, p.basket), (4_000, 3, l.basket));
    assert_eq!(env.token_amount(&Env::ata(&l.basket, &new_mint)), 0);
    assert!(env.position(&l, &l.mints[2]).is_some());
    assert_eq!(b.position_count, 4);
}

#[test]
fn managed_creator_submits_a_timelocked_book() {
    let mut env = Env::initialized();
    let l = env.launch_managed(3, 2, 1_000);
    env.seed(&l, &SeedPlan::default_for(&l));
    let creator = l.creator.insecure_clone();
    env.svm.airdrop(&creator.pubkey(), LAMPORTS).unwrap();
    let target = book(&[l.mints[0], l.mints[1], l.mints[2]], &[5_000, 3_000, 2_000]);

    // Keeper may not write a Managed book.
    let keeper = env.keeper.insecure_clone();
    let ix = env.submit_book_ix(&l, &keeper.pubkey(), &target, &[]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::Unauthorized));

    let before = env.now();
    env.submit_book(&l, &creator, &target, &[]);
    let b: Basket = env.load(&l.basket);
    assert!(!b.rebalance.active, "Managed waits for apply_book");
    let pb: PendingBook = env.load(&pending_book_pda(&l.share_mint));
    assert_eq!(pb.ready_at, before + DEFAULT_MANAGED_TIMELOCK_S);
    assert_eq!(pb.book, target);

    // Resubmitting replaces the pending book and restarts the timelock.
    env.warp(3_600);
    let target2 = book(&[l.mints[0], l.mints[1], l.mints[2]], &[4_000, 4_000, 2_000]);
    env.submit_book(&l, &creator, &target2, &[]);
    let pb: PendingBook = env.load(&pending_book_pda(&l.share_mint));
    assert_eq!(pb.book, target2);
    assert_eq!(pb.ready_at, before + 3_600 + DEFAULT_MANAGED_TIMELOCK_S);
}

#[test]
fn fixed_books_are_immutable_and_strangers_are_rejected() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 3);
    env.seed(&l, &SeedPlan::default_for(&l));
    let keeper = env.keeper.insecure_clone();
    let target = book(&[l.mints[0], l.mints[1]], &[6_000, 4_000]);
    let ix = env.submit_book_ix(&l, &keeper.pubkey(), &target, &[]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::ImmutableBasket));

    let (lm, _) = seeded_mirror(&mut env, 2, 4);
    let stranger = env.fund(LAMPORTS);
    let target = book(&[lm.mints[0], lm.mints[1]], &[6_000, 4_000]);
    let ix = env.submit_book_ix(&lm, &stranger.pubkey(), &target, &[]);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    // The creator of a Mirror basket is not its book authority either.
    let creator = lm.creator.insecure_clone();
    env.svm.airdrop(&creator.pubkey(), LAMPORTS).unwrap();
    let ix = env.submit_book_ix(&lm, &creator.pubkey(), &target, &[]);
    env.send_expect_err(&[ix], &creator, &[], err(BasketError::Unauthorized));
}

#[test]
fn book_validation() {
    let mut env = Env::initialized();
    let (l, _) = seeded_mirror(&mut env, 3, 5);
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();

    // Weights must sum to 10_000.
    let bad = book(&[l.mints[0], l.mints[1]], &[6_000, 3_000]);
    env.send_expect_err(&[env.submit_book_ix(&l, &k, &bad, &[])], &keeper, &[], err(BasketError::WeightsMustSumToTotal));
    // No duplicates, no zero weights.
    let bad = book(&[l.mints[0], l.mints[0]], &[5_000, 5_000]);
    env.send_expect_err(&[env.submit_book_ix(&l, &k, &bad, &[])], &keeper, &[], err(BasketError::DuplicateMint));
    let bad = book(&[l.mints[0], l.mints[1], l.mints[2]], &[10_000, 0, 0]);
    env.send_expect_err(&[env.submit_book_ix(&l, &k, &bad, &[])], &keeper, &[], err(BasketError::InvalidArgument));
    // Every mint whitelisted.
    let rogue = env.create_mint(6, &env.admin.pubkey());
    let bad = book(&[l.mints[0], rogue], &[5_000, 5_000]);
    env.send_expect_err(&[env.submit_book_ix(&l, &k, &bad, &[rogue])], &keeper, &[], err(BasketError::MintNotWhitelisted));
    env.whitelist(&[rogue]);
    let target = book(&[l.mints[0], rogue], &[5_000, 5_000]);
    // Paused blocks book changes.
    env.set_paused(true);
    env.send_expect_err(&[env.submit_book_ix(&l, &k, &target, &[rogue])], &keeper, &[], err(BasketError::Paused));
    env.set_paused(false);
    // A new mint's accounts may come with the submission or later through
    // `open_position` (anyone, while the rebalance is active).
    env.submit_book(&l, &keeper, &target, &[]);
    assert!(env.position(&l, &rogue).is_none());
    let anyone = env.fund(LAMPORTS);
    let ix = env.open_position_ix(&l, &anyone.pubkey(), &l.mints[1]);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::OffBook));
    let ix = env.open_position_ix(&l, &anyone.pubkey(), &rogue);
    env.send_ok(&[ix], &anyone, &[]);
    let p = env.position(&l, &rogue).unwrap();
    assert_eq!((p.weight_bps, p.index), (5_000, 3));
    let ix = env.open_position_ix(&l, &anyone.pubkey(), &rogue);
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::DuplicateMint));

    // While a window is open no new target may be submitted; after it
    // closes the keeper may replace the target (rent back to the payer).
    let target2 = book(&[l.mints[0], l.mints[1]], &[5_000, 5_000]);
    env.send_expect_err(&[env.submit_book_ix(&l, &k, &target2, &[])], &keeper, &[], err(BasketError::RebalanceActive));
    env.warp(DEFAULT_REBALANCE_WINDOW_S + 1);
    env.submit_book(&l, &keeper, &target2, &[]);
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.rebalance.seq, 2);
    assert_eq!(b.rebalance.target_hash, chain_hash(&target2));
    // The position created for `rogue` stays until it is closed by the keeper.
    assert!(env.position(&l, &rogue).is_some());
}

#[test]
fn mint_is_blocked_during_a_rebalance_but_redeem_is_not() {
    let mut env = Env::initialized();
    let (l, _s) = seeded_mirror(&mut env, 2, 6);
    let keeper = env.keeper.insecure_clone();
    let target = book(&[l.mints[0], l.mints[1]], &[7_000, 3_000]);
    env.submit_book(&l, &keeper, &target, &[]);

    let buyer = env.fund(10 * LAMPORTS);
    let deposits = vec![100_000_000u64; 2];
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::RebalanceActive));

    let holder = l.payer.insecure_clone();
    let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 4;
    env.redeem(&l, &holder, shares);
}
