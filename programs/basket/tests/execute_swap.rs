//! `execute_swap` (§5): during an open rebalance window a keeper routes a
//! swap through an allow-listed venue with the basket as authority. The
//! program checks the venue, that the input leaves a current position's
//! vault within its cumulative sell cap, that the output lands in a target
//! position's vault at or above the keeper's minimum, and that the inner
//! instruction touches no other basket-owned account.

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

struct Setup {
    l: Launched,
    pool: ComponentPool,
}

/// Mirror basket on [A, B, C]; target drops C and puts its weight on B;
/// a C↔B pool exists to trade through.
fn setup(env: &mut Env, nonce: u64) -> Setup {
    env.allow_dlmm_swaps();
    let l = env.launch_mirror(3, nonce);
    env.seed(&l, &SeedPlan::default_for(&l));
    let pool = env.create_component_pool(&l.mints[2], &l.mints[1], 1_000_000_000_000);
    let target = book(&[l.mints[0], l.mints[1]], &[3_334, 6_666]);
    let keeper = env.keeper.insecure_clone();
    env.submit_book(&l, &keeper, &target, &[]);
    let _ = target;
    Setup { l, pool }
}

#[test]
fn keeper_swaps_a_leaving_position_into_a_target_one() {
    let mut env = Env::initialized();
    let Setup { l, pool, .. } = setup(&mut env, 1);
    let keeper = env.keeper.insecure_clone();
    let (c, b) = (l.mints[2], l.mints[1]);
    let vault_c = Env::ata(&l.basket, &c);
    let vault_b = Env::ata(&l.basket, &b);
    let c_before = env.token_amount(&vault_c);
    let b_before = env.token_amount(&vault_b);

    let amount_in = c_before / 2;
    let inner = env.component_swap_ix(&pool, &l.basket, &c, &b, amount_in);
    // 1:1 pool, 0.3% fee: expect ≈ 99.7% back.
    let min_out = amount_in * 99 / 100;
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, amount_in, min_out, &inner);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("execute_swap CU: {}", m.compute_units_consumed);

    assert_eq!(env.token_amount(&vault_c), c_before - amount_in);
    let got = env.token_amount(&vault_b) - b_before;
    assert!(got >= min_out && got < amount_in, "got {got}");
    let p: Position = env.load(&position_pda(&l.share_mint, &c));
    assert_eq!((p.rebalance_seq, p.sold), (1, amount_in));

    // Sell the rest (C leaves the book: cap is 100%).
    let inner = env.component_swap_ix(&pool, &l.basket, &c, &b, c_before - amount_in);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, c_before - amount_in, 0, &inner);
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.token_amount(&vault_c), 0);
}

#[test]
fn sell_cap_follows_the_weight_change() {
    let mut env = Env::initialized();
    env.allow_dlmm_swaps();
    let l = env.launch_mirror(3, 2);
    env.seed(&l, &SeedPlan::default_for(&l));
    let (a, b) = (l.mints[0], l.mints[1]);
    let pool = env.create_component_pool(&a, &b, 1_000_000_000_000);
    // A: 33.33% → 20%: may sell (3333−2000)/3333 = 40% (+5% tolerance) of the vault, cumulatively.
    let target = book(&[a, b, l.mints[2]], &[2_000, 4_667, 3_333]);
    let keeper = env.keeper.insecure_clone();
    env.submit_book(&l, &keeper, &target, &[]);
    let vault_a = Env::ata(&l.basket, &a);
    let start = env.token_amount(&vault_a);
    let cap = start as u128 * (4_000 + DEFAULT_REBALANCE_TOLERANCE_BPS as u128) / 10_000;

    // 30% then 10% pass; another 10% breaks the cumulative cap.
    for pct in [30u64, 10] {
        let amt = start * pct / 100;
        let inner = env.component_swap_ix(&pool, &l.basket, &a, &b, amt);
        let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &a, &b, amt, 0, &inner);
        env.send_ok(&[ix], &keeper, &[]);
    }
    let p: Position = env.load(&position_pda(&l.share_mint, &a));
    assert_eq!(p.sold, start * 40 / 100);
    let amt = start * 10 / 100;
    assert!(p.sold as u128 + amt as u128 > cap);
    let inner = env.component_swap_ix(&pool, &l.basket, &a, &b, amt);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &a, &b, amt, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SellCapExceeded));

    // Buying into A (weight going up elsewhere) is not capped: B → A.
    let inner = env.component_swap_ix(&pool, &l.basket, &b, &a, 1_000_000);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &b, &a, 1_000_000, 0, &inner);
    // B's weight rises, so only the tolerance may be sold from it — 1 unit is fine.
    env.send_ok(&[ix], &keeper, &[]);
}

#[test]
fn swap_guards() {
    let mut env = Env::initialized();
    let Setup { l, pool, .. } = setup(&mut env, 3);
    let keeper = env.keeper.insecure_clone();
    let (a, b, c) = (l.mints[0], l.mints[1], l.mints[2]);
    let amount = 1_000_000;

    // Not a keeper.
    let stranger = env.fund(LAMPORTS);
    let inner = env.component_swap_ix(&pool, &l.basket, &c, &b, amount);
    let ix = env.execute_swap_ix(&l, &stranger.pubkey(), &c, &b, amount, 0, &inner);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));

    // Slippage: minimum above what the pool pays.
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, amount, amount, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SlippageExceeded));

    // Output must land in a target position: C is leaving, so B → C is off-book.
    let inner_bc = env.component_swap_ix(&pool, &l.basket, &b, &c, amount);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &b, &c, amount, 0, &inner_bc);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::OffBook));

    // More may not leave the vault than the declared amount_in.
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, amount / 2, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::InvalidArgument));

    // Venue must be allow-listed.
    let ix = env.admin_ix(
        anchor_lang::InstructionData::data(&basket::instruction::UpdateConfig {
            update: ConfigUpdate { swap_programs: Some(vec![JUPITER_V6_ID]), ..Default::default() },
        }),
    );
    let admin = env.admin.insecure_clone();
    env.send_ok(&[ix], &admin, &[]);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, amount, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SwapProgramNotAllowed));
    env.allow_dlmm_swaps();

    // The inner instruction may not reference another basket-owned token
    // account (here: vault A smuggled into the account list).
    let mut inner_bad = inner.clone();
    inner_bad.accounts.push(AccountMeta::new(Env::ata(&l.basket, &a), false));
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, amount, 0, &inner_bad);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::UnexpectedAccountInSwap));

    // Window closed.
    env.warp(DEFAULT_REBALANCE_WINDOW_S + 1);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, amount, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::RebalanceWindowClosed));

    // No rebalance at all.
    let l2 = env.launch_mirror(2, 4);
    env.seed(&l2, &SeedPlan::default_for(&l2));
    let pool2 = env.create_component_pool(&l2.mints[0], &l2.mints[1], 1_000_000_000);
    let inner2 = env.component_swap_ix(&pool2, &l2.basket, &l2.mints[0], &l2.mints[1], amount);
    let ix = env.execute_swap_ix(&l2, &keeper.pubkey(), &l2.mints[0], &l2.mints[1], amount, 0, &inner2);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NoRebalance));
}

#[test]
fn the_basket_cannot_be_drained_through_the_venue_hook() {
    // A "swap" that is really a token transfer out of a vault: the token
    // program is not an allowed venue, and even if it were, the destination
    // check (output must grow the out vault) fails.
    let mut env = Env::initialized();
    let Setup { l, .. } = setup(&mut env, 5);
    let keeper = env.keeper.insecure_clone();
    let (b, c) = (l.mints[1], l.mints[2]);
    let thief = Env::ata(&keeper.pubkey(), &c);
    env.create_ata(&keeper.pubkey(), &c);
    let inner = anchor_spl::token::spl_token::instruction::transfer(
        &anchor_spl::token::spl_token::ID,
        &Env::ata(&l.basket, &c),
        &thief,
        &l.basket,
        &[],
        1_000_000,
    )
    .unwrap();
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &c, &b, 1_000_000, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SwapProgramNotAllowed));
    assert_eq!(env.token_amount(&thief), 0);
}
