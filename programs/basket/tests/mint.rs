//! `mint`: later buys. Creation-unit rule over the vaults (D4), treasury
//! shares added to the pool at its current ratio with the sleeve step rule
//! (D5), gate (D6), 0.1 SOL minimum, mint fee as shares (§4).

mod common;

use anchor_lang::prelude::*;
use anchor_lang::InstructionData;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

/// Seed a 3-asset basket and return a funded second buyer.
fn seeded(env: &mut Env, nonce: u64) -> (Launched, SeededBasket, solana_keypair::Keypair) {
    let l = env.launch_fixed(3, nonce);
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    let buyer = env.fund(100 * LAMPORTS);
    (l, s, buyer)
}

#[test]
fn mint_follows_the_creation_unit_rule_and_pool_ratio() {
    let mut env = Env::initialized();
    let (l, s, buyer) = seeded(&mut env, 1);

    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    let supply_before = env.mint_supply(&l.share_mint);
    let pool_before = env.pool_state(&s.pool);
    let pool_a_before = env.token_amount(&pool_before.token_a_vault);
    let pool_b_before = env.token_amount(&pool_before.token_b_vault);
    let holders_before = env.holder_shares(&l, &s);
    assert!(holders_before + 2 >= supply_before - pool_a_before && holders_before <= supply_before - pool_a_before + 2);

    // Deposit exactly half of every vault -> X = H / 2.
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let quote = env.mint_quote(&l, &s, &deposits);
    let x = holders_before / 2;
    assert_eq!(quote.gross_shares, x);
    let r = DEFAULT_STEP_R_BPS; // pool SOL (0.3) < 20 SOL step threshold
    assert_eq!(quote.r_bps, r);
    let y = x * r as u64 / (BPS_TOTAL - r) as u64;
    assert_eq!(quote.nominal_treasury_shares, y);
    // SOL leg = nominal Y at the pool price = r/(1-r) of the components' value.
    let price_b = basket::math::lamports_for_shares(y, pool_before.sqrt_price).unwrap();
    assert!(quote.sleeve_lamports.abs_diff(price_b) <= 2);
    // Half the vaults at the seed price = half the seed's SOL leg.
    assert!(quote.sleeve_lamports.abs_diff(pool_b_before / 2) <= 2, "{} vs {}", quote.sleeve_lamports, pool_b_before / 2);

    let buyer_before = env.lamports(&buyer.pubkey());
    let m = env.mint(&l, &s, &buyer, &deposits, 0, quote.sleeve_lamports);
    println!("mint(3 positions) CU: {}", m.cu);

    // Vaults grew by the deposits.
    for (i, mint) in l.mints.iter().enumerate() {
        assert_eq!(env.token_amount(&Env::ata(&l.basket, mint)), vaults[i] + deposits[i]);
    }
    // Buyer got X less the 1% fee; fee vault got the fee.
    let fee = x / 100;
    assert_eq!(env.token_amount(&Env::ata(&buyer.pubkey(), &l.share_mint)), x - fee);
    let fee_ata = Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint);
    assert_eq!(env.token_amount(&fee_ata), plan_fee(&l) + fee);
    // Pool got Y shares and the matching SOL at the unchanged price.
    let pool = env.pool_state(&s.pool);
    assert_eq!(pool.sqrt_price, pool_before.sqrt_price, "adding at ratio does not move price");
    let pool_a = env.token_amount(&pool.token_a_vault);
    let pool_b = env.token_amount(&pool.token_b_vault);
    assert_eq!(pool_a - pool_a_before, quote.treasury_shares);
    assert_eq!(pool_b - pool_b_before, quote.sleeve_lamports);
    // Sleeve SOL ≈ Y × price: pool value ratio preserved (Y/y0 == b/b0).
    let ratio_a = (pool_a as f64) / (pool_a_before as f64);
    let ratio_b = (pool_b as f64) / (pool_b_before as f64);
    assert!((ratio_a - ratio_b).abs() < 1e-6, "{ratio_a} vs {ratio_b}");
    // No treasury-share dust left with the basket.
    assert_eq!(env.token_amount(&Env::ata(&l.basket, &l.share_mint)), 0);
    // Buyer paid the sleeve SOL (plus tx fees / ATA rent).
    let spent = buyer_before - env.lamports(&buyer.pubkey());
    assert!(spent >= quote.sleeve_lamports && spent < quote.sleeve_lamports + 10_000_000);
    // Supply grew by X + the pool's share side.
    let supply = env.mint_supply(&l.share_mint);
    assert_eq!(supply, supply_before + x + quote.treasury_shares);
}

#[test]
fn mint_takes_the_minimum_ratio() {
    let mut env = Env::initialized();
    let (l, s, buyer) = seeded(&mut env, 2);
    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    let holders = env.holder_shares(&l, &s);

    // 50% / 50% / 10% -> X = 10% of H; excess of the first two stays in the vaults.
    let deposits = vec![vaults[0] / 2, vaults[1] / 2, vaults[2] / 10];
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let quote = env.mint_quote(&l, &s, &deposits);
    assert_eq!(quote.gross_shares, holders / 10);
    env.mint(&l, &s, &buyer, &deposits, 0, quote.sleeve_lamports);
    let x = holders / 10;
    assert_eq!(env.token_amount(&Env::ata(&buyer.pubkey(), &l.share_mint)), x - x / 100);
    assert_eq!(env.token_amount(&Env::ata(&l.basket, &l.mints[0])), vaults[0] + vaults[0] / 2);
}

#[test]
fn sleeve_steps_down_once_the_pool_holds_enough_sol() {
    let mut env = Env::new();
    // Step threshold 0.5 SOL so the default 0.3 SOL seed sits below it.
    let mut sleeve = SleeveDefaults::default();
    sleeve.step_threshold_lamports = 500_000_000;
    env.initialize_config(ConfigUpdate { sleeve: Some(sleeve), ..Default::default() }).unwrap();
    let (l, s, buyer) = seeded(&mut env, 3);
    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();

    // First mint: pool SOL 0.3 < 0.5 -> r = 25%. Deposit 1x -> pool SOL doubles to 0.6.
    let deposits: Vec<u64> = vaults.clone();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let q1 = env.mint_quote(&l, &s, &deposits);
    assert_eq!(q1.r_bps, DEFAULT_STEP_R_BPS);
    env.mint(&l, &s, &buyer, &deposits, 0, q1.sleeve_lamports);
    let pool = env.pool_state(&s.pool);
    assert!(pool.token_b_amount >= 500_000_000, "pool SOL {}", pool.token_b_amount);

    // Second mint: r = the creator's 7%.
    let vaults2: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    let deposits2: Vec<u64> = vaults2.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits2);
    let q2 = env.mint_quote(&l, &s, &deposits2);
    assert_eq!(q2.r_bps, 700);
    assert_eq!(q2.nominal_treasury_shares, q2.gross_shares * 700 / 9300);
    // 7% sleeve vs 25%: the SOL leg per share drops accordingly.
    let per_share_1 = q1.sleeve_lamports as f64 / q1.gross_shares as f64;
    let per_share_2 = q2.sleeve_lamports as f64 / q2.gross_shares as f64;
    let expected = (700.0 / 9300.0) / (2500.0 / 7500.0);
    assert!((per_share_2 / per_share_1 - expected).abs() < 0.01, "{}", per_share_2 / per_share_1);
    let m = env.mint(&l, &s, &buyer, &deposits2, 0, q2.sleeve_lamports);
    assert!(m.logs.iter().any(|l| l.contains("Instruction: AddLiquidity")));
}

#[test]
fn slippage_minimum_gate_and_pause() {
    let mut env = Env::initialized();
    let (l, s, buyer) = seeded(&mut env, 4);
    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let q = env.mint_quote(&l, &s, &deposits);

    // SOL cap below the quote.
    let ix = env.mint_ix(&l, &s, &buyer.pubkey(), &deposits, 0, q.sleeve_lamports - 1);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::SlippageExceeded));
    // Share floor above what the deposit buys.
    let ix = env.mint_ix(&l, &s, &buyer.pubkey(), &deposits, q.gross_shares, q.sleeve_lamports);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::SlippageExceeded));
    // Below the 0.1 SOL minimum: 1/1000 of the vaults is ~0.0012 SOL of value.
    let tiny: Vec<u64> = vaults.iter().map(|v| v / 1000).collect();
    let ix = env.mint_ix(&l, &s, &buyer.pubkey(), &tiny, 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::BelowMinimum));
    // Wrong deposit count.
    let ix = env.mint_ix(&l, &s, &buyer.pubkey(), &deposits[..2], 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::ComponentCountMismatch));

    // Pause blocks mint.
    env.set_paused(true);
    let ix = env.mint_ix(&l, &s, &buyer.pubkey(), &deposits, 0, q.sleeve_lamports);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::Paused));
    env.set_paused(false);

    // Gate: window closes, then a reopen schedule lets buys through again.
    let now = env.now();
    let l2 = env.launch_with(2, 5, |a| {
        a.gate = MintGate::WindowUntil { close_ts: now + 100 };
        a.schedule = Some(ReopenSchedule { anchor_ts: now + 1000, period_s: 1000, open_s: 100 });
    });
    let plan = SeedPlan::default_for(&l2);
    let s2 = env.seed(&l2, &plan);
    let v2: Vec<u64> = l2.mints.iter().map(|m| env.token_amount(&Env::ata(&l2.basket, m))).collect();
    let d2: Vec<u64> = v2.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l2, &d2);
    env.mint(&l2, &s2, &buyer, &d2, 0, u64::MAX); // inside the window
    env.warp(200); // window closed, schedule not yet anchored
    env.fund_components_for(&buyer.pubkey(), &l2, &d2);
    let ix = env.mint_ix(&l2, &s2, &buyer.pubkey(), &d2, 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::MintGateClosed));
    env.warp(850); // t = now+1050: inside the first reopen window
    env.mint(&l2, &s2, &buyer, &d2, 0, u64::MAX);
    env.warp(100); // t = now+1150: window over
    env.fund_components_for(&buyer.pubkey(), &l2, &d2);
    let ix = env.mint_ix(&l2, &s2, &buyer.pubkey(), &d2, 0, u64::MAX);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::MintGateClosed));
}

#[test]
fn mint_rejects_foreign_or_partial_component_sets() {
    let mut env = Env::initialized();
    let (l, s, buyer) = seeded(&mut env, 6);
    let (l_other, _s_other, _) = seeded(&mut env, 7);
    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);

    // Only two of three components passed.
    let mut ix = env.mint_ix(&l, &s, &buyer.pubkey(), &deposits, 0, u64::MAX);
    let fixed = ix.accounts.len() - 3 * 4;
    ix.accounts.truncate(fixed + 2 * 4);
    let mut short = ix.clone();
    short.data = basket::instruction::Mint {
        args: MintArgs { deposits: deposits[..2].to_vec(), min_shares_out: 0, max_sleeve_lamports: u64::MAX },
    }
    .data();
    env.send_expect_err(&[short], &buyer, &[], err(BasketError::ComponentCountMismatch));

    // A position from another basket.
    let mut ix = env.mint_ix(&l, &s, &buyer.pubkey(), &deposits, 0, u64::MAX);
    let other = component_metas(&l_other.basket, &l_other.share_mint, &buyer.pubkey(), &l_other.book[..1]);
    let start = ix.accounts.len() - 3 * 4;
    ix.accounts.splice(start..start + 4, other);
    env.send_expect_err(&[ix], &buyer, &[], err(BasketError::ComponentMismatch));
}

/// 23 fixed accounts + 4 per component against the 64-account lock limit:
/// N ≤ 10 in one transaction (seed caps at 9, so 9 is the largest seeded here).
#[test]
fn mint_account_counts_and_cu_by_size() {
    let mut env = Env::initialized();
    for (i, n) in [2usize, 5, 8, 9].iter().enumerate() {
        let l = env.launch_fixed(*n, 20 + i as u64);
        let plan = SeedPlan::default_for(&l);
        let s = env.seed(&l, &plan);
        let buyer = env.fund(100 * LAMPORTS);
        let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
        let deposits: Vec<u64> = vaults.iter().map(|v| v / 2).collect();
        env.fund_components_for(&buyer.pubkey(), &l, &deposits);
        let ix = env.mint_ix(&l, &s, &buyer.pubkey(), &deposits, 0, u64::MAX);
        let accounts = ix.accounts.len();
        let m = env.mint(&l, &s, &buyer, &deposits, 0, u64::MAX);
        println!("mint N={n}: {accounts} accounts, {} CU", m.cu);
    }
}

/// Fee shares the default seed plan put in the FeeVault.
fn plan_fee(l: &Launched) -> u64 {
    SeedPlan::default_for(l).initial_shares / 100
}
