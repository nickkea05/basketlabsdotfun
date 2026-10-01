//! `close_positions` + `close_basket` + `close_fee_vault` (§5, change order
//! §2.2 / co9): the keeper's zero-supply close crank. Once every holder has
//! redeemed, the fees are swept (the creator's share redeemed too), the
//! backstop is withdrawn and closed, and the basket has been idle for
//! `close_idle_s`, the keeper closes the component positions (vault dust to
//! the treasury), then the basket: tight withdrawn and closed, leftover
//! shares burned, sleeve SOL to the treasury, deposit minus spent rent plus
//! every account rent back to the deployer. The FeeVault closes last.

mod common;

use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

/// Everyone out: the only holder (the deployer) redeems, the fees are swept
/// and the creator's cut redeemed again until nothing is outstanding.
fn wind_down_holders(env: &mut Env, l: &Launched) {
    let holder = l.payer.insecure_clone();
    let caller = env.fund(LAMPORTS);
    for _ in 0..6 {
        let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
        if shares > 0 {
            // With a live backstop the view's H can sit a few base units
            // under the last holder's balance; redeem clamps to "everything".
            env.redeem(l, &holder, shares);
        }
        if env.fee_shares(l) == 0 {
            break;
        }
        env.sweep_fees(l, &caller);
    }
    assert_eq!(env.fee_shares(l), 0, "fee shares left");
    assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)), 0);
    assert!(env.holder_shares(l) <= CLOSE_DUST_SHARES, "holder shares {}", env.holder_shares(l));
}

#[test]
fn closes_an_idle_fully_redeemed_basket_and_refunds_the_deposit() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(3, 1);
    env.seed(&l, &SeedPlan::default_for(&l));
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    let treasury = env.treasury.pubkey();
    let tight = l.tight_position(&env);

    wind_down_holders(&mut env, &l);
    // Every pro-rata withdrawal took the treasury shares with it: nothing left.
    assert!(env.mint_supply(&l.share_mint) <= CLOSE_DUST_SHARES);

    // Not idle yet.
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));
    env.warp(DEFAULT_CLOSE_IDLE_S);
    // Not a keeper.
    let stranger = env.fund(LAMPORTS);
    let ix = env.close_positions_ix(&l, &stranger.pubkey(), &l.mints);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    // Basket before positions.
    let ix = env.close_basket_ix(&l, &k);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PositionsNotClosed));

    let payer_before = env.lamports(&l.payer.pubkey());
    let dust: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();

    // Positions in two chunks.
    let ix = env.close_positions_ix(&l, &k, &l.mints[..2]);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("close_positions(2) CU: {}", m.compute_units_consumed);
    assert_eq!(env.load::<Basket>(&l.basket).position_count, 1);
    let ix = env.close_positions_ix(&l, &k, &l.mints[2..]);
    env.send_ok(&[ix], &keeper, &[]);
    for (m, d) in l.mints.iter().zip(&dust) {
        assert!(env.position(&l, m).is_none());
        assert!(env.account(&Env::ata(&l.basket, m)).is_none());
        assert_eq!(env.token_amount(&Env::ata(&treasury, m)), *d, "dust to the treasury");
    }

    let b = l.basket_state(&env);
    assert_eq!(b.deposit_lamports, LAMPORTS);
    let spent = b.deposit_spent_lamports;
    println!("deposit spent (pool + tight bin arrays): {spent}");
    assert!(spent > 150_000_000 && spent < 200_000_000, "pool 34.6M + two bin arrays 142.9M");

    let ix = env.close_basket_ix(&l, &k);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("close_basket CU: {}", m.compute_units_consumed);
    assert!(env.account(&l.basket).is_none());
    assert!(env.account(&tight).is_none(), "tight position closed");
    assert!(env.account(&Env::ata(&l.basket, &l.share_mint)).is_none());
    assert!(env.account(&Env::ata(&l.basket, &WSOL)).is_none());
    assert!(env.account(&Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint)).is_none());
    assert!(env.mint_supply(&l.share_mint) <= CLOSE_DUST_SHARES, "supply {}", env.mint_supply(&l.share_mint));
    // Every pro-rata redeem took its SOL with it; what the treasury gets here
    // is the rounding dust of the tight position.
    assert!(env.account(&Env::ata(&treasury, &WSOL)).is_some(), "sleeve dust lands on the treasury's wSOL ATA");
    assert!(env.token_amount(&Env::ata(&treasury, &WSOL)) < 10_000);
    // The pool and its bin arrays stay (non-refundable); everything else
    // comes back: deposit − spent, Basket rent, position rents, ATAs.
    let refund = env.lamports(&l.payer.pubkey()) - payer_before;
    println!("refund to the deployer at close: {refund}");
    assert!(refund > LAMPORTS - spent, "refund {refund} must cover the deposit remainder {}", LAMPORTS - spent);
    assert!(refund < LAMPORTS - spent + 100_000_000, "refund {refund} is the deposit remainder plus rents");

    // The FeeVault stays for settle_fees; nothing is pending here, so it closes.
    let fee_vault = fee_vault_pda(&l.share_mint);
    assert!(env.account(&fee_vault).is_some());
    let fv = env.fee_vault(&l);
    assert_eq!((fv.unrouted_lamports, fv.creator_owed_lamports, fv.unsettled_shares), (0, 0, 0));
    let before = env.lamports(&l.payer.pubkey());
    let ix = env.close_fee_vault_ix(&l, &k);
    env.send_ok(&[ix], &keeper, &[]);
    assert!(env.account(&fee_vault).is_none());
    assert!(env.lamports(&l.payer.pubkey()) > before);
}

#[test]
fn close_waits_for_the_backstop_to_be_wound_down() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 2);
    env.seed(&l, &SeedPlan::default_for(&l));
    env.place_and_fund_backstop(&l);
    let backstop = env.backstop_position_for(&l);
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    let payer_before = env.lamports(&l.payer.pubkey());
    let keeper_before = env.lamports(&k);

    wind_down_holders(&mut env, &l);
    // The 48 h window closes, then `close_idle_s` passes: closable except
    // for the live backstop.
    env.warp(DEFAULT_MINT_WINDOW_S + DEFAULT_CLOSE_IDLE_S + 1);
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));

    env.withdraw_backstop_all(&l);
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BackstopState));
    env.close_backstop(&l);
    assert!(env.account(&backstop).is_none());
    assert_eq!(l.basket_state(&env).backstop_state, BACKSTOP_CLOSED);

    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_ok(&[ix], &keeper, &[]);
    let b = l.basket_state(&env);
    let spent = b.deposit_spent_lamports;
    println!("deposit spent with a 4-array backstop: {spent}");
    assert!(spent > 300_000_000 && spent < 500_000_000, "pool + tight arrays + backstop arrays");
    let ix = env.close_basket_ix(&l, &k);
    env.send_ok(&[ix], &keeper, &[]);
    assert!(env.account(&l.basket).is_none());
    let refund = env.lamports(&l.payer.pubkey()) - payer_before;
    println!("refund with backstop: {refund}");
    assert!(refund > LAMPORTS - spent && refund < LAMPORTS - spent + 100_000_000);
    // The keeper got its backstop position rent back and only paid tx fees.
    let keeper_spent = keeper_before.saturating_sub(env.lamports(&k));
    assert!(keeper_spent < 1_000_000, "keeper spent {keeper_spent}");
}

#[test]
fn a_basket_with_holders_or_activity_is_not_closable() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 3);
    env.seed(&l, &SeedPlan::default_for(&l));
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    env.warp(DEFAULT_CLOSE_IDLE_S + 1);
    // Idle, but the seed buyer still holds shares.
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));

    // The holder redeems, but the fee shares are still outstanding.
    let holder = l.payer.insecure_clone();
    let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
    env.redeem(&l, &holder, shares);
    env.warp(DEFAULT_CLOSE_IDLE_S + 1);
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));

    // Redeem resets the idle clock.
    wind_down_holders(&mut env, &l);
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));
    env.warp(DEFAULT_CLOSE_IDLE_S);
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.close_basket_ix(&l, &k);
    env.send_ok(&[ix], &keeper, &[]);
    assert!(env.account(&l.basket).is_none());

    // The FeeVault refuses to close while the basket is open (another basket).
    let l2 = env.launch_fixed(2, 4);
    env.seed(&l2, &SeedPlan::default_for(&l2));
    let ix = env.close_fee_vault_ix(&l2, &k);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));
}

#[test]
fn closes_a_basket_that_was_never_seeded() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 5);
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    let payer_before = env.lamports(&l.payer.pubkey());
    let spent = l.basket_state(&env).deposit_spent_lamports;
    assert!(spent > 30_000_000 && spent < 40_000_000, "only the pool rent so far: {spent}");
    env.warp(DEFAULT_CLOSE_IDLE_S);
    let ix = env.close_positions_ix(&l, &k, &l.mints);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.close_basket_ix(&l, &k);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("close_basket(unseeded) CU: {}", m.compute_units_consumed);
    assert!(env.account(&l.basket).is_none());
    for m in &l.mints {
        assert!(env.position(&l, m).is_none());
    }
    let refund = env.lamports(&l.payer.pubkey()) - payer_before;
    assert!(refund > LAMPORTS - spent && refund < LAMPORTS - spent + 50_000_000, "refund {refund}");
    // No FeeVault was ever created.
    assert!(env.account(&fee_vault_pda(&l.share_mint)).is_none());
}
