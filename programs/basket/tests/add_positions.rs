//! `add_positions`: books larger than one transaction (D20). Basket is inert
//! until `complete`; completion checks the chain hash and weight sum.

mod common;

use anchor_lang::prelude::Pubkey;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_keypair::Keypair;
use solana_signer::Signer;

fn setup(env: &mut Env, n: usize, nonce: u64) -> (Keypair, Keypair, Vec<PositionArg>, CreateBasketArgs, Pubkey) {
    let creator = Keypair::new();
    let payer = env.fund(50 * LAMPORTS);
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..n).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let bk = book(&mints, &equal_weights(n));
    let args = fixed_args(&creator.pubkey(), nonce, &bk);
    let share_mint = share_mint_pda(&creator.pubkey(), nonce);
    (creator, payer, bk, args, share_mint)
}

#[test]
fn twenty_assets_in_chunks_completes_only_at_the_end() {
    let mut env = Env::initialized();
    let (creator, payer, bk, args, share_mint) = setup(&mut env, 20, 1);
    let basket_key = basket_pda(&share_mint);

    // create_basket with the first 6, then 7 + 7.
    let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &args, &bk[..6])];
    let meta = env.send_ok(&ixs, &payer, &[]);
    println!("create_basket(6 positions) CU: {}", meta.compute_units_consumed);
    let b: Basket = env.load(&basket_key);
    assert_eq!(b.position_count, 6);
    assert!(!b.complete);

    let meta = env.send_ok(&[add_positions_ix(&payer.pubkey(), &share_mint, &bk[6..13])], &payer, &[]);
    println!("add_positions(7) CU: {}", meta.compute_units_consumed);
    let b: Basket = env.load(&basket_key);
    assert_eq!(b.position_count, 13);
    assert!(!b.complete);

    env.send_ok(&[add_positions_ix(&payer.pubkey(), &share_mint, &bk[13..])], &payer, &[]);
    let b: Basket = env.load(&basket_key);
    assert_eq!(b.position_count, 20);
    assert!(b.complete);
    assert_eq!(b.book_acc, b.book_hash);
    for (i, p) in bk.iter().enumerate() {
        let pos: Position = env.load(&position_pda(&share_mint, &p.mint));
        assert_eq!(pos.index, i as u16);
        assert!(env.account(&pos.vault).is_some());
    }

    // Complete baskets refuse more positions.
    let admin_key = env.admin.pubkey();
    let extra = env.create_mint(6, &admin_key);
    env.whitelist(&[extra]);
    let ix = add_positions_ix(&payer.pubkey(), &share_mint, &[PositionArg { mint: extra, weight_bps: 1 }]);
    env.send_expect_err(&[ix], &payer, &[], err(BasketError::ImmutableBasket));
}

#[test]
fn wrong_order_fails_at_completion_and_duplicates_fail_immediately() {
    let mut env = Env::initialized();
    let (creator, payer, bk, args, share_mint) = setup(&mut env, 4, 2);

    let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &args, &bk[..2])];
    env.send_ok(&ixs, &payer, &[]);

    // Re-adding an existing mint: its PDA already exists.
    let ix = add_positions_ix(&payer.pubkey(), &share_mint, &[bk[0]]);
    env.send_expect_err(&[ix], &payer, &[], err(BasketError::DuplicateMint));

    // Delivering the last two in the wrong order: hash mismatch at completion.
    let ix = add_positions_ix(&payer.pubkey(), &share_mint, &[bk[3], bk[2]]);
    env.send_expect_err(&[ix], &payer, &[], err(BasketError::BookHashMismatch));

    // Correct order completes.
    env.send_ok(&[add_positions_ix(&payer.pubkey(), &share_mint, &bk[2..])], &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint));
    assert!(b.complete);
}

#[test]
fn cannot_exceed_asset_count() {
    let mut env = Env::initialized();
    let (creator, payer, bk, mut args, share_mint) = setup(&mut env, 3, 3);
    args.asset_count = 2; // signed for two, deliver three
    args.book_hash = chain_hash(&bk[..2]);
    let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &args, &bk[..1])];
    env.send_ok(&ixs, &payer, &[]);
    // Three weights still sum to 10_000, so the count cap is what trips.
    let ix = add_positions_ix(&payer.pubkey(), &share_mint, &bk[1..]);
    env.send_expect_err(&[ix], &payer, &[], err(BasketError::TooManyAssets));
    // And an over-weight second position (count fits, weights do not).
    let heavy = PositionArg { mint: bk[1].mint, weight_bps: 10_000 };
    let ix = add_positions_ix(&payer.pubkey(), &share_mint, &[heavy]);
    env.send_expect_err(&[ix], &payer, &[], err(BasketError::WeightsMustSumToTotal));
}

#[test]
fn anyone_can_pay_for_positions_but_only_whitelisted_mints_enter() {
    let mut env = Env::initialized();
    let (creator, payer, bk, args, share_mint) = setup(&mut env, 3, 4);
    let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &args, &bk[..1])];
    env.send_ok(&ixs, &payer, &[]);

    let helper = env.fund(5 * LAMPORTS);
    env.send_ok(&[add_positions_ix(&helper.pubkey(), &share_mint, &bk[1..2])], &helper, &[]);

    // Swap the last mint for one that is not whitelisted (same weight).
    let admin_key = env.admin.pubkey();
    let rogue = env.create_mint(6, &admin_key);
    let ix = add_positions_ix(&helper.pubkey(), &share_mint, &[PositionArg { mint: rogue, weight_bps: bk[2].weight_bps }]);
    env.send_expect_err(&[ix], &helper, &[], err(BasketError::MintNotWhitelisted));
}
