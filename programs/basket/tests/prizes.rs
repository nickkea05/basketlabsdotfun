//! PRIZE vault and `post_prize_payout` (change order §5, Q9). Ranking is
//! off-chain; the program checks what it can: the epoch has ended and was
//! not paid yet, the total fits the vault, the recipient count fits Config,
//! each recipient is the creator of a distinct basket with a live
//! CreatorLock, and that basket claimed at least `min_epoch_pool_fees` of
//! pool fees (SOL) during the epoch.

mod common;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_instruction;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer as _;

const DAY: i64 = 24 * 60 * 60;

/// A seeded basket whose creator locked shares for `lock_days` (0 = no lock)
/// and whose pool saw `round_trips` × 0.1 SOL of trading, fees claimed.
fn contender(env: &mut Env, nonce: u64, lock_days: i64, round_trips: usize) -> Launched {
    let (l, _) = env.launch_and_seed(2, nonce);
    if lock_days > 0 {
        let creator = l.creator.insecure_clone();
        let ix = env.lock_creator_shares_ix(&l, &creator.pubkey(), 1_000_000, lock_days * DAY);
        env.send_ok(&[ix], &creator, &[]);
    }
    for _ in 0..round_trips {
        let (trader, got) = env.trader_who_bought(&l, LAMPORTS / 10);
        env.sell_shares(&l, &trader, got);
    }
    if round_trips > 0 {
        let caller = env.fund(LAMPORTS);
        let (routed, _) = env.claim_tight_fees(&l, &caller);
        assert!(routed > 0);
    }
    l
}

fn fund_prizes(env: &mut Env, lamports: u64) {
    let admin = env.admin.insecure_clone();
    let ix = system_instruction::transfer(&admin.pubkey(), &prize_vault_pda(), lamports);
    env.send_ok(&[ix], &admin, &[]);
}

/// Warp to the first second of the next prize epoch.
fn next_epoch(env: &mut Env) -> u64 {
    let p = env.config().prizes;
    let now = env.now();
    let e = p.epoch_at(now);
    let boundary = p.epoch_anchor_ts + (e as i64 + 1) * p.epoch_s;
    env.warp(boundary - now + 1);
    assert_eq!(env.config().prizes.epoch_at(env.now()), e + 1);
    e
}

fn setup(env: &mut Env) {
    // 0.1 SOL of claimed pool fees per epoch is the mainnet bar; a 0.1 SOL
    // round trip pays ~0.0008 SOL here, so lower it for the test.
    let mut prizes = env.config().prizes;
    assert_eq!(prizes.min_epoch_pool_fees, LAMPORTS / 10);
    assert_eq!(prizes.epoch_s, 14 * DAY);
    assert_eq!(prizes.max_recipients, 3);
    assert_eq!(&prizes.split_bps[..3], &[5_000, 3_000, 2_000]);
    prizes.min_epoch_pool_fees = 100_000;
    env.update_config(ConfigUpdate { prizes: Some(prizes), ..Default::default() });
    fund_prizes(env, LAMPORTS);
}

#[test]
fn pays_one_epoch_by_rank_split_to_eligible_creators() {
    let mut env = Env::initialized();
    setup(&mut env);
    let keeper: Keypair = env.keeper.insecure_clone();
    let first = contender(&mut env, 1, 30, 2);
    let second = contender(&mut env, 2, 30, 1);
    let b1 = first.basket_state(&env);
    let epoch = env.config().prizes.epoch_at(env.now());
    assert_eq!(b1.fee_epoch, epoch);
    assert!(b1.fee_epoch_lamports >= 100_000, "fees {}", b1.fee_epoch_lamports);
    // The fee path also credits the vault: 5 % of what was routed.
    let vault_free = env.free_lamports(&prize_vault_pda());
    assert!(vault_free > LAMPORTS, "{vault_free}");
    assert_eq!(env.prize_vault().received_lamports, vault_free - LAMPORTS);

    // Not over yet.
    let recipients = [(first.creator.pubkey(), first.share_mint), (second.creator.pubkey(), second.share_mint)];
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 2, &recipients);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PrizeEpoch));

    assert_eq!(next_epoch(&mut env), epoch);
    let w1 = env.lamports(&first.creator.pubkey());
    let w2 = env.lamports(&second.creator.pubkey());
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 2, &recipients);
    let m = env.send_ok(&[ix], &keeper, &[]);
    eprintln!("post_prize_payout(2) CU: {}", m.compute_units_consumed);
    assert_eq!(env.lamports(&first.creator.pubkey()) - w1, LAMPORTS / 4, "rank 1: 50 %");
    assert_eq!(env.lamports(&second.creator.pubkey()) - w2, LAMPORTS * 3 / 20, "rank 2: 30 %");
    let v = env.prize_vault();
    assert_eq!(v.paid_lamports, LAMPORTS / 4 + LAMPORTS * 3 / 20, "the third slot's 20 % stays in the vault");
    assert_eq!((v.last_paid_epoch, v.paid_any), (epoch, true));
    assert_eq!(env.free_lamports(&prize_vault_pda()), vault_free - v.paid_lamports);

    // The same epoch cannot be paid twice; the next one can, once it ends.
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 10, &recipients);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PrizeEpoch));
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch + 1, LAMPORTS / 10, &recipients);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PrizeEpoch));
    // Epoch + 1: trade again so the fee window shows it, then pay.
    let (trader, got) = env.trader_who_bought(&first, LAMPORTS / 10);
    env.sell_shares(&first, &trader, got);
    let caller = env.fund(LAMPORTS);
    env.claim_tight_fees(&first, &caller);
    assert_eq!(first.basket_state(&env).epoch_pool_fees(epoch), b1.fee_epoch_lamports, "previous epoch kept");
    next_epoch(&mut env);
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch + 1, LAMPORTS / 10, &recipients[..1]);
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.prize_vault().last_paid_epoch, epoch + 1);
}

#[test]
fn eligibility_lock_fees_distinct_baskets_and_wallets() {
    let mut env = Env::initialized();
    setup(&mut env);
    let keeper = env.keeper.insecure_clone();
    let eligible = contender(&mut env, 1, 30, 1);
    let no_lock = contender(&mut env, 2, 0, 1);
    let no_fees = contender(&mut env, 3, 30, 0);
    let short_lock = contender(&mut env, 4, 1, 1); // expires before the epoch ends
    let epoch = next_epoch(&mut env);
    let ok = (eligible.creator.pubkey(), eligible.share_mint);

    for bad in [
        (no_lock.creator.pubkey(), no_lock.share_mint),
        (no_fees.creator.pubkey(), no_fees.share_mint),
        (short_lock.creator.pubkey(), short_lock.share_mint),
        // Wrong wallet for the basket.
        (no_lock.creator.pubkey(), eligible.share_mint),
    ] {
        let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 10, &[ok, bad]);
        env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PrizeIneligible));
    }
    // The same basket twice.
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 10, &[ok, ok]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PrizeIneligible));
    // Zero recipients, or more than Config allows.
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 10, &[]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::InvalidArgument));
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 10, &[ok, ok, ok, ok]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::InvalidArgument));
    // More than the vault holds, or nothing.
    let free = env.free_lamports(&prize_vault_pda());
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, free + 1, &[ok]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::MathOverflow));
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, 0, &[ok]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::MathOverflow));
    // Keeper only; pause.
    let stranger = env.fund(LAMPORTS);
    let ix = env.post_prize_payout_ix(&stranger.pubkey(), epoch, LAMPORTS / 10, &[ok]);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    env.set_paused(true);
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 10, &[ok]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::Paused));
    env.set_paused(false);

    // The eligible one alone is paid its 50 %; the rest stays.
    let w = env.lamports(&eligible.creator.pubkey());
    let ix = env.post_prize_payout_ix(&keeper.pubkey(), epoch, LAMPORTS / 10, &[ok]);
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.lamports(&eligible.creator.pubkey()) - w, LAMPORTS / 20);

    // The basket keeps its last two claim epochs; a claim two epochs later
    // rolls the old one out of the window (paid epochs only ever move
    // forward, so a stale record cannot be paid twice either way).
    next_epoch(&mut env);
    next_epoch(&mut env);
    assert_eq!(eligible.basket_state(&env).epoch_pool_fees(epoch), eligible.basket_state(&env).fee_epoch_lamports);
    let (trader, got) = env.trader_who_bought(&eligible, LAMPORTS / 10);
    env.sell_shares(&eligible, &trader, got);
    let caller = env.fund(LAMPORTS);
    env.claim_tight_fees(&eligible, &caller);
    let b = eligible.basket_state(&env);
    assert_eq!(b.fee_epoch, epoch + 3);
    assert_eq!(b.epoch_pool_fees(epoch), 0);
    assert_eq!(b.epoch_pool_fees(epoch + 2), 0, "nothing was claimed in the skipped epochs");
}

#[test]
fn the_fee_path_feeds_the_prize_vault() {
    let mut env = Env::initialized();
    let before = env.free_lamports(&prize_vault_pda());
    let (l, _) = env.launch_and_seed(2, 1);
    let (trader, got) = env.trader_who_bought(&l, LAMPORTS / 10);
    env.sell_shares(&l, &trader, got);
    let caller = env.fund(LAMPORTS);
    let (routed, _) = env.claim_tight_fees(&l, &caller);
    let fees = l.basket_state(&env).fees;
    let rest = routed - routed * fees.creator_split_bps as u64 / BPS_TOTAL as u64;
    let others = (fees.buyback_split_bps + fees.team_split_bps + fees.prize_split_bps) as u64;
    let prize_line = rest - rest * fees.buyback_split_bps as u64 / others - rest * fees.team_split_bps as u64 / others;
    assert_eq!(env.free_lamports(&prize_vault_pda()) - before, prize_line);
    assert_eq!(env.prize_vault().received_lamports, prize_line);
}
