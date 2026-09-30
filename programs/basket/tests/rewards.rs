//! `post_rewards_root` / `distribute_rewards` / `close_rewards_root` (D15).
//! The keeper commits an epoch's hold-time-weighted allocation as a Merkle
//! root over `(index, wallet, amount)` leaves, funded from the FeeVault's
//! holder reserve (shares or SOL). Anyone may push a leaf to its wallet;
//! each leaf pays once. A root that is fully paid closes itself; one that
//! is not can be closed by the keeper after a grace period, returning the
//! unpaid remainder to the reserve.

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

/// Seeded Fixed basket with fees swept: holder reserve has shares (from the
/// seed mint fee) and SOL (from a pool trade, claimed).
fn with_reserves(env: &mut Env, nonce: u64) -> (Launched, SeededBasket) {
    let l = env.launch_fixed(2, nonce);
    let s = env.seed(&l, &SeedPlan::default_for(&l));
    let trader = env.fund(10 * LAMPORTS);
    env.buy_shares(&l, &s, &trader, LAMPORTS / 2);
    env.claim_pool_fees(&l, &s, &trader);
    let creator = l.creator.pubkey();
    env.svm.airdrop(&creator, LAMPORTS).unwrap();
    env.sweep_fees(&l, &trader);
    let v = env.fee_vault(&l);
    assert!(v.holder_reserve_shares > 0 && v.holder_reserve_lamports > 0);
    (l, s)
}

#[test]
fn share_rewards_are_posted_and_pushed_leaf_by_leaf() {
    let mut env = Env::initialized();
    let (l, _s) = with_reserves(&mut env, 1);
    let keeper = env.keeper.insecure_clone();
    let reserve = env.fee_vault(&l).holder_reserve_shares;

    let wallets: Vec<Pubkey> = (0..3).map(|_| env.fund(LAMPORTS).pubkey()).collect();
    let rows = vec![(wallets[0], reserve / 2), (wallets[1], reserve / 4), (wallets[2], reserve - reserve / 2 - reserve / 4)];
    let tree = MerkleTree::from_rows(&rows);
    let total: u64 = rows.iter().map(|r| r.1).sum();
    assert_eq!(total, reserve);

    let keeper_before = env.lamports(&keeper.pubkey());
    let ix = env.post_rewards_root_ix(&l, &keeper.pubkey(), 1, tree.root(), l.share_mint, total, 3);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("post_rewards_root CU: {}", m.compute_units_consumed);
    let root_key = rewards_root_pda(&l.share_mint, 1);
    let r: RewardsRoot = env.load(&root_key);
    assert_eq!((r.epoch, r.root, r.reward_mint, r.total_amount, r.distributed_amount, r.leaf_count), (1, tree.root(), l.share_mint, total, 0, 3));
    assert_eq!(r.payer, keeper.pubkey());
    assert_eq!(r.posted_at, env.now());
    assert!(!r.is_claimed(0) && !r.is_claimed(2));
    // Posting moves the amount out of the reserve (it is now committed).
    assert_eq!(env.fee_vault(&l).holder_reserve_shares, 0);

    // Anyone pushes. Leaves 2, 0, 1 in that order.
    let pusher = env.fund(LAMPORTS);
    for i in [2usize, 0, 1] {
        let (w, a) = rows[i];
        let ix = env.distribute_rewards_ix(&l, &pusher.pubkey(), 1, &w, i as u32, a, tree.proof(i));
        let m = env.send_ok(&[ix], &pusher, &[]);
        if i == 2 {
            println!("distribute_rewards CU: {}", m.compute_units_consumed);
        }
        assert_eq!(env.token_amount(&Env::ata(&w, &l.share_mint)), a);
        if i != 1 {
            let r: RewardsRoot = env.load(&root_key);
            assert!(r.is_claimed(i as u32));
        }
    }
    // Fully paid: the root closed and the keeper got the rent back.
    assert!(env.account(&root_key).is_none());
    assert!(env.lamports(&keeper.pubkey()) > keeper_before - 20_000);
    let v = env.fee_vault(&l);
    assert_eq!(v.distributed_shares, total);
    assert_eq!(env.token_amount(&Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint)), 0);
}

#[test]
fn sol_rewards_and_guards() {
    let mut env = Env::initialized();
    let (l, _s) = with_reserves(&mut env, 2);
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    let reserve = env.fee_vault(&l).holder_reserve_lamports;
    let stranger = env.fund(LAMPORTS);

    let w0 = env.fund(LAMPORTS).pubkey();
    let w1 = env.fund(LAMPORTS).pubkey();
    let rows = vec![(w0, reserve / 3), (w1, reserve / 3)];
    let tree = MerkleTree::from_rows(&rows);
    let total = rows[0].1 + rows[1].1;

    // Not a keeper.
    let ix = env.post_rewards_root_ix(&l, &stranger.pubkey(), 7, tree.root(), WSOL, total, 2);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    // More than the reserve.
    let ix = env.post_rewards_root_ix(&l, &k, 7, tree.root(), WSOL, reserve + 1, 2);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BelowMinimum));
    // Unknown reward mint.
    let ix = env.post_rewards_root_ix(&l, &k, 7, tree.root(), l.mints[0], total, 2);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::InvalidArgument));
    // Zero leaves / zero total.
    let ix = env.post_rewards_root_ix(&l, &k, 7, tree.root(), WSOL, total, 0);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::InvalidArgument));
    let ix = env.post_rewards_root_ix(&l, &k, 7, tree.root(), WSOL, 0, 2);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::ZeroAmount));

    let ix = env.post_rewards_root_ix(&l, &k, 7, tree.root(), WSOL, total, 2);
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.fee_vault(&l).holder_reserve_lamports, reserve - total);
    // Same epoch twice.
    let ix = env.post_rewards_root_ix(&l, &k, 7, tree.root(), WSOL, 1, 2);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::ReplayDetected));

    // Wrong amount, wrong proof, wrong index.
    let ix = env.distribute_rewards_ix(&l, &k, 7, &w0, 0, rows[0].1 + 1, tree.proof(0));
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BadProof));
    let ix = env.distribute_rewards_ix(&l, &k, 7, &w0, 0, rows[0].1, tree.proof(1));
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BadProof));
    let ix = env.distribute_rewards_ix(&l, &k, 7, &w0, 5, rows[0].1, tree.proof(0));
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::AlreadyClaimed));

    let before = env.lamports(&w0);
    let ix = env.distribute_rewards_ix(&l, &k, 7, &w0, 0, rows[0].1, tree.proof(0));
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.lamports(&w0) - before, rows[0].1);
    // Paid once.
    let ix = env.distribute_rewards_ix(&l, &k, 7, &w0, 0, rows[0].1, tree.proof(0));
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::AlreadyClaimed));
    let r: RewardsRoot = env.load(&rewards_root_pda(&l.share_mint, 7));
    assert_eq!(r.distributed_amount, rows[0].1);
    assert_eq!(env.fee_vault(&l).distributed_lamports, rows[0].1);

    // A SOL leaf for an empty wallet below rent exemption is refused (the
    // keeper's off-chain batch will skip it; the leaf stays open).
    let ghost = Pubkey::new_unique();
    let rows2 = vec![(ghost, 1_000u64)];
    let tree2 = MerkleTree::from_rows(&rows2);
    let ix = env.post_rewards_root_ix(&l, &k, 8, tree2.root(), WSOL, 1_000, 1);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.distribute_rewards_ix(&l, &k, 8, &ghost, 0, 1_000, tree2.proof(0));
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BelowMinimum));
}

#[test]
fn keeper_closes_a_stale_root_and_the_remainder_returns_to_the_reserve() {
    let mut env = Env::initialized();
    let (l, _s) = with_reserves(&mut env, 3);
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    let reserve = env.fee_vault(&l).holder_reserve_shares;
    let w0 = env.fund(LAMPORTS).pubkey();
    let w1 = env.fund(LAMPORTS).pubkey();
    let rows = vec![(w0, reserve / 2), (w1, reserve / 2)];
    let tree = MerkleTree::from_rows(&rows);
    let ix = env.post_rewards_root_ix(&l, &k, 1, tree.root(), l.share_mint, reserve, 2);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.distribute_rewards_ix(&l, &k, 1, &w0, 0, rows[0].1, tree.proof(0));
    env.send_ok(&[ix], &keeper, &[]);

    // Too early, and not for strangers.
    let ix = env.close_rewards_root_ix(&l, &k, 1);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::TimelockActive));
    env.warp(REWARDS_ROOT_TTL_S);
    let stranger = env.fund(LAMPORTS);
    let ix = env.close_rewards_root_ix(&l, &stranger.pubkey(), 1);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));

    let payer_before = env.lamports(&k);
    let ix = env.close_rewards_root_ix(&l, &k, 1);
    env.send_ok(&[ix], &keeper, &[]);
    assert!(env.account(&rewards_root_pda(&l.share_mint, 1)).is_none());
    assert!(env.lamports(&k) > payer_before);
    let v = env.fee_vault(&l);
    assert_eq!(v.holder_reserve_shares, reserve - rows[0].1, "unpaid half back in the reserve");
    assert_eq!(v.distributed_shares, rows[0].1);
    // Rewards are not affected by the pause flag (not a mint or book change).
    env.set_paused(true);
    let ix = env.post_rewards_root_ix(&l, &k, 2, tree.root(), l.share_mint, reserve - rows[0].1, 2);
    env.send_ok(&[ix], &keeper, &[]);
}
