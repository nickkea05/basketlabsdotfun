//! `recenter_tight` (change order Q4, Q7): the keeper moves the tight
//! position to `[active − w, active + w]` when the active bin has drifted
//! past 70 % of the half-width (5-minute minimum interval) or has left the
//! range (always). While the mint gate is open the ask side is rebuilt by
//! minting treasury shares at the active-bin price.

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::dlmm;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn tight_center(b: &Basket) -> i32 {
    b.tight.lower_bin_id + b.tight.width() / 2
}

#[test]
fn recenter_needs_drift_past_the_trigger_band() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(3, 1);
    let keeper = env.keeper.insecure_clone();
    let b = l.basket_state(&env);
    let cfg = env.config();
    let half = b.tight.width() / 2;
    let trigger = half * cfg.pools.recenter_trigger_bps as i32 / BPS_TOTAL as i32;
    assert_eq!((half, trigger), (31, 21), "index preset: ±31 bins, trigger at 21");
    let center = tight_center(&b);
    assert_eq!(center, b.pool.launch_active_id);

    // At the centre, and just inside the band: refused.
    assert!(!env.recenter_allowed(&l));
    let ix = env.recenter_tight_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::RecenterNotNeeded));
    let trader = env.fund(50 * LAMPORTS);
    // The interval counts from launch: five minutes pass first.
    env.warp(cfg.pools.recenter_min_interval_s);
    env.buy_up_to(&l, &trader, center + trigger - 1);
    assert!(l.pool.active_id(&env) < center + trigger);
    assert!(!env.recenter_allowed(&l));
    let ix = env.recenter_tight_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::RecenterNotNeeded));

    // Past the trigger: allowed. Everything moves to the new range.
    env.buy_up_to(&l, &trader, center + trigger);
    let active = l.pool.active_id(&env);
    assert!(env.recenter_allowed(&l));
    let old = b.tight;
    let before = env.tight_amounts(&l);
    let idle_x = env.idle_shares(&l);
    let idle_y = env.idle_sol(&l);
    let supply_before = env.mint_supply(&l.share_mint);
    let h_before = env.holder_shares(&l);
    let keeper_before = env.lamports(&keeper.pubkey());
    let (new_position, lower, upper) = env.new_tight_for(&l, active);
    let (lo_new, hi_new) = dlmm::bin_array_range(lower, upper);
    let missing = (lo_new..=hi_new).filter(|i| env.account(&l.pool.bin_array(*i)).is_none()).count() as u64;
    let spent_before = b.deposit_spent_lamports;
    env.warp(1);
    let m = env.recenter_tight_ok(&l);
    eprintln!("recenter_tight CU: {} ({} new arrays)", m.compute_units_consumed, missing);
    assert!(m.compute_units_consumed < 1_400_000);

    let b = l.basket_state(&env);
    assert_eq!(b.tight.key, new_position);
    assert_eq!((b.tight.lower_bin_id, b.tight.upper_bin_id), (lower, upper));
    assert_eq!(tight_center(&b), active);
    assert_eq!(b.recenter_count, 1);
    assert_eq!(b.last_recenter_ts, env.now());
    assert!(env.account(&old.key).is_none(), "old position closed");
    // The backstop slice is still idle (unplaced), so only the removed amounts
    // went back in; the gate is open, so asks were topped up to match the bids.
    assert!(env.idle_shares(&l).abs_diff(idle_x) <= 70, "idle shares {} vs {idle_x} (per-bin dust)", env.idle_shares(&l));
    assert!(env.idle_sol(&l).abs_diff(idle_y) <= 70);
    let after = env.tight_amounts(&l);
    assert!(after.amount_y.abs_diff(before.amount_y) <= 70, "SOL side {} vs {}", after.amount_y, before.amount_y);
    let price = dlmm::bin_price_q64(active, b.pool.preset.bin_step).unwrap();
    let target_x = dlmm::x_for_lamports(before.amount_y, price).unwrap();
    let topped_up = env.mint_supply(&l.share_mint) - supply_before;
    assert!(target_x > before.amount_x, "a run-up leaves fewer asks than bids");
    assert!(topped_up.abs_diff(target_x - before.amount_x) <= 1, "top-up {topped_up} vs {}", target_x - before.amount_x);
    assert!(after.amount_x.abs_diff(target_x) <= 70, "shares side {} vs {target_x}", after.amount_x);
    // Treasury shares in positions are not holder shares.
    assert!(env.holder_shares(&l).abs_diff(h_before) <= 70);
    // Rent: the keeper paid the new position and got the old one's back; new
    // bin arrays came out of the deposit.
    let array_rent = b.deposit_spent_lamports - spent_before;
    assert!(array_rent.abs_diff(missing * 71_437_440) <= missing, "{array_rent}");
    let keeper_delta = env.lamports(&keeper.pubkey()) as i128 - keeper_before as i128;
    assert!(keeper_delta.abs() < 200_000, "position rent round-trips: {keeper_delta}");

    // Immediately after: inside the new range and within the interval → refused
    // even past the trigger; after five minutes it is allowed again.
    env.buy_up_to(&l, &trader, active + trigger);
    assert!(!env.recenter_allowed(&l));
    let ix = env.recenter_tight_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::RecenterNotNeeded));
    env.warp(cfg.pools.recenter_min_interval_s);
    assert!(env.recenter_allowed(&l));
    env.recenter_tight_ok(&l);
    assert_eq!(l.basket_state(&env).recenter_count, 2);
}

#[test]
fn out_of_range_recenters_at_once_and_gathers_the_idle_legs() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 2);
    env.place_and_fund_backstop(&l);
    let keeper = env.keeper.insecure_clone();
    env.recenter_tight(&l).err().expect("nothing to do at launch");
    let b = l.basket_state(&env);

    // Price crashes below tight into the backstop's bids (keeper late). A mint
    // now leaves its SOL leg idle.
    let seller = l.payer.insecure_clone();
    env.sell_down_to(&l, &seller, b.tight.lower_bin_id - 8);
    let active = l.pool.active_id(&env);
    assert!(!b.tight.contains(active));
    let deposits: Vec<u64> = env.vaults(&l).iter().map(|v| v / 4).collect();
    env.mint_from_new_buyer(&l, &deposits);
    let idle_y = env.idle_sol(&l);
    assert!(idle_y > 0, "the SOL leg waits idle");
    let removed = env.tight_amounts(&l);
    assert_eq!(removed.amount_y, 0, "tight is all shares once price is below it");

    // Out of range: allowed regardless of interval (last re-centre was "now").
    assert!(env.recenter_allowed(&l));
    let supply_before = env.mint_supply(&l.share_mint);
    let m = env.recenter_tight_ok(&l);
    eprintln!("recenter_tight (out of range, live backstop) CU: {}", m.compute_units_consumed);
    let b = l.basket_state(&env);
    assert_eq!(tight_center(&b), active);
    // Backstop live → idle belongs to the sleeve: it all went in.
    assert!(env.idle_sol(&l) <= 70 && env.idle_shares(&l) <= 70);
    let after = env.tight_amounts(&l);
    // (Within 1 %: the active bin is shared with the backstop, so a deposit
    // there buys a slice of its mixed composition.)
    assert!(after.amount_y <= idle_y && idle_y - after.amount_y < idle_y / 100, "{} vs {idle_y}", after.amount_y);
    // Gate open, asks already outnumber bids at this price: no top-up.
    let price = dlmm::bin_price_q64(active, b.pool.preset.bin_step).unwrap();
    let target_x = dlmm::x_for_lamports(idle_y, price).unwrap();
    assert!(removed.amount_x >= target_x);
    assert_eq!(env.mint_supply(&l.share_mint), supply_before, "no treasury shares minted");
    assert!(after.amount_x.abs_diff(removed.amount_x) < removed.amount_x / 100);
    // The backstop was not touched.
    assert_eq!(l.basket_state(&env).backstop_state, BACKSTOP_LIVE);

    // Immediately again (in range now) → refused.
    let ix = env.recenter_tight_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::RecenterNotNeeded));
}

#[test]
fn closed_gate_rebuilds_no_asks() {
    let mut env = Env::initialized();
    let now = env.now();
    let l = env.launch_with(2, 3, |a| a.gate = MintGate::WindowUntil { close_ts: now + DEFAULT_MIN_MINT_WINDOW_S });
    env.seed(&l, &SeedPlan::default_for(&l));
    env.place_and_fund_backstop(&l);
    let b = l.basket_state(&env);
    // Run-up past the tight range into the backstop's asks, then the window closes.
    let buyer = env.fund(100 * LAMPORTS);
    env.buy_up_to(&l, &buyer, b.tight.upper_bin_id + 5);
    env.warp(DEFAULT_MIN_MINT_WINDOW_S + 1);
    assert!(!l.basket_state(&env).gate_open_at(env.now()));
    let active = l.pool.active_id(&env);
    let removed = env.tight_amounts(&l);
    assert!(removed.amount_x <= 70 && removed.amount_y > 0, "all bids after a run-up");
    let supply_before = env.mint_supply(&l.share_mint);
    let idle_y = env.idle_sol(&l);

    let m = env.recenter_tight_ok(&l);
    eprintln!("recenter_tight (gate closed) CU: {}", m.compute_units_consumed);
    assert_eq!(env.mint_supply(&l.share_mint), supply_before, "no treasury shares minted while the gate is closed");
    let b = l.basket_state(&env);
    assert_eq!(tight_center(&b), active);
    let after = env.tight_amounts(&l);
    // No asks were built. (Depositing SOL into the active bin buys a slice of
    // that bin's mixed composition, so a sliver of X shows up: value-neutral.)
    let price = dlmm::bin_price_q64(active, b.pool.preset.bin_step).unwrap();
    let x_value = dlmm::lamports_for_x(after.amount_x, price).unwrap();
    assert!(x_value * 200 < after.amount_y, "asks come only from holders selling: {} shares (≈{x_value} lamports)", after.amount_x);
    let want_y = removed.amount_y + idle_y;
    assert!(after.amount_y <= want_y && want_y - after.amount_y < want_y / 100, "{} vs {want_y}", after.amount_y);
}

#[test]
fn keeper_only_pause_and_wrong_accounts() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 4);
    let b = l.basket_state(&env);
    let seller = l.payer.insecure_clone();
    env.push_price_below(&l, &seller, b.tight.lower_bin_id - 3);
    assert!(env.recenter_allowed(&l));
    env.warp(1);
    let keeper = env.keeper.insecure_clone();

    let stranger = env.fund(10 * LAMPORTS);
    let ix = env.recenter_tight_ix(&l, &stranger.pubkey());
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    env.set_paused(true);
    let ix = env.recenter_tight_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::Paused));
    env.set_paused(false);
    // Wrong new-position PDA.
    let mut ix = env.recenter_tight_ix(&l, &keeper.pubkey());
    let (new_position, _, _) = env.new_tight_for(&l, l.pool.active_id(&env));
    let idx = ix.accounts.iter().position(|m| m.pubkey == new_position).unwrap();
    ix.accounts[idx].pubkey = l.placeholder();
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PositionMismatch));
    // Missing bin arrays.
    let mut ix = env.recenter_tight_ix(&l, &keeper.pubkey());
    let fixed = env.keeper_pool_ix(&l, &keeper.pubkey(), None, None, vec![], vec![]).accounts.len();
    ix.accounts.truncate(fixed);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BinArrayMismatch));
    // Correct call goes through.
    env.recenter_tight_ok(&l);
}
