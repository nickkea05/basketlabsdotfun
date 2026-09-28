//! `claim_pool_fees`: anyone cranks the basket's DAMM v2 position fees
//! (`collect_fee_mode = OnlyB`, so SOL) into the FeeVault, held as native
//! lamports until `sweep_fees` splits them (§4, §5).

mod common;

use basket::error::BasketError;
use common::*;
use solana_signer::Signer as _;

fn seeded(env: &mut Env, n: usize, nonce: u64) -> (Launched, SeededBasket) {
    let l = env.launch_fixed(n, nonce);
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    (l, s)
}

#[test]
fn claims_swap_fees_into_the_fee_vault_as_sol() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 3, 1);
    let fee_vault = fee_vault_pda(&l.share_mint);
    let free_before = env.fee_vault_free_lamports(&l);
    assert_eq!(free_before, 0, "FeeVault starts with rent only");

    // A round trip through the pool: 1 SOL in, shares back out.
    let trader = env.fund(10 * LAMPORTS);
    env.buy_shares(&l, &s, &trader, LAMPORTS);
    let bought = env.token_amount(&Env::ata(&trader.pubkey(), &l.share_mint));
    assert!(bought > 0);
    env.sell_shares(&l, &s, &trader, bought);
    let sol_back = env.token_amount(&Env::ata(&trader.pubkey(), &WSOL));

    // Fixed/index pool fee is 0.3%; Meteora keeps 20% of it. Fees on the
    // buy are taken on the SOL in, on the sell on the SOL out.
    let lp_share = |sol: u64| sol * 30 / 10_000 * 80 / 100;
    let expected = lp_share(LAMPORTS) + lp_share(sol_back);

    let caller = env.fund(LAMPORTS);
    let caller_before = env.lamports(&caller.pubkey());
    let claimed = env.claim_pool_fees(&l, &s, &caller);
    assert!(claimed.abs_diff(expected) * 100 <= expected, "claimed {claimed} vs expected {expected}");
    assert_eq!(env.fee_vault_free_lamports(&l), claimed, "held as native lamports in the FeeVault");
    // Caller only paid the tx fee: the temporary wSOL account's rent came back.
    let spent = caller_before - env.lamports(&caller.pubkey());
    assert!(spent < 20_000, "caller spent {spent}");
    assert!(env.account(&Env::ata(&fee_vault, &WSOL)).is_none(), "no wSOL account left behind");
    // Nothing on the share side (OnlyB).
    let fee_shares_before = env.token_amount(&Env::ata(&fee_vault, &l.share_mint));

    // No new trades: claiming again is a no-op, not an error.
    let again = env.claim_pool_fees(&l, &s, &caller);
    assert_eq!(again, 0);
    assert_eq!(env.token_amount(&Env::ata(&fee_vault, &l.share_mint)), fee_shares_before);
}

#[test]
fn claim_is_permissionless_and_ignores_pause() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 2, 2);
    let trader = env.fund(10 * LAMPORTS);
    env.buy_shares(&l, &s, &trader, LAMPORTS);
    env.set_paused(true);
    let anyone = env.fund(LAMPORTS);
    assert!(env.claim_pool_fees(&l, &s, &anyone) > 0);
}

#[test]
fn claim_checks_the_position_belongs_to_the_basket() {
    let mut env = Env::initialized();
    let (l1, s1) = seeded(&mut env, 2, 3);
    let (l2, s2) = seeded(&mut env, 2, 4);
    let trader = env.fund(10 * LAMPORTS);
    env.buy_shares(&l1, &s1, &trader, LAMPORTS);
    // Basket 2's accounts with basket 1's pool: the has_one checks fail.
    let mut ix = env.claim_pool_fees_ix(&l2, &s2, &trader.pubkey());
    let good = env.claim_pool_fees_ix(&l1, &s1, &trader.pubkey());
    ix.accounts[9] = good.accounts[9].clone(); // pool
    ix.accounts[10] = good.accounts[10].clone(); // pool_position
    env.send_expect_err(&[ix], &trader, &[], err(BasketError::PoolMismatch));
}
