//! `sweep_fees` / `sweep_fees_components` (change order Q1): the mint and
//! redeem fees withheld as shares in the FeeVault are split by the schedule
//! frozen into the basket — the creator's line is paid in shares, the rest
//! is redeemed in kind: the tight SOL leg is routed to BUYBACK / team /
//! PRIZE at once, the components land in the FeeVault's own ATAs for
//! `settle_fees`. Nothing here prices anything.

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

struct Lines {
    buyback: u64,
    team: u64,
    prize: u64,
}

fn lines(env: &Env) -> Lines {
    let config = env.config();
    Lines { buyback: env.lamports(&buyback_vault_pda()), team: env.lamports(&config.team_wallet), prize: env.lamports(&prize_vault_pda()) }
}

fn protocol_split(fees: &FeeSchedule, amount: u64) -> (u64, u64, u64) {
    let total = (fees.buyback_split_bps + fees.team_split_bps + fees.prize_split_bps) as u64;
    let buyback = amount * fees.buyback_split_bps as u64 / total;
    let team = amount * fees.team_split_bps as u64 / total;
    (buyback, team, amount - buyback - team)
}

#[test]
fn sweep_pays_the_creator_in_shares_and_settles_the_rest_in_kind() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(3, 1);
    let b = l.basket_state(&env);
    let fees = b.fees;
    let fee_vault = fee_vault_pda(&l.share_mint);
    let creator = l.creator.pubkey();

    // Shares in the FeeVault: the 1 % seed mint fee plus a mint's fee.
    let plan = SeedPlan::default_for(&l);
    assert_eq!(env.fee_shares(&l), plan.initial_shares / 100);
    let deposits: Vec<u64> = env.vaults(&l).iter().map(|v| v / 4).collect();
    let q = env.mint_quote(&l, &deposits);
    env.mint_from_new_buyer(&l, &deposits);
    let shares_total = env.fee_shares(&l);
    assert_eq!(shares_total, plan.initial_shares / 100 + q.gross_shares * DEFAULT_MINT_FEE_BPS as u64 / BPS_TOTAL as u64);

    let h = env.holder_shares(&l);
    let vaults = env.vaults(&l);
    let tight_before = env.tight_amounts(&l);
    let idle_y = env.idle_sol(&l);
    let idle_x = env.idle_shares(&l);
    let supply_before = env.mint_supply(&l.share_mint);
    let before = lines(&env);
    let creator_sol_before = env.lamports(&creator);
    // The creator is the deployer, who made the first buy: they hold shares already.
    let creator_shares_before = env.token_amount(&Env::ata(&creator, &l.share_mint));
    let caller = env.fund(LAMPORTS);
    let caller_before = env.lamports(&caller.pubkey());

    let cu = env.sweep_fees(&l, &caller);
    eprintln!("sweep_fees CU: {cu}");

    // Creator: 20 % in shares, nothing in SOL.
    let creator_shares = shares_total * fees.creator_split_bps as u64 / BPS_TOTAL as u64;
    assert_eq!(env.token_amount(&Env::ata(&creator, &l.share_mint)), creator_shares_before + creator_shares);
    assert_eq!(env.lamports(&creator), creator_sol_before);
    // The rest was burned and redeemed in kind at net / H.
    let net = shares_total - creator_shares;
    assert_eq!(env.fee_shares(&l), 0);
    for (i, mint) in l.mints.iter().enumerate() {
        let expected = (vaults[i] as u128 * net as u128 / h as u128) as u64;
        assert_eq!(env.token_amount(&Env::ata(&fee_vault, mint)), expected, "component {i} in the FeeVault's ATA");
        assert_eq!(env.token_amount(&Env::ata(&l.basket, mint)), vaults[i] - expected);
    }
    // SOL leg: net / H of tight SOL + idle SOL, routed 50 / 25 / 5 (scaled to the three lines).
    let pool_sol = tight_before.amount_y + idle_y;
    let sol = (pool_sol as u128 * net as u128 / h as u128) as u64;
    let after = lines(&env);
    let routed = (after.buyback - before.buyback) + (after.team - before.team) + (after.prize - before.prize);
    assert!(routed <= sol && sol - routed <= 70, "routed {routed} vs pro-rata {sol}");
    let (bb, tm, pz) = protocol_split(&fees, routed);
    assert_eq!((after.buyback - before.buyback, after.team - before.team, after.prize - before.prize), (bb, tm, pz));
    assert_eq!(env.fee_vault_free_lamports(&l), 0, "nothing left on the FeeVault");
    // Withdrawn treasury shares burned along with the settled fee shares.
    let tight = env.tight_amounts(&l);
    let x_out = tight_before.amount_x - tight.amount_x;
    let idle_x_share = (idle_x as u128 * net as u128 / h as u128) as u64;
    assert_eq!(env.mint_supply(&l.share_mint), supply_before - net - x_out - idle_x_share);
    assert!(env.holder_shares(&l).abs_diff(h - net) <= 1);
    // Bookkeeping closed out.
    let b = l.basket_state(&env);
    assert_eq!(b.pending_redeem_shares, 0);
    assert!(env.account(&redemption_pda(&l.share_mint, &fee_vault)).is_none(), "redemption closed");
    let fv = env.fee_vault(&l);
    assert_eq!((fv.swept_creator_shares, fv.unsettled_shares), (creator_shares, 0));
    assert_eq!((fv.routed_buyback_lamports, fv.routed_team_lamports, fv.routed_prize_lamports), (bb, tm, pz));
    // The caller paid tx fees plus the creator's share ATA (theirs to keep);
    // the temporary wSOL account's rent came back.
    let spent = caller_before - env.lamports(&caller.pubkey());
    assert!(spent < 3 * 2_100_000 + 50_000, "caller spent {spent} (the FeeVault's component ATAs)");
    assert!(env.account(&Env::ata(&fee_vault, &WSOL)).is_none());
}

#[test]
fn sweep_needs_something_to_sweep_and_only_moves_what_accrued() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 2);
    let caller = env.fund(LAMPORTS);
    let creator = l.creator.pubkey();
    env.sweep_fees(&l, &caller);
    let creator_shares_1 = env.token_amount(&Env::ata(&creator, &l.share_mint));
    let fv1 = env.fee_vault(&l);

    // Nothing new: refused (the Redemption account would be re-created for nothing).
    let ix = env.sweep_fees_ix(&l, &caller.pubkey());
    env.send_expect_err(&[ix], &caller, &[], err(BasketError::ZeroAmount));

    // A redeem adds 0.25 % of the redeemed shares; only that is swept.
    let holder = l.payer.insecure_clone();
    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let fee = gross * DEFAULT_REDEEM_FEE_BPS as u64 / BPS_TOTAL as u64;
    env.redeem(&l, &holder, gross);
    assert_eq!(env.fee_shares(&l), fee);
    let b = l.basket_state(&env);
    // (The holder is the creator: they just redeemed half their shares.)
    let creator_shares_2 = env.token_amount(&Env::ata(&creator, &l.share_mint));
    assert_eq!(creator_shares_2, creator_shares_1 - gross);
    env.sweep_fees(&l, &caller);
    let creator_shares = fee * b.fees.creator_split_bps as u64 / BPS_TOTAL as u64;
    assert_eq!(env.token_amount(&Env::ata(&creator, &l.share_mint)), creator_shares_2 + creator_shares);
    let fv2 = env.fee_vault(&l);
    assert_eq!(fv2.swept_creator_shares, fv1.swept_creator_shares + creator_shares);
    assert_eq!(env.fee_shares(&l), 0);
}

#[test]
fn sweep_uses_the_schedule_frozen_into_the_basket() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 3);
    // Admin raises the base tier for future baskets to 30 %.
    env.update_config(ConfigUpdate { creator_tier_bps: Some([3_000; TIER_COUNT]), ..Default::default() });
    let (l2, _) = env.launch_and_seed(2, 4);
    assert_eq!(l.basket_state(&env).fees.creator_split_bps, 2_000);
    assert_eq!(l2.basket_state(&env).fees.creator_split_bps, 3_000);

    let caller = env.fund(LAMPORTS);
    for (basket, bps) in [(&l, 2_000u64), (&l2, 3_000)] {
        let total = env.fee_shares(basket);
        let before = env.token_amount(&Env::ata(&basket.creator.pubkey(), &basket.share_mint));
        env.sweep_fees(basket, &caller);
        assert_eq!(env.token_amount(&Env::ata(&basket.creator.pubkey(), &basket.share_mint)) - before, total * bps / BPS_TOTAL as u64);
    }
}

#[test]
fn sweep_with_a_large_book_pays_components_in_chunks() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(9, 5);
    let fee_vault = fee_vault_pda(&l.share_mint);
    let caller = env.fund(LAMPORTS);
    let shares_total = env.fee_shares(&l);
    let h = env.holder_shares(&l);
    let vaults = env.vaults(&l);
    let b = l.basket_state(&env);
    let net = shares_total - shares_total * b.fees.creator_split_bps as u64 / BPS_TOTAL as u64;

    let m = env.send_ok(&[env.sweep_fees_ix(&l, &caller.pubkey())], &caller, &[]);
    eprintln!("sweep_fees (N=9) CU: {}", m.compute_units_consumed);
    let red: Redemption = env.load(&redemption_pda(&l.share_mint, &fee_vault));
    assert_eq!((red.holder, red.shares, red.position_count, red.paid_count), (fee_vault, net, 9, 0));
    assert_eq!(l.basket_state(&env).pending_redeem_shares, net);
    assert_eq!(env.fee_vault(&l).unsettled_shares, net);
    // The burned shares stay in the denominator until the components are paid.
    assert!(env.holder_shares(&l).abs_diff(h) <= 1);
    // A second sweep cannot start while this one is open.
    let ix = env.sweep_fees_ix(&l, &caller.pubkey());
    assert!(env.send(&[ix], &caller, &[]).is_err());

    let m = env.send_ok(&[env.sweep_fees_components_ix(&l, &caller.pubkey(), &l.book[..POSITIONS_PER_TX])], &caller, &[]);
    eprintln!("sweep_fees_components (8) CU: {}", m.compute_units_consumed);
    let ix = env.sweep_fees_components_ix(&l, &caller.pubkey(), &l.book[7..8]);
    env.send_expect_err(&[ix], &caller, &[], err(BasketError::DuplicateMint));
    env.send_ok(&[env.sweep_fees_components_ix(&l, &caller.pubkey(), &l.book[POSITIONS_PER_TX..])], &caller, &[]);
    assert!(env.account(&redemption_pda(&l.share_mint, &fee_vault)).is_none());
    assert_eq!(l.basket_state(&env).pending_redeem_shares, 0);
    assert_eq!(env.fee_vault(&l).unsettled_shares, 0);
    for (i, mint) in l.mints.iter().enumerate() {
        let expected = (vaults[i] as u128 * net as u128 / h as u128) as u64;
        assert!(env.token_amount(&Env::ata(&fee_vault, mint)).abs_diff(expected) <= 1, "component {i}");
    }
    assert!(env.holder_shares(&l).abs_diff(h - net) <= 1);
}

#[test]
fn sweep_rejects_wrong_creator_or_team_wallet() {
    let mut env = Env::initialized();
    let (l, _) = env.launch_and_seed(2, 6);
    let caller = env.fund(LAMPORTS);
    let good = env.sweep_fees_ix(&l, &caller.pubkey());
    let stranger = Pubkey::new_unique();

    let mut ix = good.clone();
    let idx = ix.accounts.iter().position(|m| m.pubkey == l.creator.pubkey()).unwrap();
    ix.accounts[idx].pubkey = stranger;
    ix.accounts[idx + 1].pubkey = Env::ata(&stranger, &l.share_mint);
    env.send_expect_err(&[ix], &caller, &[], err(BasketError::Unauthorized));

    let mut ix = good.clone();
    let idx = ix.accounts.iter().position(|m| m.pubkey == env.config().team_wallet).unwrap();
    ix.accounts[idx].pubkey = stranger;
    env.send_expect_err(&[ix], &caller, &[], err(BasketError::Unauthorized));

    // Anyone may sweep, paused or not.
    env.set_paused(true);
    env.sweep_fees(&l, &caller);
}
