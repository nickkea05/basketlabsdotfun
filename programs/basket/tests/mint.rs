//! `mint`: later buys. Creation-unit rule over the vaults (D4), treasury
//! shares placed in the tight position at the active bin's price with the
//! sleeve step rule (D5), 20 % of the sleeve to the backstop while it is
//! live (Q6), gate (D6), 0.1 SOL minimum, mint fee as shares (§4).

mod common;

use anchor_lang::InstructionData;
use basket::constants::*;
use basket::dlmm;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer as _;

/// Seed a 3-asset basket and return a funded second buyer.
fn seeded(env: &mut Env, nonce: u64) -> (Launched, Keypair) {
    let (l, _) = env.launch_and_seed(3, nonce);
    let buyer = env.fund(100 * LAMPORTS);
    (l, buyer)
}

/// Fee shares the default seed plan put in the FeeVault.
fn plan_fee(l: &Launched) -> u64 {
    SeedPlan::default_for(l).initial_shares / 100
}

#[test]
fn mint_follows_the_creation_unit_rule_and_pool_price() {
    let mut env = Env::initialized();
    let (l, buyer) = seeded(&mut env, 1);

    let vaults = env.vaults(&l);
    let supply_before = env.mint_supply(&l.share_mint);
    let tight_before = env.tight_amounts(&l);
    let idle_x_before = env.idle_shares(&l);
    let idle_y_before = env.idle_sol(&l);
    let holders_before = env.holder_shares(&l);
    assert_eq!(holders_before, SeedPlan::default_for(&l).initial_shares);

    // Deposit exactly half of every vault -> X = H / 2.
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let quote = env.mint_quote(&l, &deposits);
    let x = holders_before / 2;
    assert_eq!(quote.gross_shares, x);
    let r = DEFAULT_STEP_R_BPS; // pool SOL (0.33) < 20 SOL step threshold
    assert_eq!(quote.r_bps, r);
    let y = x * r as u64 / (BPS_TOTAL - r) as u64;
    assert_eq!(quote.treasury_shares, y);
    // SOL leg = Y at the active bin's price: half the seed's SOL leg.
    let active = l.pool.active_id(&env);
    assert_eq!(quote.sleeve_lamports, dlmm::lamports_for_x(y, l.pool.price_q64(active)).unwrap());
    assert_eq!(quote.sleeve_lamports, SeedPlan::default_for(&l).sleeve_lamports / 2);
    assert_eq!((quote.backstop_x, quote.backstop_y), (0, 0), "no backstop yet: everything to tight");

    let buyer_before = env.lamports(&buyer.pubkey());
    let m = env.mint(&l, &buyer, &deposits, 0, quote.sleeve_lamports);
    eprintln!("mint(3 positions) CU: {}", m.cu);
    assert!(m.cu < 1_200_000, "mint CU {}", m.cu);

    // Vaults grew by the deposits.
    for (i, mint) in l.mints.iter().enumerate() {
        assert_eq!(env.token_amount(&Env::ata(&l.basket, mint)), vaults[i] + deposits[i]);
    }
    // Buyer got X less the 1 % fee; fee vault got the fee.
    let fee = x / 100;
    assert_eq!(env.token_amount(&Env::ata(&buyer.pubkey(), &l.share_mint)), x - fee);
    assert_eq!(env.fee_shares(&l), plan_fee(&l) + fee);
    // Tight got Y shares and the matching SOL at the unchanged price.
    assert_eq!(l.pool.active_id(&env), active, "adding at the active price does not move it");
    let tight = env.tight_amounts(&l);
    let dx = tight.amount_x - tight_before.amount_x;
    let dy = tight.amount_y - tight_before.amount_y;
    assert!(dx <= y && y - dx <= 70, "tight X grew {dx} vs {y}");
    assert!(dy <= quote.sleeve_lamports && quote.sleeve_lamports - dy <= 70, "tight Y grew {dy} vs {}", quote.sleeve_lamports);
    // Only DLMM's per-bin rounding stays idle.
    assert!(env.idle_shares(&l) - idle_x_before <= 70);
    assert!(env.idle_sol(&l) - idle_y_before <= 70);
    // Buyer paid the sleeve SOL (plus tx fees / ATA rent).
    let spent = buyer_before - env.lamports(&buyer.pubkey());
    assert!(spent >= quote.sleeve_lamports && spent < quote.sleeve_lamports + 10_000_000);
    // Supply grew by X + Y; holders by X.
    assert_eq!(env.mint_supply(&l.share_mint), supply_before + x + y);
    assert_eq!(env.holder_shares(&l), holders_before + x);
}

#[test]
fn mint_takes_the_minimum_ratio() {
    let mut env = Env::initialized();
    let (l, buyer) = seeded(&mut env, 2);
    let vaults = env.vaults(&l);
    let holders = env.holder_shares(&l);

    // 50% / 50% / 10% -> X = 10% of H; excess of the first two stays in the vaults.
    let deposits = vec![vaults[0] / 2, vaults[1] / 2, vaults[2] / 10];
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let quote = env.mint_quote(&l, &deposits);
    assert_eq!(quote.gross_shares, holders / 10);
    env.mint(&l, &buyer, &deposits, 0, quote.sleeve_lamports);
    let x = holders / 10;
    assert_eq!(env.token_amount(&Env::ata(&buyer.pubkey(), &l.share_mint)), x - x / 100);
    assert_eq!(env.token_amount(&Env::ata(&l.basket, &l.mints[0])), vaults[0] + vaults[0] / 2);
}

#[test]
fn mint_prices_the_sleeve_at_the_current_active_bin() {
    // After trading moves the price, a mint's SOL leg follows the active bin,
    // not the launch bin; the quote still needs no oracle.
    let mut env = Env::initialized();
    let (l, buyer) = seeded(&mut env, 3);
    let launch_quote = env.mint_quote(&l, &[500_000_000, 500_000_000, 500_000_000]);
    // Buy 0.05 SOL of shares: the active bin climbs a few steps.
    let (_trader, _) = env.trader_who_bought(&l, LAMPORTS / 20);
    let active = l.pool.active_id(&env);
    assert!(active > LAUNCH_ACTIVE_ID, "active {active}");
    let quote = env.mint_quote(&l, &[500_000_000, 500_000_000, 500_000_000]);
    assert!(quote.sleeve_lamports > launch_quote.sleeve_lamports, "{} vs {}", quote.sleeve_lamports, launch_quote.sleeve_lamports);
    assert_eq!(quote.sleeve_lamports, dlmm::lamports_for_x(quote.treasury_shares, l.pool.price_q64(active)).unwrap());
    env.fund_components_for(&buyer.pubkey(), &l, &[500_000_000, 500_000_000, 500_000_000]);
    let ix = env.mint_ix(&l, &buyer.pubkey(), &[500_000_000, 500_000_000, 500_000_000], 0, launch_quote.sleeve_lamports);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::SlippageExceeded));
    env.mint(&l, &buyer, &[500_000_000, 500_000_000, 500_000_000], 0, quote.sleeve_lamports);
    assert_eq!(l.pool.active_id(&env), active);
}

#[test]
fn sleeve_steps_down_once_the_pool_holds_enough_sol() {
    let mut env = Env::new();
    // Step threshold 0.5 SOL so the default 0.33 SOL seed sits below it.
    let mut sleeve = SleeveDefaults::default();
    sleeve.step_threshold_lamports = 500_000_000;
    env.initialize_config(ConfigUpdate { sleeve: Some(sleeve), ..Default::default() }).unwrap();
    let (l, buyer) = seeded(&mut env, 3);
    let vaults = env.vaults(&l);

    // First mint: pool SOL 0.33 < 0.5 -> r = 25 %. Deposit 1x -> pool SOL doubles to 0.67.
    let deposits: Vec<u64> = vaults.clone();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let q1 = env.mint_quote(&l, &deposits);
    assert_eq!(q1.r_bps, DEFAULT_STEP_R_BPS);
    env.mint(&l, &buyer, &deposits, 0, q1.sleeve_lamports);
    let pool_sol = env.sol_in_positions(&l) + env.idle_sol(&l);
    assert!(pool_sol >= 500_000_000, "pool SOL {pool_sol}");

    // Second mint: r = the creator's 7 %.
    let vaults2 = env.vaults(&l);
    let deposits2: Vec<u64> = vaults2.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits2);
    let q2 = env.mint_quote(&l, &deposits2);
    assert_eq!(q2.r_bps, 700);
    assert_eq!(q2.treasury_shares, q2.gross_shares * 700 / 9300);
    // 7 % sleeve vs 25 %: the SOL leg per share drops accordingly.
    let per_share_1 = q1.sleeve_lamports as f64 / q1.gross_shares as f64;
    let per_share_2 = q2.sleeve_lamports as f64 / q2.gross_shares as f64;
    let expected = (700.0 / 9300.0) / (2500.0 / 7500.0);
    assert!((per_share_2 / per_share_1 - expected).abs() < 0.01, "{}", per_share_2 / per_share_1);
    let m = env.mint(&l, &buyer, &deposits2, 0, q2.sleeve_lamports);
    assert!(m.logs.iter().any(|l| l.contains("Instruction: AddLiquidityByStrategy2")), "logs: {:#?}", m.logs);
}

#[test]
fn mint_funds_the_backstop_once_it_is_live() {
    let mut env = Env::initialized();
    let (l, buyer) = seeded(&mut env, 8);
    env.place_and_fund_backstop(&l);
    let b = l.basket_state(&env);
    assert_eq!(b.backstop_state, BACKSTOP_LIVE);
    let backstop_before = env.backstop_amounts(&l);
    let tight_before = env.tight_amounts(&l);

    let vaults = env.vaults(&l);
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let q = env.mint_quote(&l, &deposits);
    let slice = env.config().pools.backstop_slice_bps;
    assert_eq!(q.backstop_x, q.treasury_shares * slice as u64 / BPS_TOTAL as u64);
    assert_eq!(q.backstop_y, q.sleeve_lamports * slice as u64 / BPS_TOTAL as u64);
    let m = env.mint(&l, &buyer, &deposits, 0, q.sleeve_lamports);
    eprintln!("mint with backstop add CU: {}", m.cu);
    assert!(m.cu < 1_400_000, "mint CU {}", m.cu);

    let backstop = env.backstop_amounts(&l);
    let tight = env.tight_amounts(&l);
    let bx = backstop.amount_x - backstop_before.amount_x;
    let by = backstop.amount_y - backstop_before.amount_y;
    let tx = tight.amount_x - tight_before.amount_x;
    let ty = tight.amount_y - tight_before.amount_y;
    // 20 % to the backstop, 80 % to tight (DLMM rounding per bin).
    assert!(bx <= q.backstop_x && q.backstop_x - bx <= 70, "backstop X {bx} vs {}", q.backstop_x);
    assert!(by <= q.backstop_y && q.backstop_y - by <= 70, "backstop Y {by} vs {}", q.backstop_y);
    let want_tx = q.treasury_shares - q.backstop_x;
    let want_ty = q.sleeve_lamports - q.backstop_y;
    assert!(tx <= want_tx && want_tx - tx <= 70, "tight X {tx} vs {want_tx}");
    assert!(ty <= want_ty && want_ty - ty <= 70, "tight Y {ty} vs {want_ty}");
    assert_eq!(env.holder_shares(&l), SeedPlan::default_for(&l).initial_shares + q.gross_shares);
}

#[test]
fn mint_with_the_active_bin_outside_tight_places_only_what_fits() {
    // Keeper late: the price has left the tight range. The mint still goes
    // through; the leg the range cannot hold waits idle for recenter_tight.
    let mut env = Env::initialized();
    let (l, buyer) = seeded(&mut env, 9);
    let b = l.basket_state(&env);
    // The seed buyer dumps into outside bids until the active bin is below tight.
    env.push_price_below(&l, &l.payer, b.tight.lower_bin_id - 10);
    let active = l.pool.active_id(&env);
    assert!(active < b.tight.lower_bin_id, "active {active}");

    let idle_x_before = env.idle_shares(&l);
    let idle_y_before = env.idle_sol(&l);
    let tight_before = env.tight_amounts(&l);
    assert_eq!(tight_before.amount_y, 0, "all tight bids were consumed");
    let vaults = env.vaults(&l);
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let q = env.mint_quote(&l, &deposits);
    env.mint(&l, &buyer, &deposits, 0, q.sleeve_lamports);

    // Tight is entirely above the active bin: only shares (asks) can go in;
    // the SOL leg stays idle.
    let tight = env.tight_amounts(&l);
    let dx = tight.amount_x - tight_before.amount_x;
    assert!(dx <= q.treasury_shares && q.treasury_shares - dx <= 70, "tight X grew {dx} vs {}", q.treasury_shares);
    assert_eq!(tight.amount_y, 0);
    assert_eq!(env.idle_sol(&l) - idle_y_before, q.sleeve_lamports, "SOL leg waits idle");
    assert!(env.idle_shares(&l) - idle_x_before <= 70);
    // The buyer's shares are the creation-unit amount regardless of the price.
    assert_eq!(env.token_amount(&Env::ata(&buyer.pubkey(), &l.share_mint)), q.gross_shares - q.gross_shares / 100);
}

#[test]
fn slippage_minimum_gate_and_pause() {
    let mut env = Env::initialized();
    let (l, buyer) = seeded(&mut env, 4);
    let vaults = env.vaults(&l);
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let q = env.mint_quote(&l, &deposits);

    // SOL cap below the quote.
    let ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, 0, q.sleeve_lamports - 1);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::SlippageExceeded));
    // Share floor above what the deposit buys.
    let ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, q.gross_shares, q.sleeve_lamports);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::SlippageExceeded));
    // Below the 0.1 SOL minimum: 1/1000 of the vaults is ~0.0013 SOL of value.
    let tiny: Vec<u64> = vaults.iter().map(|v| v / 1000).collect();
    let ix = env.mint_ix(&l, &buyer.pubkey(), &tiny, 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::BelowMinimum));
    // Wrong deposit count.
    let ix = env.mint_ix(&l, &buyer.pubkey(), &deposits[..2], 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::ComponentCountMismatch));

    // Pause blocks mint.
    env.set_paused(true);
    let ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, 0, q.sleeve_lamports);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::Paused));
    env.set_paused(false);

    // Gate: the 24 h window closes, then a reopen schedule lets buys through again.
    let now = env.now();
    let window = DEFAULT_MIN_MINT_WINDOW_S;
    let l2 = env.launch_with(2, 5, |a| {
        a.gate = MintGate::WindowUntil { close_ts: now + window };
        a.schedule = Some(ReopenSchedule { anchor_ts: now + window + 1_000, period_s: 1_000, open_s: 100 });
    });
    let plan = SeedPlan::default_for(&l2);
    env.seed(&l2, &plan);
    let v2 = env.vaults(&l2);
    let d2: Vec<u64> = v2.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l2, &d2);
    env.mint(&l2, &buyer, &d2, 0, u64::MAX); // inside the window
    env.warp(window + 200); // window closed, schedule not yet anchored
    env.fund_components_for(&buyer.pubkey(), &l2, &d2);
    let ix = env.mint_ix(&l2, &buyer.pubkey(), &d2, 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::MintGateClosed));
    env.warp(850); // t = now + window + 1050: inside the first reopen window
    env.mint(&l2, &buyer, &d2, 0, u64::MAX);
    env.warp(100); // t = now + window + 1150: window over
    env.fund_components_for(&buyer.pubkey(), &l2, &d2);
    let ix = env.mint_ix(&l2, &buyer.pubkey(), &d2, 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::MintGateClosed));
}

#[test]
fn mint_rejects_foreign_or_partial_component_sets() {
    let mut env = Env::initialized();
    let (l, buyer) = seeded(&mut env, 6);
    let (l_other, _) = seeded(&mut env, 7);
    let vaults = env.vaults(&l);
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let tail_len = env.pool_tail(&l).len();

    // Only two of three components passed.
    let mut ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, 0, u64::MAX);
    let comp_start = ix.accounts.len() - tail_len - 3 * 4;
    ix.accounts.drain(comp_start + 2 * 4..comp_start + 3 * 4);
    ix.data = basket::instruction::Mint {
        args: MintArgs { deposits: deposits[..2].to_vec(), min_shares_out: 0, max_sleeve_lamports: u64::MAX },
    }
    .data();
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::ComponentCountMismatch));

    // A position from another basket.
    let mut ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, 0, u64::MAX);
    let other = component_metas(&l_other.basket, &l_other.share_mint, &buyer.pubkey(), &l_other.book[..1]);
    ix.accounts.splice(comp_start + 2 * 4..comp_start + 3 * 4, other);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::ComponentMismatch));

    // Unseeded basket cannot mint (no FeeVault yet, so Anchor rejects first).
    let l3 = env.launch_fixed(2, 30);
    env.fund_components_for(&buyer.pubkey(), &l3, &[1, 1]);
    let mut ix = env.mint_ix(&l3, &buyer.pubkey(), &[1, 1], 0, u64::MAX);
    for m in ix.accounts.iter_mut() {
        if m.pubkey == anchor_lang::prelude::Pubkey::default() {
            m.pubkey = l3.placeholder();
        }
    }
    assert!(env.send(&[ix], &buyer, &[]).is_err());
}

/// 22 fixed accounts + 4 per component + bin arrays against the 64-account
/// lock limit: N ≤ 9 in one transaction (seed caps at 9 as well).
#[test]
fn mint_account_counts_and_cu_by_size() {
    let mut env = Env::initialized();
    for (i, n) in [2usize, 5, 8, 9].iter().enumerate() {
        let (l, _) = env.launch_and_seed(*n, 20 + i as u64);
        let buyer = env.fund(100 * LAMPORTS);
        let vaults = env.vaults(&l);
        let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
        env.fund_components_for(&buyer.pubkey(), &l, &deposits);
        let ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, 0, u64::MAX);
        let accounts = ix.accounts.len();
        assert!(accounts <= 64, "N={n}: {accounts} accounts");
        let m = env.mint(&l, &buyer, &deposits, 0, u64::MAX);
        eprintln!("mint N={n}: {accounts} accounts, {} CU", m.cu);
        if *n >= 8 {
            // With a live backstop the mint carries its bin arrays too (H and
            // the pool's SOL side are read from both positions): N=8 is the
            // widest single-tx mint; N=9 runs out of account locks.
            env.place_and_fund_backstop(&l);
            env.fund_components_for(&buyer.pubkey(), &l, &deposits);
            let ix = env.mint_ix(&l, &buyer.pubkey(), &deposits, 0, u64::MAX);
            let accounts = ix.accounts.len();
            let r = env.send(&[ix], &buyer, &[]);
            if *n == 8 {
                let m = r.expect("mint N=8 with live backstop");
                eprintln!("mint N={n} + backstop add: {accounts} accounts, {} CU", m.compute_units_consumed);
                assert!(m.compute_units_consumed < 1_400_000, "mint CU {}", m.compute_units_consumed);
            } else {
                let e = r.err().expect("N=9 with live backstop exceeds 64 locks");
                assert!(format!("{:?}", e.err).contains("TooManyAccountLocks"), "{:?}", e.err);
                eprintln!("mint N={n} + backstop: {accounts} accounts -> TooManyAccountLocks");
            }
        }
    }
}
