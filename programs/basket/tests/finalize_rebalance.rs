//! `close_position` + `finalize_rebalance` (§5, D20): once the swaps are
//! done the keeper closes the positions that left the book (empty vaults,
//! rent back to the basket's rent payer) and walks the target book in
//! chunks, setting each position's weight and order. When the last chunk
//! lands the chain hash must equal the submitted target, weights must sum
//! to 10_000 and no stray position may remain; then `book_hash` flips, the
//! rebalance closes and the PendingBook's rent goes back to whoever paid it.

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn mirror_with_target(env: &mut Env, n: usize, nonce: u64, target: impl Fn(&Launched) -> (Vec<PositionArg>, Vec<Pubkey>)) -> (Launched, SeededBasket, Vec<PositionArg>) {
    let l = env.launch_mirror(n, nonce);
    let s = env.seed(&l, &SeedPlan::default_for(&l));
    let (t, new_mints) = target(&l);
    let keeper = env.keeper.insecure_clone();
    env.submit_book(&l, &keeper, &t, &new_mints);
    (l, s, t)
}

#[test]
fn finalize_reweights_reorders_and_flips_the_book_hash() {
    let mut env = Env::initialized();
    env.allow_cp_amm_swaps();
    let new_mint = env.create_mint(6, &env.admin.pubkey());
    env.whitelist(&[new_mint]);
    // [A, B, C] → [C, new, A]: B leaves, `new` enters, order changes.
    let (l, s, target) = mirror_with_target(&mut env, 3, 1, |l| {
        (book(&[l.mints[2], new_mint, l.mints[0]], &[5_000, 2_000, 3_000]), vec![new_mint])
    });
    let keeper = env.keeper.insecure_clone();
    let (a, b, c) = (l.mints[0], l.mints[1], l.mints[2]);

    // Trade B away for the new mint through a pool.
    let pool = env.create_component_pool(&b, &new_mint, 1_000_000_000_000);
    let vault_b = Env::ata(&l.basket, &b);
    let amt = env.token_amount(&vault_b);
    let inner = env.component_swap_ix(&pool, &l.basket, &b, &new_mint, amt);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &b, &new_mint, amt, amt * 99 / 100, &inner);
    env.send_ok(&[ix], &keeper, &[]);

    // Finalize is refused while B still has a position.
    let ix = env.finalize_ix(&l, &keeper.pubkey(), &target);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PositionsNotClosed));

    // Close B: vault empty, rent to the basket's payer.
    let payer_before = env.lamports(&l.payer.pubkey());
    let ix = env.close_position_ix(&l, &keeper.pubkey(), &b);
    env.send_ok(&[ix], &keeper, &[]);
    assert!(env.position(&l, &b).is_none());
    assert!(env.account(&vault_b).is_none());
    assert!(env.lamports(&l.payer.pubkey()) > payer_before);
    assert_eq!(env.load::<Basket>(&l.basket).position_count, 3);

    let pb_payer = env.load::<PendingBook>(&pending_book_pda(&l.share_mint)).payer;
    let payer_lamports = env.lamports(&pb_payer);
    let ix = env.finalize_ix(&l, &keeper.pubkey(), &target);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("finalize_rebalance(3) CU: {}", m.compute_units_consumed);

    let bk: Basket = env.load(&l.basket);
    assert!(!bk.rebalance.active);
    assert_eq!(bk.book_hash, chain_hash(&target));
    assert_eq!(bk.book_acc, chain_hash(&target));
    assert_eq!((bk.position_count, bk.asset_count, bk.weight_acc), (3, 3, BPS_TOTAL));
    for (i, e) in target.iter().enumerate() {
        let p = env.position(&l, &e.mint).unwrap();
        assert_eq!((p.weight_bps, p.index as usize), (e.weight_bps, i), "entry {i}");
    }
    assert!(env.account(&pending_book_pda(&l.share_mint)).is_none(), "pending book closed");
    assert!(env.lamports(&pb_payer) > payer_lamports - 20_000, "rent refunded");

    // The basket works on the new book: mint priced off [C, new, A].
    let buyer = env.fund(10 * LAMPORTS);
    let new_book = target.clone();
    let vaults: Vec<u64> = new_book.iter().map(|e| env.token_amount(&Env::ata(&l.basket, &e.mint))).collect();
    assert!(vaults.iter().all(|v| *v > 0));
    let deposits: Vec<u64> = vaults.iter().map(|v| v / 10).collect();
    for (e, d) in new_book.iter().zip(&deposits) {
        env.mint_to(&e.mint, &buyer.pubkey(), *d);
    }
    let h = env.holder_shares(&l, &s);
    let l2 = Launched { book: new_book.clone(), mints: new_book.iter().map(|e| e.mint).collect(), ..l };
    let r = env.mint(&l2, &s, &buyer, &deposits, 0, u64::MAX);
    assert!(r.cu > 0);
    let got = env.token_amount(&Env::ata(&buyer.pubkey(), &l2.share_mint));
    assert!(got.abs_diff(h / 10 * 99 / 100) <= h / 10_000, "{got} vs {}", h / 10);
    let _ = (a, c);
}

#[test]
fn finalize_in_chunks_for_large_books() {
    let mut env = Env::initialized();
    let (l, _s, target) = mirror_with_target(&mut env, 9, 2, |l| {
        // 8 entries: 3_000 + 7 × 1_000.
        let mut w = vec![1_000u16; 8];
        w[0] = 3_000;
        let mints: Vec<Pubkey> = l.mints[..8].to_vec();
        (book(&mints, &w), vec![])
    });
    let keeper = env.keeper.insecure_clone();
    // Position 8 leaves; give its tokens away out of band is not possible —
    // instead sell it into the pool of position 7.
    env.allow_cp_amm_swaps();
    let pool = env.create_component_pool(&l.mints[8], &l.mints[7], 1_000_000_000_000);
    let amt = env.token_amount(&Env::ata(&l.basket, &l.mints[8]));
    let inner = env.component_swap_ix(&pool, &l.basket, &l.mints[8], &l.mints[7], amt);
    let ix = env.execute_swap_ix(&l, &keeper.pubkey(), &l.mints[8], &l.mints[7], amt, 0, &inner);
    env.send_ok(&[ix], &keeper, &[]);
    let ix = env.close_position_ix(&l, &keeper.pubkey(), &l.mints[8]);
    env.send_ok(&[ix], &keeper, &[]);

    // First 5 entries, then the last 3.
    let ix = env.finalize_ix(&l, &keeper.pubkey(), &target[..5]);
    env.send_ok(&[ix], &keeper, &[]);
    let bk: Basket = env.load(&l.basket);
    assert!(bk.rebalance.active);
    assert_eq!(bk.rebalance.acc_count, 5);
    assert_eq!(bk.book_hash, chain_hash(&l.book), "not flipped yet");
    // Repeating a chunk out of order is rejected: entries must continue the walk.
    let ix = env.finalize_ix(&l, &keeper.pubkey(), &target[..5]);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::BookHashMismatch));
    let ix = env.finalize_ix(&l, &keeper.pubkey(), &target[5..]);
    let m = env.send_ok(&[ix], &keeper, &[]);
    println!("finalize_rebalance(3 of 8) CU: {}", m.compute_units_consumed);
    let bk: Basket = env.load(&l.basket);
    assert!(!bk.rebalance.active);
    assert_eq!(bk.book_hash, chain_hash(&target));
    assert_eq!(bk.position_count, 8);
}

#[test]
fn close_position_guards() {
    let mut env = Env::initialized();
    let (l, _s, _target) = mirror_with_target(&mut env, 3, 3, |l| (book(&[l.mints[0], l.mints[1]], &[5_000, 5_000]), vec![]));
    let keeper = env.keeper.insecure_clone();
    let (a, c) = (l.mints[0], l.mints[2]);

    // Still holds tokens.
    let ix = env.close_position_ix(&l, &keeper.pubkey(), &c);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PositionNotEmpty));
    // In the target book: may not be closed even if empty.
    let ix = env.close_position_ix(&l, &keeper.pubkey(), &a);
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::OffBook));
    // Not a keeper.
    let stranger = env.fund(LAMPORTS);
    let ix = env.close_position_ix(&l, &stranger.pubkey(), &c);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));

    // A finalize with the wrong target is refused by the hash at completion.
    let wrong = book(&[l.mints[0], l.mints[1]], &[6_000, 4_000]);
    let ix = env.finalize_ix(&l, &keeper.pubkey(), &wrong);
    // Position C is still open → refused before the hash even matters.
    env.send_expect_err(&[ix], &keeper, &[], err(BasketError::PositionsNotClosed));
}

#[test]
fn finalize_without_a_rebalance_or_by_a_stranger_fails() {
    let mut env = Env::initialized();
    let l = env.launch_mirror(2, 4);
    env.seed(&l, &SeedPlan::default_for(&l));
    let keeper = env.keeper.insecure_clone();
    let ix = env.finalize_ix_without_pending(&l, &keeper.pubkey(), &l.book);
    assert!(env.send(&[ix], &keeper, &[]).is_err());

    let (l2, _s, target) = mirror_with_target(&mut env, 2, 5, |l| (book(&[l.mints[0], l.mints[1]], &[7_000, 3_000]), vec![]));
    let stranger = env.fund(LAMPORTS);
    let ix = env.finalize_ix(&l2, &stranger.pubkey(), &target);
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
}
