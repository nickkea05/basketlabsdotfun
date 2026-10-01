//! `settle_fees` (change order Q1): the keeper sells one of the FeeVault's
//! component ATAs to SOL through an allow-listed venue and routes the SOL to
//! BUYBACK / team / PRIZE. The swap output is whatever the venue returns; the
//! program only checks the allow-list, the slippage floor the keeper set and
//! that the inner instruction touched no other FeeVault token account.

mod common;

use anchor_lang::prelude::*;
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

/// A swept basket whose FeeVault holds components, and a venue for component 0.
fn swept(env: &mut Env, nonce: u64) -> (Launched, VenuePool) {
    let (l, _) = env.launch_and_seed(2, nonce);
    let caller = env.fund(LAMPORTS);
    env.sweep_fees(&l, &caller);
    env.allow_dlmm_swaps();
    let venue = env.create_venue_pool(&l.mints[0], &WSOL, 10_000_000_000, 10 * LAMPORTS);
    (l, venue)
}

#[test]
fn settle_swaps_a_component_to_sol_and_routes_three_ways() {
    let mut env = Env::initialized();
    let (l, venue) = swept(&mut env, 1);
    let fee_vault = fee_vault_pda(&l.share_mint);
    let in_ata = Env::ata(&fee_vault, &l.mints[0]);
    let held = env.token_amount(&in_ata);
    assert!(held > 0);
    let keeper = env.keeper.insecure_clone();
    let before = lines(&env);
    let creator_before = env.lamports(&l.creator.pubkey());
    let keeper_before = env.lamports(&keeper.pubkey());

    let inner = env.venue_swap_ix(&venue, &fee_vault, &l.mints[0], held);
    let ix = env.settle_fees_ix(&l, &keeper.pubkey(), &l.mints[0], held, 1, &inner);
    let m = env.send_ok(&[ix], &keeper, &[]);
    eprintln!("settle_fees CU: {}", m.compute_units_consumed);

    assert_eq!(env.token_amount(&in_ata), 0, "the whole component ATA was sold");
    let after = lines(&env);
    let routed = (after.buyback - before.buyback) + (after.team - before.team) + (after.prize - before.prize);
    // 1:1 venue less its 0.3 % fee and the walk down the bins.
    assert!(routed > held * 95 / 100 && routed < held, "routed {routed} for {held} sold");
    let fees = l.basket_state(&env).fees;
    let total = (fees.buyback_split_bps + fees.team_split_bps + fees.prize_split_bps) as u64;
    let bb = routed * fees.buyback_split_bps as u64 / total;
    let tm = routed * fees.team_split_bps as u64 / total;
    assert_eq!((after.buyback - before.buyback, after.team - before.team, after.prize - before.prize), (bb, tm, routed - bb - tm));
    assert_eq!(env.lamports(&l.creator.pubkey()), creator_before, "the creator was paid in shares already");
    assert_eq!(env.fee_vault_free_lamports(&l), 0);
    assert!(env.account(&Env::ata(&fee_vault, &WSOL)).is_none(), "temporary wSOL account closed");
    let spent = keeper_before - env.lamports(&keeper.pubkey());
    assert!(spent < 20_000, "keeper spent {spent}");
    let fv = env.fee_vault(&l);
    assert!(fv.routed_buyback_lamports >= bb && fv.routed_prize_lamports >= routed - bb - tm);
}

#[test]
fn settle_checks_keeper_allowlist_slippage_and_accounts() {
    let mut env = Env::initialized();
    let (l, venue) = swept(&mut env, 2);
    let fee_vault = fee_vault_pda(&l.share_mint);
    let in_ata = Env::ata(&fee_vault, &l.mints[0]);
    let held = env.token_amount(&in_ata);
    let keeper = env.keeper.insecure_clone();
    let inner = env.venue_swap_ix(&venue, &fee_vault, &l.mints[0], held / 2);

    // Not the keeper.
    let stranger = env.fund(LAMPORTS);
    let ix = env.settle_fees_ix(&l, &stranger.pubkey(), &l.mints[0], held / 2, 1, &inner);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    // Venue not on the allow-list.
    env.update_config(ConfigUpdate { swap_programs: Some(vec![]), ..Default::default() });
    let ix = env.settle_fees_ix(&l, &keeper.pubkey(), &l.mints[0], held / 2, 1, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SwapProgramNotAllowed));
    env.allow_dlmm_swaps();
    // Slippage floor above what the venue pays.
    let ix = env.settle_fees_ix(&l, &keeper.pubkey(), &l.mints[0], held / 2, held, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SlippageExceeded));
    // Selling wSOL "to SOL" makes no sense.
    env.create_ata(&fee_vault, &WSOL);
    let ix = env.settle_fees_ix(&l, &keeper.pubkey(), &WSOL, 1, 1, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::InvalidArgument));
    // The inner instruction may not touch another FeeVault token account.
    let other_ata = Env::ata(&fee_vault, &l.mints[1]);
    let mut smuggled = inner.clone();
    smuggled.accounts.push(anchor_lang::solana_program::instruction::AccountMeta::new(other_ata, false));
    let ix = env.settle_fees_ix(&l, &keeper.pubkey(), &l.mints[0], held / 2, 1, &smuggled);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::UnexpectedAccountInSwap));
    // Paused.
    env.set_paused(true);
    let ix = env.settle_fees_ix(&l, &keeper.pubkey(), &l.mints[0], held / 2, 1, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::Paused));
    env.set_paused(false);
    // Partial amounts work and leave the rest in the ATA.
    let ix = env.settle_fees_ix(&l, &keeper.pubkey(), &l.mints[0], held / 2, 1, &inner);
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.token_amount(&in_ata), held - held / 2);
}
