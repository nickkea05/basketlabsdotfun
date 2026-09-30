//! `lock_creator_shares` / `unlock_creator_shares`: the creator parks
//! shares in a program-owned ATA until a chosen time (leaderboard skin).
//! Topping up extends the lock to the later of the two dates; unlock
//! returns everything and closes the accounts.

mod common;

use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

/// Seeded Fixed basket whose creator holds some shares (the creator line of
/// the seed mint fee, via `sweep_fees`).
fn creator_with_shares(env: &mut Env, nonce: u64) -> (Launched, u64) {
    let l = env.launch_fixed(2, nonce);
    env.seed(&l, &SeedPlan::default_for(&l));
    let creator = l.creator.pubkey();
    env.svm.airdrop(&creator, LAMPORTS).unwrap();
    let caller = env.fund(LAMPORTS);
    env.sweep_fees(&l, &caller);
    let shares = env.token_amount(&Env::ata(&creator, &l.share_mint));
    assert!(shares > 0);
    (l, shares)
}

#[test]
fn lock_tops_up_extends_and_unlocks_after_expiry() {
    let mut env = Env::initialized();
    let (l, shares) = creator_with_shares(&mut env, 1);
    let creator = l.creator.insecure_clone();
    let c = creator.pubkey();
    let lock = creator_lock_pda(&l.share_mint);
    let lock_ata = Env::ata(&lock, &l.share_mint);

    let t0 = env.now();
    let ix = env.lock_creator_shares_ix(&l, &c, shares / 2, 7 * 86_400);
    let m = env.send_ok(&[ix], &creator, &[]);
    println!("lock_creator_shares CU: {}", m.compute_units_consumed);
    let k: CreatorLock = env.load(&lock);
    assert_eq!((k.basket, k.creator, k.amount), (l.basket, c, shares / 2));
    assert_eq!((k.locked_at, k.unlock_at), (t0, t0 + 7 * 86_400));
    assert_eq!(env.token_amount(&lock_ata), shares / 2);
    assert_eq!(env.token_amount(&Env::ata(&c, &l.share_mint)), shares - shares / 2);

    // Top up with a longer lock: amount adds, unlock moves out.
    env.warp(86_400);
    let ix = env.lock_creator_shares_ix(&l, &c, shares - shares / 2, 30 * 86_400);
    env.send_ok(&[ix], &creator, &[]);
    let k: CreatorLock = env.load(&lock);
    assert_eq!(k.amount, shares);
    assert_eq!(k.unlock_at, t0 + 86_400 + 30 * 86_400);
    assert_eq!(k.locked_at, t0, "first lock time kept");
    // A shorter top-up never shortens the lock.
    env.svm.airdrop(&c, LAMPORTS).unwrap();
    let ix = env.lock_creator_shares_ix(&l, &c, 1, MIN_CREATOR_LOCK_S);
    assert!(env.send(&[ix], &creator, &[]).is_err(), "no shares left to lock");

    // Early unlock refused.
    let ix = env.unlock_creator_shares_ix(&l, &c);
    env.send_expect_err(&[ix], &creator, &[], err(BasketError::LockActive));
    env.warp(30 * 86_400 - 1);
    let ix = env.unlock_creator_shares_ix(&l, &c);
    env.send_expect_err(&[ix], &creator, &[], err(BasketError::LockActive));
    env.warp(1);
    let sol_before = env.lamports(&c);
    let ix = env.unlock_creator_shares_ix(&l, &c);
    let m = env.send_ok(&[ix], &creator, &[]);
    println!("unlock_creator_shares CU: {}", m.compute_units_consumed);
    assert_eq!(env.token_amount(&Env::ata(&c, &l.share_mint)), shares);
    assert!(env.account(&lock).is_none() && env.account(&lock_ata).is_none());
    assert!(env.lamports(&c) > sol_before, "rent back");

    // Lock again after an unlock: fresh account.
    let ix = env.lock_creator_shares_ix(&l, &c, shares, MIN_CREATOR_LOCK_S);
    env.send_ok(&[ix], &creator, &[]);
    let k: CreatorLock = env.load(&lock);
    assert_eq!(k.amount, shares);
    assert_eq!(k.locked_at, env.now());
}

#[test]
fn lock_guards() {
    let mut env = Env::initialized();
    let (l, shares) = creator_with_shares(&mut env, 2);
    let creator = l.creator.insecure_clone();
    let c = creator.pubkey();

    let ix = env.lock_creator_shares_ix(&l, &c, 0, MIN_CREATOR_LOCK_S);
    env.send_expect_err(&[ix], &creator, &[], err(BasketError::ZeroAmount));
    let ix = env.lock_creator_shares_ix(&l, &c, shares, MIN_CREATOR_LOCK_S - 1);
    env.send_expect_err(&[ix], &creator, &[], err(BasketError::InvalidArgument));
    let ix = env.lock_creator_shares_ix(&l, &c, shares, MAX_CREATOR_LOCK_S + 1);
    env.send_expect_err(&[ix], &creator, &[], err(BasketError::InvalidArgument));

    // Only the creator. (A stranger with shares of their own included.)
    let stranger = env.fund(LAMPORTS);
    env.create_ata(&stranger.pubkey(), &l.share_mint);
    let ix = env.lock_creator_shares_ix(&l, &stranger.pubkey(), 1, MIN_CREATOR_LOCK_S);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    let ix = env.unlock_creator_shares_ix(&l, &stranger.pubkey());
    assert!(env.send(&[ix], &stranger, &[]).is_err());

    // Unlock without a lock.
    let ix = env.unlock_creator_shares_ix(&l, &c);
    assert!(env.send(&[ix], &creator, &[]).is_err());

    // Paused does not stop the creator from locking or unlocking.
    env.set_paused(true);
    let ix = env.lock_creator_shares_ix(&l, &c, shares, MIN_CREATOR_LOCK_S);
    env.send_ok(&[ix], &creator, &[]);
}
