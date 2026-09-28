//! `redeem`: burn shares, receive pro-rata components in kind plus the
//! pro-rata slice of the sleeve (SOL out, withdrawn treasury shares burned),
//! 0.25% fee withheld as shares. Frozen component accounts are skipped into a
//! `FrozenClaim`; large books use `redeem_begin` + `redeem_components`
//! (D9, D10, §4, §5).

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn seeded(env: &mut Env, n: usize, nonce: u64) -> (Launched, SeededBasket) {
    let l = env.launch_fixed(n, nonce);
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    (l, s)
}

#[test]
fn redeem_pays_pro_rata_components_and_sleeve() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 3, 1);
    let holder = l.payer.insecure_clone();
    let holder_shares_total = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
    let h = env.holder_shares(&l, &s);
    assert_eq!(h, holder_shares_total + SeedPlan::default_for(&l).initial_shares / 100, "H = buyer + fee vault");

    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    let pool_before = env.pool_state(&s.pool);
    let pos_before = env.position_state(&basket::damm::position(&s.position_nft_mint));
    let fee_ata = Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint);
    let fee_before = env.token_amount(&fee_ata);
    let sol_before = env.lamports(&holder.pubkey());

    // Redeem half of the holder's shares.
    let gross = holder_shares_total / 2;
    let fee = gross * DEFAULT_REDEEM_FEE_BPS as u64 / BPS_TOTAL as u64;
    let net = gross - fee;
    let r = env.redeem(&l, &s, &holder, gross);
    println!("redeem(3 positions) CU: {}", r.cu);

    // Components: net / H of every vault.
    for (i, mint) in l.mints.iter().enumerate() {
        let expected = (vaults[i] as u128 * net as u128 / h as u128) as u64;
        assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), mint)), expected, "component {i}");
        assert_eq!(env.token_amount(&Env::ata(&l.basket, mint)), vaults[i] - expected);
    }
    // Shares: gross burned from the holder, fee parked in the FeeVault.
    assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)), holder_shares_total - gross);
    assert_eq!(env.token_amount(&fee_ata), fee_before + fee);
    // Sleeve: net / H of the position's liquidity removed, price unchanged,
    // withdrawn shares burned, SOL to the holder.
    let pool = env.pool_state(&s.pool);
    let pos = env.position_state(&basket::damm::position(&s.position_nft_mint));
    let expected_l = pos_before.unlocked_liquidity * net as u128 / h as u128;
    assert_eq!(pos_before.unlocked_liquidity - pos.unlocked_liquidity, expected_l);
    assert_eq!(pool.sqrt_price, pool_before.sqrt_price);
    let (a_out, b_out) = basket::math::position_amounts(expected_l, pool.sqrt_min_price, pool.sqrt_price, pool.sqrt_max_price).unwrap();
    assert_eq!(env.token_amount(&pool_before.token_b_vault), env.token_amount(&pool.token_b_vault));
    let pool_b_delta = pool_before.token_b_amount - pool.token_b_amount;
    assert!(pool_b_delta.abs_diff(b_out) <= 1, "pool SOL out {pool_b_delta} vs {b_out}");
    let sol_gained = env.lamports(&holder.pubkey()) as i128 - sol_before as i128;
    assert!(sol_gained > b_out as i128 - 100_000 && sol_gained <= b_out as i128, "SOL gained {sol_gained} vs {b_out}");
    assert!(env.account(&Env::ata(&holder.pubkey(), &NATIVE_MINT)).is_none(), "temporary wSOL account closed");
    // Supply: gross − fee burned from holder side, plus the withdrawn pool shares.
    assert_eq!(env.token_amount(&Env::ata(&l.basket, &l.share_mint)), 0);
    let supply_expected = holder_shares_total + fee_before + pool_before.token_a_amount - net - a_out;
    assert!(env.mint_supply(&l.share_mint).abs_diff(supply_expected) <= 1);
    // Holder count H shrank by exactly net.
    assert!(env.holder_shares(&l, &s).abs_diff(h - net) <= 1);

    // Redeeming everything left (as the sole holder besides the fee vault) empties nothing improperly.
    let rest = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
    env.redeem(&l, &s, &holder, rest);
    assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)), 0);
    for mint in &l.mints {
        assert!(env.token_amount(&Env::ata(&l.basket, mint)) > 0, "fee-vault share of the vaults stays");
    }
}

#[test]
fn redeem_is_never_gated_or_paused() {
    let mut env = Env::initialized();
    let now = env.now();
    let l = env.launch_with(2, 2, |a| a.gate = MintGate::WindowUntil { close_ts: now + 60 });
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    let holder = l.payer.insecure_clone();
    env.warp(120);
    env.set_paused(true);
    let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 4;
    env.redeem(&l, &s, &holder, shares);
}

#[test]
fn redeem_validations() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 2, 3);
    let holder = l.payer.insecure_clone();
    let owned = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));

    let ix = env.redeem_ix(&l, &s, &holder.pubkey(), 0, &[]);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::ZeroAmount));
    // More than owned fails in the token program (insufficient funds = 0x1).
    let ix = env.redeem_ix(&l, &s, &holder.pubkey(), owned + 1, &[]);
    assert!(env.send(&[ix], &holder, &[]).is_err());
    // Partial component set.
    let mut ix = env.redeem_ix(&l, &s, &holder.pubkey(), owned / 2, &[]);
    ix.accounts.truncate(ix.accounts.len() - 4);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::ComponentCountMismatch));
    // A stranger cannot redeem the holder's shares.
    let stranger = env.fund(LAMPORTS);
    let mut ix = env.redeem_ix(&l, &s, &holder.pubkey(), owned / 2, &[]);
    ix.accounts[0] = anchor_lang::solana_program::instruction::AccountMeta::new(stranger.pubkey(), true);
    assert!(env.send(&[ix], &stranger, &[]).is_err());
}

#[test]
fn frozen_component_is_skipped_into_a_claim() {
    let mut env = Env::initialized();
    // Mints whose freeze authority is the admin (xStocks-style).
    let l = env.launch_fixed_freezable(3, 4);
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    let holder = l.payer.insecure_clone();
    let h = env.holder_shares(&l, &s);
    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();

    // Freeze the holder's ATA for component 1 (it exists from funding).
    let frozen_mint = l.mints[1];
    let holder_ata = Env::ata(&holder.pubkey(), &frozen_mint);
    env.freeze(&frozen_mint, &holder_ata);

    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let net = gross - gross * DEFAULT_REDEEM_FEE_BPS as u64 / BPS_TOTAL as u64;
    let owed = (vaults[1] as u128 * net as u128 / h as u128) as u64;

    // Without a claim account the transaction reverts with ComponentFrozen.
    let ix = env.redeem_ix(&l, &s, &holder.pubkey(), gross, &[]);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::ComponentFrozen));

    // With it, redeem succeeds, the frozen leg is recorded, the others paid.
    let claim = frozen_claim_pda(&l.share_mint, &holder.pubkey(), &frozen_mint);
    let ix = env.redeem_ix(&l, &s, &holder.pubkey(), gross, &[claim]);
    env.send_ok(&[ix], &holder, &[]);
    let c: FrozenClaim = env.load(&claim);
    assert_eq!((c.wallet, c.mint, c.amount), (holder.pubkey(), frozen_mint, owed));
    assert_eq!(env.token_amount(&Env::ata(&l.basket, &frozen_mint)), vaults[1], "frozen leg stays in the vault");
    let p: Position = env.load(&position_pda(&l.share_mint, &frozen_mint));
    assert_eq!(p.owed, owed);
    assert!(env.token_amount(&Env::ata(&holder.pubkey(), &l.mints[0])) > 0);

    // Owed amounts are excluded from later pro-rata maths: a second holder
    // minting now is priced off (vault − owed).
    let buyer = env.fund(100 * LAMPORTS);
    let deposits: Vec<u64> = l
        .mints
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let available = env.token_amount(&Env::ata(&l.basket, m)) - if i == 1 { owed } else { 0 };
            available / 2
        })
        .collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    let q = env.mint_quote(&l, &s, &deposits);
    let h_now = env.holder_shares(&l, &s);
    assert!(q.gross_shares.abs_diff(h_now / 2) <= 1, "{} vs {}", q.gross_shares, h_now / 2);
    env.mint(&l, &s, &buyer, &deposits, 0, u64::MAX);

    // claim_frozen fails while frozen, pays out once thawed, and closes the claim.
    let ix = env.claim_frozen_ix(&l, &holder.pubkey(), &frozen_mint);
    assert!(env.send(&[ix], &holder, &[]).is_err());
    env.thaw(&frozen_mint, &holder_ata);
    let ix = env.claim_frozen_ix(&l, &holder.pubkey(), &frozen_mint);
    env.send_ok(&[ix], &holder, &[]);
    assert_eq!(env.token_amount(&holder_ata), owed);
    assert!(env.account(&claim).is_none());
    let p: Position = env.load(&position_pda(&l.share_mint, &frozen_mint));
    assert_eq!(p.owed, 0);

    // A second frozen redemption tops up an existing claim.
    env.freeze(&frozen_mint, &holder_ata);
    let gross2 = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let ix = env.redeem_ix(&l, &s, &holder.pubkey(), gross2, &[claim]);
    env.send_ok(&[ix], &holder, &[]);
    let ix = env.redeem_ix(&l, &s, &holder.pubkey(), gross2 / 2, &[claim]);
    env.send_ok(&[ix], &holder, &[]);
    let c: FrozenClaim = env.load(&claim);
    let p: Position = env.load(&position_pda(&l.share_mint, &frozen_mint));
    assert!(c.amount > 0 && c.amount == p.owed);
}

#[test]
fn two_step_redeem_for_large_books() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 9, 5);
    let holder = l.payer.insecure_clone();
    let h = env.holder_shares(&l, &s);
    let vaults: Vec<u64> = l.mints.iter().map(|m| env.token_amount(&Env::ata(&l.basket, m))).collect();
    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let net = gross - gross * DEFAULT_REDEEM_FEE_BPS as u64 / BPS_TOTAL as u64;

    // Step 1: burn + sleeve, components pending.
    let m = env.send_ok(&[env.redeem_begin_ix(&l, &s, &holder.pubkey(), gross)], &holder, &[]);
    println!("redeem_begin CU: {}", m.compute_units_consumed);
    let red: Redemption = env.load(&redemption_pda(&l.share_mint, &holder.pubkey()));
    assert_eq!((red.shares, red.position_count, red.paid_count), (net, 9, 0));
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.pending_redeem_shares, net);
    // H is unchanged while components are pending (burned shares still count).
    assert!(env.holder_shares(&l, &s).abs_diff(h) <= 1);

    // A second begin for the same holder is blocked until this one settles.
    let ix = env.redeem_begin_ix(&l, &s, &holder.pubkey(), 1_000);
    assert!(env.send(&[ix], &holder, &[]).is_err());

    // Step 2: components in two chunks; a repeated chunk is rejected.
    let m = env.send_ok(&[env.redeem_components_ix(&l, &s, &holder.pubkey(), &l.book[..5], &[])], &holder, &[]);
    println!("redeem_components(5) CU: {}", m.compute_units_consumed);
    let red: Redemption = env.load(&redemption_pda(&l.share_mint, &holder.pubkey()));
    assert_eq!(red.paid_count, 5);
    let ix = env.redeem_components_ix(&l, &s, &holder.pubkey(), &l.book[4..6], &[]);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::DuplicateMint));
    env.send_ok(&[env.redeem_components_ix(&l, &s, &holder.pubkey(), &l.book[5..], &[])], &holder, &[]);
    assert!(env.account(&redemption_pda(&l.share_mint, &holder.pubkey())).is_none(), "closed when complete");
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.pending_redeem_shares, 0);
    for (i, mint) in l.mints.iter().enumerate() {
        let expected = (vaults[i] as u128 * net as u128 / h as u128) as u64;
        assert!(env.token_amount(&Env::ata(&holder.pubkey(), mint)).abs_diff(expected) <= 1, "component {i}");
    }
    assert!(env.holder_shares(&l, &s).abs_diff(h - net) <= 1);
}

#[test]
fn redeem_account_counts_and_cu_by_size() {
    let mut env = Env::initialized();
    for (i, n) in [2usize, 5, 8, 9].iter().enumerate() {
        let (l, s) = seeded(&mut env, *n, 10 + i as u64);
        let holder = l.payer.insecure_clone();
        let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
        let ix = env.redeem_ix(&l, &s, &holder.pubkey(), shares, &[]);
        let accounts = ix.accounts.len();
        let r = env.redeem(&l, &s, &holder, shares);
        println!("redeem N={n}: {accounts} accounts, {} CU", r.cu);
    }
}
