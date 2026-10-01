//! BUYBACK vault (change order §2.5, Q8): the fee path credits the PDA with
//! SOL; `set_bskt_mint` is set once under the admin's timelock; the keeper
//! stages SOL onto the vault's wSOL account and `execute_buyback` swaps it
//! for $BSKT through an allow-listed venue and burns the output.

mod common;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::system_instruction;
use basket::error::BasketError;
use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer as _;

/// Admin drops `lamports` of fee SOL onto the BUYBACK PDA (what the fee
/// path does), creates the $BSKT mint and the venue to buy it with.
fn setup(env: &mut Env, lamports: u64) -> (Pubkey, VenuePool) {
    let admin = env.admin.insecure_clone();
    let ix = system_instruction::transfer(&admin.pubkey(), &buyback_vault_pda(), lamports);
    env.send_ok(&[ix], &admin, &[]);
    let bskt = env.create_mint(6, &admin.pubkey());
    env.allow_dlmm_swaps();
    // 1 BSKT base unit per lamport at the active bin.
    let venue = env.create_venue_pool(&bskt, &WSOL, 20_000_000_000, 20 * LAMPORTS);
    (bskt, venue)
}

fn keeper(env: &Env) -> Keypair {
    env.keeper.insecure_clone()
}

#[test]
fn nothing_moves_until_the_bskt_mint_is_set_and_then_only_once() {
    let mut env = Env::initialized();
    let (bskt, _venue) = setup(&mut env, LAMPORTS);
    let keeper = keeper(&env);
    env.create_ata(&buyback_vault_pda(), &WSOL);

    assert_eq!(env.config().bskt_mint, None);
    let ix = env.stage_buyback_ix(&keeper.pubkey(), LAMPORTS / 2);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BsktMintState));

    env.set_bskt_mint(bskt).unwrap();
    assert_eq!(env.config().bskt_mint, Some(bskt));
    assert!(env.set_bskt_mint(Pubkey::new_unique()).is_err(), "immutable once set");
    assert_eq!(env.config().bskt_mint, Some(bskt));

    let ix = env.stage_buyback_ix(&keeper.pubkey(), LAMPORTS / 2);
    env.send_ok(&[ix], &keeper, &[]);
}

#[test]
fn keeper_stages_sol_then_buys_and_burns_bskt() {
    let mut env = Env::initialized();
    let (bskt, venue) = setup(&mut env, LAMPORTS);
    let keeper = keeper(&env);
    env.set_bskt_mint(bskt).unwrap();
    let vault = buyback_vault_pda();
    let wsol_ata = env.create_ata(&vault, &WSOL);
    let vault_free_before = env.free_lamports(&vault);
    assert_eq!(vault_free_before, LAMPORTS);

    // Stage: lamports move from the PDA onto its wSOL account.
    let staged = LAMPORTS / 2;
    let ix = env.stage_buyback_ix(&keeper.pubkey(), staged);
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.free_lamports(&vault), LAMPORTS - staged);
    assert_eq!(env.token_amount(&wsol_ata), 0, "not synced yet");
    assert_eq!(env.buyback_vault().spent_lamports, staged);
    // Over-staging is refused.
    let ix = env.stage_buyback_ix(&keeper.pubkey(), LAMPORTS);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::MathOverflow));

    // Execute: sync, swap wSOL → BSKT through the venue, burn.
    let supply_before = env.mint_supply(&bskt);
    let inner = env.venue_swap_ix(&venue, &vault, &WSOL, staged);
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, staged, staged * 95 / 100, &inner);
    let m = env.send_ok(&[ix], &keeper, &[]);
    eprintln!("execute_buyback CU: {}", m.compute_units_consumed);
    let burned = supply_before - env.mint_supply(&bskt);
    assert!(burned > staged * 95 / 100 && burned < staged, "burned {burned} for {staged} lamports");
    assert_eq!(env.buyback_vault().burned_bskt, burned);
    assert_eq!(env.token_amount(&Env::ata(&vault, &bskt)), 0, "everything bought is burned");
    assert_eq!(env.token_amount(&wsol_ata), 0, "all staged wSOL spent");
    assert_eq!(env.free_lamports(&vault), LAMPORTS - staged, "the unstaged half stays");

    // A second round with the rest.
    let ix = env.stage_buyback_ix(&keeper.pubkey(), LAMPORTS - staged);
    env.send_ok(&[ix], &keeper, &[]);
    let inner = env.venue_swap_ix(&venue, &vault, &WSOL, LAMPORTS - staged);
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, LAMPORTS - staged, 1, &inner);
    env.send_ok(&[ix], &keeper, &[]);
    assert_eq!(env.free_lamports(&vault), 0);
    assert_eq!(env.buyback_vault().spent_lamports, LAMPORTS);
    assert!(env.buyback_vault().burned_bskt > burned);
}

#[test]
fn buyback_guards() {
    let mut env = Env::initialized();
    let (bskt, venue) = setup(&mut env, LAMPORTS);
    let keeper = keeper(&env);
    let vault = buyback_vault_pda();
    env.create_ata(&vault, &WSOL);
    let stranger = env.fund(LAMPORTS);

    // Execute before the mint is set.
    let inner = env.venue_swap_ix(&venue, &vault, &WSOL, 1_000_000);
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, 1_000_000, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BsktMintState));
    env.set_bskt_mint(bskt).unwrap();

    // Keeper only.
    let ix = env.stage_buyback_ix(&stranger.pubkey(), 1_000_000);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
    let ix = env.stage_buyback_ix(&keeper.pubkey(), 0);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::ZeroAmount));
    let ix = env.stage_buyback_ix(&keeper.pubkey(), 100_000_000);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.execute_buyback_ix(&stranger.pubkey(), &bskt, 1_000_000, 0, &inner);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));

    // Wrong mint account.
    let other = env.create_mint(6, &env.admin.pubkey());
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &other, 1_000_000, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BsktMintState));

    // Venue must be allow-listed.
    env.update_config(basket::state::ConfigUpdate { swap_programs: Some(vec![]), ..Default::default() });
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, 1_000_000, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SwapProgramNotAllowed));
    env.allow_dlmm_swaps();

    // Slippage floor.
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, 1_000_000, 1_000_000, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::SlippageExceeded));

    // The inner instruction may not drain more than `amount_in`.
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, 500_000, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::InvalidArgument));

    // Paused.
    env.set_paused(true);
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, 1_000_000, 0, &inner);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::Paused));
    let ix = env.stage_buyback_ix(&keeper.pubkey(), 1_000_000);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::Paused));
    env.set_paused(false);

    // And the happy path still works afterwards.
    let ix = env.execute_buyback_ix(&keeper.pubkey(), &bskt, 1_000_000, 0, &inner);
    env.send_ok(&[ix], &keeper, &[]);
    assert!(env.buyback_vault().burned_bskt > 0);
}
