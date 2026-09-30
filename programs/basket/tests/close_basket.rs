//! `close_positions` + `close_basket` (§5): the keeper's zero-supply close
//! crank. Once every outside holder has redeemed (only the FeeVault's own
//! fee shares are left) and the basket has been idle for `close_idle_s`,
//! the keeper closes the positions (vault dust to the treasury), then the
//! basket: sleeve drained to the treasury, leftover shares burned, FeeVault
//! and Basket closed, rent back to the original payer.

mod common;

use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

#[test]
fn closes_an_idle_fully_redeemed_basket() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(3, 1);
    let s = env.seed(&l, &SeedPlan::default_for(&l));
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    let holder = l.payer.insecure_clone();
    let treasury = env.treasury.pubkey();

    // The only holder redeems everything; the fee shares stay in the vault.
    let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
    env.redeem(&l, &s, &holder, shares);
    let fee_shares = env.token_amount(&Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint));
    assert!(fee_shares > 0);
    // Supply = fee shares + treasury shares still in the pool position.
    assert!(env.mint_supply(&l.share_mint) > fee_shares);
    // Holder shares now equal the fee stash up to pool rounding (+1 here).
    assert!(env.holder_shares(&l, &s) <= fee_shares + CLOSE_DUST_SHARES);

    // Not idle yet.
    let ix = env.close_positions_ix(&l, Some(&s), &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));
    env.warp(DEFAULT_CLOSE_IDLE_S);
    // Not a keeper.
    let stranger = env.fund(LAMPORTS);
    let ix = env.close_positions_ix(&l, Some(&s), &stranger.pubkey(), &l.mints);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    // Basket before positions.
    let ix = env.close_basket_ix(&l, Some(&s), &k);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PositionsNotClosed));

    let payer_before = env.lamports(&holder.pubkey());
    let dust: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    assert!(dust.iter().all(|d| *d > 0), "fee-share fraction of every vault remains");

    // Positions in two chunks.
    let ix = env.close_positions_ix(&l, Some(&s), &k, &l.mints[..2]);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("close_positions(2) CU: {}", m.compute_units_consumed);
    assert_eq!(env.load::<Basket>(&l.basket).position_count, 1);
    let ix = env.close_positions_ix(&l, Some(&s), &k, &l.mints[2..]);
    env.send_ok(&[ix], &keeper, &[]);
    for (m, d) in l.mints.iter().zip(&dust) {
        assert!(env.position(&l, m).is_none());
        assert!(env.account(&Env::ata(&l.basket, m)).is_none());
        assert_eq!(env.token_amount(&Env::ata(&treasury, m)), *d, "dust to the treasury");
    }

    let ix = env.close_basket_ix(&l, Some(&s), &k);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("close_basket CU: {}", m.compute_units_consumed);
    assert!(env.account(&l.basket).is_none());
    assert!(env.account(&fee_vault_pda(&l.share_mint)).is_none());
    assert!(env.account(&Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint)).is_none());
    assert!(env.account(&basket::damm::position(&s.position_nft_mint)).is_none(), "cp-amm position closed");
    assert!(env.account(&basket::damm::position_nft_account(&s.position_nft_mint)).is_none());
    // Everything burned except the base units the pool vault keeps by flooring.
    assert!(env.mint_supply(&l.share_mint) <= CLOSE_DUST_SHARES, "supply {}", env.mint_supply(&l.share_mint));
    assert!(env.token_amount(&Env::ata(&treasury, &WSOL)) > 0, "sleeve SOL to the treasury");
    assert!(env.lamports(&holder.pubkey()) > payer_before + 10_000_000, "rent back to the payer");
}

#[test]
fn a_basket_with_holders_or_activity_is_not_closable() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 2);
    let s = env.seed(&l, &SeedPlan::default_for(&l));
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    env.warp(DEFAULT_CLOSE_IDLE_S + 1);
    // Idle, but the seed buyer still holds shares.
    let ix = env.close_positions_ix(&l, Some(&s), &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));

    // Redeem resets the idle clock.
    let holder = l.payer.insecure_clone();
    let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
    env.redeem(&l, &s, &holder, shares);
    let ix = env.close_positions_ix(&l, Some(&s), &k, &l.mints);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::NotClosable));
    env.warp(DEFAULT_CLOSE_IDLE_S);
    let ix = env.close_positions_ix(&l, Some(&s), &k, &l.mints);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.close_basket_ix(&l, Some(&s), &k);
    env.send_ok(&[ix], &keeper, &[]);
    assert!(env.account(&l.basket).is_none());
}

#[test]
fn closes_a_basket_that_was_never_seeded() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 3);
    let keeper = env.keeper.insecure_clone();
    let k = keeper.pubkey();
    let payer_before = env.lamports(&l.payer.pubkey());
    env.warp(DEFAULT_CLOSE_IDLE_S);
    let ix = env.close_positions_ix(&l, None, &k, &l.mints);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.close_basket_ix(&l, None, &k);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("close_basket(unseeded) CU: {}", m.compute_units_consumed);
    assert!(env.account(&l.basket).is_none());
    for m in &l.mints {
        assert!(env.position(&l, m).is_none());
    }
    assert!(env.lamports(&l.payer.pubkey()) > payer_before);
}
