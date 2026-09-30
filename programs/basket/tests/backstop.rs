//! Keeper backstop lifecycle (change order Q5, Q6, Q15, round 2):
//! `place_backstop` → `fund_backstop` per bin array (ascending) → live →
//! `withdraw_backstop` per array (reset or wind-down) → `close_backstop` →
//! re-place around the current active bin.

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::dlmm;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn position_width(env: &Env, position: &Pubkey) -> i32 {
    let data = env.account(position).expect("position").data;
    let h = dlmm::position_header(&data).unwrap();
    h.upper_bin_id - h.lower_bin_id + 1
}

#[test]
fn place_backstop_reserves_four_arrays_around_the_launch_bin() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(3, 1);
    let b = l.basket_state(&env);
    assert_eq!(b.backstop_state, BACKSTOP_UNPLACED);
    assert!(!b.backstop.is_set());
    // The seed kept the backstop slice idle for the keeper.
    let plan = SeedPlan::default_for(&l);
    let slice = env.config().pools.backstop_slice_bps as u64;
    let idle_x = env.idle_shares(&l);
    let idle_y = env.idle_sol(&l);
    let treasury_shares = plan.initial_shares / 3; // r = 25 %: Y = X·r/(1−r)
    assert!(idle_x.abs_diff(treasury_shares * slice / BPS_TOTAL as u64) <= 70, "idle shares {idle_x}");
    assert!(idle_y.abs_diff(plan.sleeve_lamports * slice / BPS_TOTAL as u64) <= 70, "idle SOL {idle_y}");

    let (lower, upper, lo, hi) = env.backstop_placement(&l);
    let preset = b.pool.preset;
    assert_eq!(hi - lo + 1, preset.backstop_max_arrays as i64, "index preset: 4 arrays");
    assert!(lower <= b.pool.launch_active_id - preset.backstop_half_width_bins as i32);
    assert!(upper >= b.pool.launch_active_id + preset.backstop_half_width_bins as i32);
    // Two of the four arrays exist already (tight's); the keeper creates the rest.
    let existing = (lo..=hi).filter(|i| env.account(&l.pool.bin_array(*i)).is_some()).count();
    assert_eq!(existing, 2);

    let keeper = env.keeper.insecure_clone();
    let keeper_before = env.lamports(&keeper.pubkey());
    let spent_before = b.deposit_spent_lamports;
    let m = env.place_backstop(&l);
    eprintln!("place_backstop CU: {}", m.compute_units_consumed);

    let b = l.basket_state(&env);
    assert_eq!(b.backstop_state, BACKSTOP_FUNDING);
    assert_eq!(b.backstop_mask, 0);
    assert_eq!((b.backstop.lower_bin_id, b.backstop.upper_bin_id), (lower, upper));
    assert_eq!(b.backstop.key, env.backstop_position_for(&l));
    for i in lo..=hi {
        assert!(env.account(&l.pool.bin_array(i)).is_some(), "bin array {i} created");
    }
    // Created one array wide; grown as it is funded.
    assert_eq!(position_width(&env, &b.backstop.key), dlmm::MAX_BIN_PER_ARRAY);
    // Two new bin arrays were paid from the deposit; the position rent is the keeper's.
    let array_rent = b.deposit_spent_lamports - spent_before;
    let position_rent = env.lamports(&b.backstop.key);
    let keeper_paid = keeper_before - env.lamports(&keeper.pubkey());
    eprintln!("bin arrays from deposit: {array_rent}; position rent (keeper): {position_rent}");
    assert!(array_rent > 100_000_000 && array_rent < 160_000_000, "2 arrays ≈ 0.143 SOL: {array_rent}");
    assert!(keeper_paid.abs_diff(position_rent) < 50_000, "keeper paid {keeper_paid}, position rent {position_rent}");
    // Nothing deposited yet.
    assert_eq!(env.idle_shares(&l), idle_x);
    assert_eq!(env.idle_sol(&l), idle_y);

    // A second placement is refused while one is in progress.
    let ix = env.place_backstop_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
}

#[test]
fn fund_backstop_walks_the_arrays_in_ascending_order() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(3, 2);
    env.place_backstop(&l);
    let keeper = env.keeper.insecure_clone();
    let b = l.basket_state(&env);
    let (lo, hi) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
    let idle_x = env.idle_shares(&l);
    let idle_y = env.idle_sol(&l);
    let active = l.pool.active_id(&env);

    // Out of order, out of range: refused.
    let ix = env.fund_backstop_ix(&l, &keeper.pubkey(), lo + 1);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
    let ix = env.fund_backstop_ix(&l, &keeper.pubkey(), hi + 1);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BinArrayMismatch));
    // Wrong bin array in the tail.
    let mut ix = env.fund_backstop_ix(&l, &keeper.pubkey(), lo);
    let n = ix.accounts.len();
    ix.accounts[n - 1].pubkey = l.pool.bin_array(hi);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BinArrayMismatch));

    let mut funded_x = 0u64;
    let mut funded_y = 0u64;
    for idx in lo..=hi {
        let x_before = env.idle_shares(&l);
        let y_before = env.idle_sol(&l);
        let ix = env.fund_backstop_ix(&l, &keeper.pubkey(), idx);
        let m = env.send_ok(&[ix], &keeper, &[]);
        eprintln!("fund_backstop[{}] CU: {}", idx - lo, m.compute_units_consumed);
        assert!(m.compute_units_consumed < 1_400_000);
        let b = l.basket_state(&env);
        assert_eq!(b.backstop_mask, ((1u16 << (idx - lo + 1)) - 1) as u8, "mask after array {}", idx - lo);
        // The position now covers everything up to this array.
        assert_eq!(position_width(&env, &b.backstop.key), dlmm::array_upper_bin(idx) - b.backstop.lower_bin_id + 1);
        let dx = x_before - env.idle_shares(&l);
        let dy = y_before - env.idle_sol(&l);
        // Arrays below the active bin take SOL only, above take shares only.
        if dlmm::array_upper_bin(idx) < active {
            assert_eq!(dx, 0, "array {idx} is all bids");
            assert!(dy > 0);
        } else if dlmm::array_lower_bin(idx) > active {
            assert_eq!(dy, 0, "array {idx} is all asks");
            assert!(dx > 0);
        }
        funded_x += dx;
        funded_y += dy;
        // Repeating an array is refused.
        let ix = env.fund_backstop_ix(&l, &keeper.pubkey(), idx);
        env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
        assert_eq!(l.basket_state(&env).backstop_state, if idx == hi { BACKSTOP_LIVE } else { BACKSTOP_FUNDING });
    }
    // The whole idle slice went in (per-bin rounding dust aside).
    assert!(idle_x - funded_x <= 4 * 70, "idle shares left {}", idle_x - funded_x);
    assert!(idle_y - funded_y <= 4 * 70, "idle SOL left {}", idle_y - funded_y);
    let bs = env.backstop_amounts(&l);
    assert!(bs.amount_x.abs_diff(funded_x) <= 4 * 70 && bs.amount_y.abs_diff(funded_y) <= 4 * 70);
    // Every bin of the range holds something: flat bids below, flat asks above.
    let b = l.basket_state(&env);
    let (bid_x, bid_y) = l.pool.bin_amounts(&env, b.backstop.lower_bin_id, active - 1);
    let (ask_x, ask_y) = l.pool.bin_amounts(&env, active + 1, b.backstop.upper_bin_id);
    assert!(bid_y > 0 && ask_x > 0);
    let _ = (bid_x, ask_y); // tight shares the same bins near the active one
    // Placing again while live is refused.
    let ix = env.place_backstop_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
}

#[test]
fn mints_keep_working_while_the_backstop_is_being_funded() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 3);
    env.place_backstop(&l);
    let keeper = env.keeper.insecure_clone();
    let b = l.basket_state(&env);
    let (lo, hi) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
    env.send_ok(&[env.fund_backstop_ix(&l, &keeper.pubkey(), lo)], &keeper, &[]);

    // The backstop is narrower than its planned range: a mint still goes
    // through and sends all of its sleeve to tight (the backstop is not live).
    let deposits: Vec<u64> = env.vaults(&l).iter().map(|v| v / 4).collect();
    let tight_before = env.tight_amounts(&l);
    let q = env.mint_quote(&l, &deposits);
    assert_eq!((q.backstop_x, q.backstop_y), (0, 0));
    env.mint_from_new_buyer(&l, &deposits);
    let tight = env.tight_amounts(&l);
    assert!(tight.amount_y - tight_before.amount_y >= q.sleeve_lamports - 70);
    // A redeem too.
    let holder = l.payer.insecure_clone();
    let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 10;
    env.redeem(&l, &holder, shares);

    // Funding finishes with what is idle now.
    for idx in lo + 1..=hi {
        env.send_ok(&[env.fund_backstop_ix(&l, &keeper.pubkey(), idx)], &keeper, &[]);
    }
    assert_eq!(l.basket_state(&env).backstop_state, BACKSTOP_LIVE);
    assert!(env.idle_shares(&l) <= 4 * 70 && env.idle_sol(&l) <= 4 * 70);
}

#[test]
fn withdraw_needs_a_reset_or_a_wind_down_then_close_and_replace() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 4);
    env.place_and_fund_backstop(&l);
    let keeper = env.keeper.insecure_clone();
    let b = l.basket_state(&env);
    let (lo, hi) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);
    let old_position = b.backstop.key;

    // Price inside the backstop, gate open: nothing to withdraw yet.
    let ix = env.withdraw_backstop_ix(&l, &keeper.pubkey(), lo);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
    // Closing before the arrays are emptied is refused too.
    let ix = env.close_backstop_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));

    // A run-up carries the active bin above the backstop: a reset is due.
    let holder = l.payer.insecure_clone();
    env.push_price_above(&l, &holder, b.backstop.upper_bin_id + 10);
    let active = l.pool.active_id(&env);
    assert!(active > b.backstop.upper_bin_id);
    let bs = env.backstop_amounts(&l);
    assert_eq!(bs.amount_x, 0, "all asks were taken on the way up");
    assert!(bs.amount_y > 0);
    let fv_before: FeeVault = env.load(&fee_vault_pda(&l.share_mint));
    let idle_y_before = env.idle_sol(&l);
    let keeper_before = env.lamports(&keeper.pubkey());

    // Arrays are emptied one per transaction, in any order; each clears its bit.
    let order = [hi, lo, lo + 1, hi - 1];
    for (n, idx) in order.iter().enumerate() {
        let ix = env.withdraw_backstop_ix(&l, &keeper.pubkey(), *idx);
        let m = env.send_ok(&[ix], &keeper, &[]);
        eprintln!("withdraw_backstop[{}] CU: {}", idx - lo, m.compute_units_consumed);
        let b = l.basket_state(&env);
        assert_eq!(b.backstop_state, BACKSTOP_WITHDRAWING);
        assert_eq!(b.backstop_mask & (1 << (idx - lo)), 0);
        assert_eq!(b.backstop_mask.count_ones() as usize, 4 - n - 1);
        // An emptied array cannot be withdrawn twice.
        let ix = env.withdraw_backstop_ix(&l, &keeper.pubkey(), *idx);
        env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
    }
    let b = l.basket_state(&env);
    assert_eq!(b.backstop_mask, 0);
    let bs = env.backstop_amounts(&l);
    assert_eq!((bs.amount_x, bs.amount_y), (0, 0));
    // The SOL came back idle; the swap fees earned in those bins are parked
    // on the FeeVault for the next `claim_pool_fees` to route.
    let returned = env.idle_sol(&l) - idle_y_before;
    assert!(returned > 0);
    let fv: FeeVault = env.load(&fee_vault_pda(&l.share_mint));
    let parked = fv.unrouted_lamports - fv_before.unrouted_lamports;
    assert!(parked > 0, "backstop bins earned fees on the run-up");
    assert!(env.lamports(&fee_vault_pda(&l.share_mint)) >= fv.unrouted_lamports);
    assert!(b.fee_epoch_lamports >= parked, "counted towards the prize epoch");

    // Close: rent back to the keeper, state Closed, ref cleared.
    let rent = env.lamports(&old_position);
    let m = env.close_backstop(&l);
    eprintln!("close_backstop CU: {}", m.compute_units_consumed);
    assert!(env.account(&old_position).is_none());
    let b = l.basket_state(&env);
    assert_eq!(b.backstop_state, BACKSTOP_CLOSED);
    assert!(!b.backstop.is_set());
    let keeper_delta = env.lamports(&keeper.pubkey()) as i128 - keeper_before as i128;
    assert!(keeper_delta > rent as i128 - 100_000, "keeper got the position rent back: {keeper_delta} vs {rent}");

    // Re-place around the current active bin; the arrays here are new.
    let (lower, upper, lo2, hi2) = env.backstop_placement(&l);
    assert!(lower <= active && active <= upper);
    assert!(lo2 > lo);
    let spent_before = b.deposit_spent_lamports;
    let missing = (lo2..=hi2).filter(|i| env.account(&l.pool.bin_array(*i)).is_none()).count() as u64;
    env.place_backstop(&l);
    let b = l.basket_state(&env);
    assert_eq!(b.backstop_state, BACKSTOP_FUNDING);
    assert_eq!((b.backstop.lower_bin_id, b.backstop.upper_bin_id), (lower, upper));
    assert_ne!(b.backstop.key, old_position);
    let array_rent = b.deposit_spent_lamports - spent_before;
    eprintln!("re-place: {missing} new arrays, {array_rent} lamports from deposit");
    assert!(missing > 0 && array_rent > 0);
    assert!(array_rent.abs_diff(missing * 71_437_440) <= missing, "one array ≈ 0.0714 SOL each");
    // Fund again from the returned SOL (all bids now: the price sits at the top
    // of what holders sold; asks come from the next mint or re-centre).
    let cus = env.fund_backstop_all(&l);
    eprintln!("re-fund CU: {cus:?}");
    let b = l.basket_state(&env);
    assert_eq!(b.backstop_state, BACKSTOP_LIVE);
    let bs = env.backstop_amounts(&l);
    assert!(bs.amount_y > 0);
    assert!(env.idle_sol(&l) <= 4 * 70);
}

#[test]
fn wind_down_lets_the_keeper_withdraw_with_the_price_inside() {
    let mut env = Env::initialized();
    let now = env.now();
    let l = env.launch_with(2, 5, |a| a.gate = MintGate::WindowUntil { close_ts: now + DEFAULT_MIN_MINT_WINDOW_S });
    env.seed(&l, &SeedPlan::default_for(&l));
    env.place_and_fund_backstop(&l);
    let keeper = env.keeper.insecure_clone();
    let b = l.basket_state(&env);
    let (lo, _) = dlmm::bin_array_range(b.backstop.lower_bin_id, b.backstop.upper_bin_id);

    // Gate closed but not idle long enough.
    env.warp(DEFAULT_MIN_MINT_WINDOW_S + 1);
    let ix = env.withdraw_backstop_ix(&l, &keeper.pubkey(), lo);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
    // Idle for `close_idle_s`: allowed.
    env.warp(env.config().close_idle_s);
    let cus = env.withdraw_backstop_all(&l);
    eprintln!("wind-down withdraw CU: {cus:?}");
    env.close_backstop(&l);
    assert_eq!(l.basket_state(&env).backstop_state, BACKSTOP_CLOSED);
}

#[test]
fn keeper_only_and_pause() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 6);
    let stranger = env.fund(10 * LAMPORTS);
    let ix = env.place_backstop_ix(&l, &stranger.pubkey());
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    env.set_paused(true);
    let keeper = env.keeper.insecure_clone();
    let ix = env.place_backstop_ix(&l, &keeper.pubkey());
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::Paused));
    env.set_paused(false);
    env.place_backstop(&l);
    // Not seeded: nothing to place (the FeeVault does not even exist yet).
    let l2 = env.launch_fixed(2, 7);
    let ix = env.place_backstop_ix(&l2, &keeper.pubkey());
    assert!(env.send(&[ix], &keeper, &[]).is_err());
}
