//! `claim_pool_fees`: anyone cranks one of the basket's DLMM positions over
//! a bin range. Pool swap fees accrue in SOL (`collect_fee_mode = OnlyY`)
//! and are routed at once â€” creator (tier) / BUYBACK / team / PRIZE â€” along
//! with anything the keeper parked on the FeeVault; the claimed total is
//! recorded per prize epoch (change order Â§2.5, Q1, Q9).

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::dlmm;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

struct Lines {
    creator: u64,
    buyback: u64,
    team: u64,
    prize: u64,
}

fn lines(env: &Env, l: &Launched) -> Lines {
    let config = env.config();
    Lines {
        creator: env.lamports(&l.creator.pubkey()) + env.fee_vault(l).creator_owed_lamports,
        buyback: env.lamports(&buyback_vault_pda()),
        team: env.lamports(&config.team_wallet),
        prize: env.lamports(&prize_vault_pda()),
    }
}

/// LP fee (80 % of the pool's base fee; Meteora keeps 20 %) on `amount`.
fn lp_fee(amount: u64, base_fee_bps: u16) -> u64 {
    amount * base_fee_bps as u64 / BPS_TOTAL as u64 * (BPS_TOTAL - dlmm::ILM_PROTOCOL_SHARE) as u64 / BPS_TOTAL as u64
}

#[test]
fn claims_swap_fees_as_sol_and_routes_four_ways() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(3, 1);
    let b = l.basket_state(&env);
    let fee_vault = fee_vault_pda(&l.share_mint);
    assert_eq!(env.fee_vault_free_lamports(&l), 0, "FeeVault starts with rent only");
    let fee_shares_before = env.fee_shares(&l);
    let idle_x_before = env.idle_shares(&l);

    // A round trip through the pool: 1 SOL in, shares back out.
    let trader = env.fund(10 * LAMPORTS);
    let bought = env.buy_shares(&l, &trader, LAMPORTS / 10);
    assert!(bought > 0);
    let sol_back = env.sell_shares(&l, &trader, bought);
    // Fees are taken in SOL on both legs: the SOL in on the buy, the SOL
    // out on the sell (OnlyY). The dynamic fee may add to the base fee.
    let base = lp_fee(LAMPORTS / 10, b.pool.preset.base_fee_bps) + lp_fee(sol_back, b.pool.preset.base_fee_bps);

    let caller = env.fund(LAMPORTS);
    let caller_before = env.lamports(&caller.pubkey());
    let before = lines(&env, &l);
    let (routed, cu) = env.claim_tight_fees(&l, &caller);
    eprintln!("claim_pool_fees (tight, 63 bins) CU: {cu}; routed {routed} lamports (base-fee estimate {base})");
    assert!(routed >= base * 95 / 100 && routed <= base * 3, "routed {routed} vs base {base}");
    let after = lines(&env, &l);
    let fees = b.fees;
    let creator = after.creator - before.creator;
    let buyback = after.buyback - before.buyback;
    let team = after.team - before.team;
    let prize = after.prize - before.prize;
    assert_eq!(creator + buyback + team + prize, routed);
    assert_eq!(creator, routed * fees.creator_split_bps as u64 / BPS_TOTAL as u64, "creator tier line");
    let rest = routed - creator;
    let others = (fees.buyback_split_bps + fees.team_split_bps + fees.prize_split_bps) as u64;
    assert_eq!(buyback, rest * fees.buyback_split_bps as u64 / others);
    assert_eq!(team, rest * fees.team_split_bps as u64 / others);
    assert_eq!(prize, rest - buyback - team);
    assert_eq!((fees.creator_split_bps, fees.buyback_split_bps, fees.team_split_bps, fees.prize_split_bps), (2_000, 5_000, 2_500, 500));
    // Nothing stays on the FeeVault; the caller only paid the tx fee.
    assert_eq!(env.fee_vault_free_lamports(&l), 0);
    let spent = caller_before - env.lamports(&caller.pubkey());
    assert!(spent < 20_000, "caller spent {spent}");
    assert!(env.account(&Env::ata(&fee_vault, &WSOL)).is_none(), "no wSOL account left behind");
    // Nothing on the share side.
    assert_eq!(env.fee_shares(&l), fee_shares_before);
    assert_eq!(env.idle_shares(&l), idle_x_before);
    // Counters.
    let fv = env.fee_vault(&l);
    assert_eq!((fv.swept_creator_lamports, fv.routed_buyback_lamports, fv.routed_team_lamports, fv.routed_prize_lamports), (creator, buyback, team, prize));
    assert_eq!(fv.unrouted_lamports, 0);
    assert_eq!(env.buyback_vault().received_lamports, buyback);
    assert_eq!(env.prize_vault().received_lamports, prize);
    let b = l.basket_state(&env);
    assert_eq!(b.fee_epoch_lamports, routed);
    assert_eq!(b.fee_epoch, env.config().prizes.epoch_at(env.now()));

    // No new trades: claiming again routes nothing and is not an error.
    let (again, _) = env.claim_tight_fees(&l, &caller);
    assert_eq!(again, 0);
}

#[test]
fn claim_covers_the_backstop_one_array_at_a_time() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 2);
    env.place_and_fund_backstop(&l);
    let b = l.basket_state(&env);
    let (lo, hi) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
    // Trade through the backstop's bins above tight and back.
    let buyer = env.fund(100 * LAMPORTS);
    env.buy_up_to(&l, &buyer, b.tight.upper_bin_id + 15);
    env.sell_down_to(&l, &buyer, b.tight.upper_bin_id - 5);
    let caller = env.fund(LAMPORTS);

    // The tight position earned in its binsâ€¦
    let (tight_routed, _) = env.claim_tight_fees(&l, &caller);
    assert!(tight_routed > 0);
    // â€¦and the backstop in the bins the price crossed above tight.
    let mut per_array = vec![];
    for idx in lo..=hi {
        let lower = dlmm::array_lower_bin(idx).max(b.backstop.lower_bin_id);
        let upper = dlmm::array_upper_bin(idx).min(b.backstop.upper_bin_id);
        let ix = env.claim_pool_fees_ix(&l, &caller.pubkey(), &b.backstop.key, lower, upper);
        let before = env.routed_total(&l);
        let m = env.send_ok(&[ix], &caller, &[]);
        per_array.push((env.routed_total(&l) - before, m.compute_units_consumed));
    }
    eprintln!("claim backstop per array (routed, CU): {per_array:?}");
    let total: u64 = per_array.iter().map(|(r, _)| r).sum();
    assert!(total > 0, "the backstop earned fees where the price crossed it");
    let array_of = |bin: i32| dlmm::bin_array_index(bin) - lo;
    assert!(per_array[array_of(b.tight.upper_bin_id + 10) as usize].0 > 0);
    // A range outside the position, or a foreign position, is refused.
    let ix = env.claim_pool_fees_ix(&l, &caller.pubkey(), &b.backstop.key, b.backstop.lower_bin_id - 1, b.backstop.lower_bin_id);
    env.send_expect_err(&[ix], &caller, &[], err(BasketError::PositionMismatch));
    let ix = env.claim_pool_fees_ix(&l, &caller.pubkey(), &l.placeholder(), 0, 1);
    env.send_expect_err(&[ix], &caller, &[], err(BasketError::PositionMismatch));
    // Missing bin arrays.
    let mut ix = env.claim_pool_fees_ix(&l, &caller.pubkey(), &b.tight.key, b.tight.lower_bin_id, b.tight.upper_bin_id);
    ix.accounts.truncate(ix.accounts.len() - 2);
    env.send_expect_err(&[ix], &caller, &[], err(BasketError::BinArrayMismatch));
}

#[test]
fn claim_routes_what_the_keeper_parked() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 3);
    env.place_and_fund_backstop(&l);
    let b = l.basket_state(&env);
    // Trades inside tight, then a run-up (into the backstop's asks) that
    // forces a re-centre: the keeper's claim on the old position is parked
    // on the FeeVault, not routed.
    let trader = env.fund(100 * LAMPORTS);
    let got = env.buy_shares(&l, &trader, LAMPORTS / 10);
    env.sell_shares(&l, &trader, got);
    env.warp(env.config().pools.recenter_min_interval_s);
    env.buy_up_to(&l, &trader, b.tight.upper_bin_id + 3);
    env.warp(1);
    env.recenter_tight_ok(&l);
    let fv = env.fee_vault(&l);
    assert!(fv.unrouted_lamports > 0, "re-centre parked the old position's fees");
    assert_eq!(env.fee_vault_free_lamports(&l), fv.unrouted_lamports);
    let b = l.basket_state(&env);
    assert_eq!(b.fee_epoch_lamports, fv.unrouted_lamports, "parked fees already count for the epoch");

    // The next claim (nothing new to claim on the fresh position) routes them.
    let caller = env.fund(LAMPORTS);
    let (routed, _) = env.claim_tight_fees(&l, &caller);
    assert_eq!(routed, fv.unrouted_lamports);
    assert_eq!(env.fee_vault(&l).unrouted_lamports, 0);
    assert_eq!(env.fee_vault_free_lamports(&l), 0);
    assert_eq!(l.basket_state(&env).fee_epoch_lamports, fv.unrouted_lamports, "not counted twice");
}

#[test]
fn creator_line_waits_until_the_wallet_can_hold_it() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 4);
    // Empty the creator's wallet: a payout below rent-exemption would be
    // rejected by the runtime, so it accrues on the FeeVault instead.
    let creator = l.creator.insecure_clone();
    let sink = env.fund(LAMPORTS);
    let balance = env.lamports(&creator.pubkey());
    let drain = anchor_lang::solana_program::system_instruction::transfer(&creator.pubkey(), &sink.pubkey(), balance - 5_000);
    env.send_ok(&[drain], &creator, &[]);
    assert_eq!(env.lamports(&creator.pubkey()), 0);

    let trader = env.fund(10 * LAMPORTS);
    let got = env.buy_shares(&l, &trader, LAMPORTS / 10);
    env.sell_shares(&l, &trader, got);
    let caller = env.fund(LAMPORTS);
    let (routed, _) = env.claim_tight_fees(&l, &caller);
    let fv = env.fee_vault(&l);
    assert!(routed > 0 && fv.creator_owed_lamports == routed * 2_000 / 10_000, "creator line owed: {}", fv.creator_owed_lamports);
    assert_eq!(env.lamports(&creator.pubkey()), 0);
    assert_eq!(env.fee_vault_free_lamports(&l), fv.creator_owed_lamports, "held on the FeeVault");

    // Enough volume later: the accrued line is paid in one go.
    let min = 890_880; // rent-exempt minimum of an empty account
    while env.fee_vault(&l).creator_owed_lamports < min {
        let got = env.buy_shares(&l, &trader, LAMPORTS / 10);
        env.sell_shares(&l, &trader, got);
        env.claim_tight_fees(&l, &caller);
        if env.lamports(&creator.pubkey()) > 0 {
            break;
        }
    }
    let fv = env.fee_vault(&l);
    assert_eq!(fv.creator_owed_lamports, 0);
    assert!(env.lamports(&creator.pubkey()) >= min);
    assert_eq!(fv.swept_creator_lamports, env.lamports(&creator.pubkey()));
}

#[test]
fn epoch_counters_roll_over() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 5);
    let trader = env.fund(10 * LAMPORTS);
    let caller = env.fund(LAMPORTS);
    let prizes = env.config().prizes;
    let epoch0 = prizes.epoch_at(env.now());

    let got = env.buy_shares(&l, &trader, LAMPORTS / 10);
    env.sell_shares(&l, &trader, got);
    let (r0, _) = env.claim_tight_fees(&l, &caller);
    let b = l.basket_state(&env);
    assert_eq!((b.fee_epoch, b.fee_epoch_lamports, b.fee_prev_epoch_lamports), (epoch0, r0, 0));

    // Next epoch: current moves to previous.
    env.warp(prizes.epoch_s);
    let got = env.buy_shares(&l, &trader, LAMPORTS / 20);
    env.sell_shares(&l, &trader, got);
    let (r1, _) = env.claim_tight_fees(&l, &caller);
    let b = l.basket_state(&env);
    assert_eq!((b.fee_epoch, b.fee_epoch_lamports, b.fee_prev_epoch_lamports), (epoch0 + 1, r1, r0));

    // Skipping an epoch: previous is empty.
    env.warp(2 * prizes.epoch_s);
    let got = env.buy_shares(&l, &trader, LAMPORTS / 20);
    env.sell_shares(&l, &trader, got);
    let (r3, _) = env.claim_tight_fees(&l, &caller);
    let b = l.basket_state(&env);
    assert_eq!((b.fee_epoch, b.fee_epoch_lamports, b.fee_prev_epoch_lamports), (epoch0 + 3, r3, 0));
}

#[test]
fn claim_is_permissionless_and_ignores_pause() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 6);
    let trader = env.fund(10 * LAMPORTS);
    env.buy_shares(&l, &trader, LAMPORTS / 10);
    env.set_paused(true);
    let anyone = env.fund(LAMPORTS);
    let (routed, _) = env.claim_tight_fees(&l, &anyone);
    assert!(routed > 0);
    // Wrong creator / team wallet accounts are refused.
    let b = l.basket_state(&env);
    let mut ix = env.claim_pool_fees_ix(&l, &anyone.pubkey(), &b.tight.key, b.tight.lower_bin_id, b.tight.upper_bin_id);
    let idx = ix.accounts.iter().position(|m| m.pubkey == l.creator.pubkey()).unwrap();
    ix.accounts[idx].pubkey = anyone.pubkey();
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::Unauthorized));
    let mut ix = env.claim_pool_fees_ix(&l, &anyone.pubkey(), &b.tight.key, b.tight.lower_bin_id, b.tight.upper_bin_id);
    let idx = ix.accounts.iter().position(|m| m.pubkey == env.config().team_wallet).unwrap();
    ix.accounts[idx].pubkey = anyone.pubkey();
    env.send_expect_err(&[ix], &anyone, &[], err(BasketError::Unauthorized));
}
