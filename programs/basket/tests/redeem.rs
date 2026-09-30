//! `redeem`: burn shares, receive pro-rata components in kind plus the
//! pro-rata slice of the sleeve held in the **tight** position and idle
//! (SOL out, withdrawn treasury shares burned), 0.25% fee withheld as
//! shares. The backstop is never touched (change order Q14). Frozen
//! component accounts are skipped into a `FrozenClaim`; large books use
//! `redeem_begin` + `redeem_components` (D9, D10, §4, §5).

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn seeded(env: &mut Env, n: usize, nonce: u64) -> Launched {
    let (l, _) = env.launch_and_seed(n, nonce);
    l
}

fn redeem_fee(gross: u64) -> u64 {
    gross * DEFAULT_REDEEM_FEE_BPS as u64 / BPS_TOTAL as u64
}

/// Everything a redeem should move, computed from the state before it.
struct Expect {
    h: u64,
    net: u64,
    fee: u64,
    components: Vec<u64>,
    /// Pro-rata of tight SOL + idle SOL.
    lamports: u64,
    /// Pro-rata of the idle treasury shares (burned alongside the tight X).
    idle_x_share: u64,
}

fn expect_for(env: &Env, l: &Launched, gross: u64) -> Expect {
    let h = env.holder_shares(l);
    let fee = redeem_fee(gross);
    let net = gross - fee;
    let vaults = env.vaults(l);
    let components = vaults.iter().map(|v| (*v as u128 * net as u128 / h as u128) as u64).collect();
    let pool_sol = env.tight_amounts(l).amount_y + env.idle_sol(l);
    let lamports = (pool_sol as u128 * net as u128 / h as u128) as u64;
    let idle_x_share = (env.idle_shares(l) as u128 * net as u128 / h as u128) as u64;
    Expect { h, net, fee, components, lamports, idle_x_share }
}

/// A DLMM position's per-bin liquidity shares.
fn position_shares(env: &Env, position: &Pubkey) -> Vec<u128> {
    let data = env.account(position).expect("position").data;
    let h = basket::dlmm::position_header(&data).unwrap();
    (0..(h.upper_bin_id - h.lower_bin_id + 1) as usize).map(|o| basket::dlmm::position_bin_share(&data, o).unwrap()).collect()
}

/// SOL a wallet holds: lamports plus its wSOL ATA balance (the deployer keeps
/// a wSOL ATA from launch, so the redeem's SOL leg may land there).
fn sol_of(env: &Env, wallet: &Pubkey) -> u64 {
    let ata = Env::ata(wallet, &WSOL);
    env.lamports(wallet) + if env.account(&ata).is_some() { env.token_amount(&ata) } else { 0 }
}

#[test]
fn redeem_pays_pro_rata_components_and_tight_sleeve() {
    let mut env = Env::initialized();
    let l = seeded(&mut env, 3, 1);
    let holder = l.payer.insecure_clone();
    let owned = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
    let plan = SeedPlan::default_for(&l);
    assert_eq!(env.holder_shares(&l), owned + plan.initial_shares / 100, "H = buyer + fee vault");

    let vaults = env.vaults(&l);
    let tight_before = env.tight_amounts(&l);
    let fee_ata = Env::ata(&fee_vault_pda(&l.share_mint), &l.share_mint);
    let fee_before = env.token_amount(&fee_ata);
    let supply_before = env.mint_supply(&l.share_mint);
    let idle_x_before = env.idle_shares(&l);
    let idle_y_before = env.idle_sol(&l);
    assert!(idle_x_before > 0 && idle_y_before > 0, "the backstop slice waits idle until the keeper places it");
    let sol_before = sol_of(&env, &holder.pubkey());
    let active_before = l.pool.active_id(&env);

    // Redeem half of the holder's shares.
    let gross = owned / 2;
    let e = expect_for(&env, &l, gross);
    let r = env.redeem(&l, &holder, gross);
    eprintln!("redeem(3 positions) CU: {}", r.cu);

    // Components: net / H of every vault.
    for (i, mint) in l.mints.iter().enumerate() {
        assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), mint)), e.components[i], "component {i}");
        assert_eq!(env.token_amount(&Env::ata(&l.basket, mint)), vaults[i] - e.components[i]);
    }
    // Shares: gross burned from the holder, fee parked in the FeeVault.
    assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)), owned - gross);
    assert_eq!(env.token_amount(&fee_ata), fee_before + e.fee);

    // Sleeve: ⌊net·10⁴/H⌋ bps of the tight position removed, price unchanged.
    let tight = env.tight_amounts(&l);
    let bps = (e.net as u128 * BPS_TOTAL as u128 / e.h as u128) as u64;
    let want_y = tight_before.amount_y * bps / BPS_TOTAL as u64;
    let want_x = tight_before.amount_x * bps / BPS_TOTAL as u64;
    let y_out = tight_before.amount_y - tight.amount_y;
    let x_out = tight_before.amount_x - tight.amount_x;
    assert!(y_out.abs_diff(want_y) <= 70, "tight SOL out {y_out} vs {want_y}");
    assert!(x_out.abs_diff(want_x) <= 70, "tight shares out {x_out} vs {want_x}");
    assert_eq!(l.pool.active_id(&env), active_before);
    // SOL to the holder: the tight leg plus net/H of the idle SOL (the bps
    // shortfall is topped up from idle), exactly the pro-rata of both.
    let sol_gained = sol_of(&env, &holder.pubkey()) as i128 - sol_before as i128;
    assert!(sol_gained > e.lamports as i128 - 100_000 && sol_gained <= e.lamports as i128, "SOL gained {sol_gained} vs pro-rata {}", e.lamports);
    assert!(sol_gained > y_out as i128, "idle SOL contributes too");
    assert!(env.idle_sol(&l).abs_diff(idle_y_before - (e.lamports - y_out)) <= 2);
    // Withdrawn treasury shares burned, along with net/H of the idle shares.
    assert_eq!(env.idle_shares(&l), idle_x_before - e.idle_x_share);
    assert_eq!(env.mint_supply(&l.share_mint), supply_before - e.net - x_out - e.idle_x_share);
    // H shrank by exactly net.
    assert!(env.holder_shares(&l).abs_diff(e.h - e.net) <= 1);

    // A fresh holder without a wSOL account gets one created and closed again.
    let buyer = env.fund(10 * LAMPORTS);
    let deposits: Vec<u64> = env.vaults(&l).iter().map(|v| v / 2).collect();
    env.fund_components_for(&buyer.pubkey(), &l, &deposits);
    env.mint(&l, &buyer, &deposits, 0, u64::MAX);
    let bought = env.token_amount(&Env::ata(&buyer.pubkey(), &l.share_mint));
    let before = env.lamports(&buyer.pubkey());
    let e2 = expect_for(&env, &l, bought);
    env.redeem(&l, &buyer, bought);
    let gained = env.lamports(&buyer.pubkey()) as i128 - before as i128;
    assert!(gained > e2.lamports as i128 - 100_000 && gained <= e2.lamports as i128, "SOL gained {gained} vs {}", e2.lamports);
    assert!(env.account(&Env::ata(&buyer.pubkey(), &WSOL)).is_none(), "temporary wSOL account closed");

    // Redeeming everything left leaves the fee vault's share of the vaults.
    let rest = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));
    env.redeem(&l, &holder, rest);
    assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)), 0);
    for mint in &l.mints {
        assert!(env.token_amount(&Env::ata(&l.basket, mint)) > 0, "fee-vault share of the vaults stays");
    }
    let tight = env.tight_amounts(&l);
    assert!(tight.amount_y > 0, "fee vault's slice of the sleeve stays in tight");
}

#[test]
fn redeem_leaves_the_backstop_alone() {
    let mut env = Env::initialized();
    let l = seeded(&mut env, 3, 2);
    env.place_and_fund_backstop(&l);
    let holder = l.payer.insecure_clone();
    let backstop_before = env.backstop_amounts(&l);
    assert!(backstop_before.amount_x > 0 && backstop_before.amount_y > 0);
    let tight_before = env.tight_amounts(&l);
    let idle_before = env.idle_sol(&l);

    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 3;
    let e = expect_for(&env, &l, gross);
    let sol_before = sol_of(&env, &holder.pubkey());
    let backstop_key = l.basket_state(&env).backstop.key;
    let backstop_shares_before = position_shares(&env, &backstop_key);
    let r = env.redeem(&l, &holder, gross);
    eprintln!("redeem with live backstop CU: {}", r.cu);

    // The backstop's liquidity shares are untouched; its token amounts only
    // move by per-bin rounding as the shared bins shrink.
    let backstop = env.backstop_amounts(&l);
    assert_eq!(position_shares(&env, &backstop_key), backstop_shares_before, "backstop untouched");
    assert!(backstop.amount_x.abs_diff(backstop_before.amount_x) <= 70, "{} vs {}", backstop.amount_x, backstop_before.amount_x);
    assert!(backstop.amount_y.abs_diff(backstop_before.amount_y) <= 70, "{} vs {}", backstop.amount_y, backstop_before.amount_y);
    let tight = env.tight_amounts(&l);
    assert!(tight.amount_y < tight_before.amount_y);
    // Pro-rata of tight + idle only: the backstop's SOL is not in the numerator.
    let sol_gained = sol_of(&env, &holder.pubkey()) as i128 - sol_before as i128;
    assert!(sol_gained.abs_diff(e.lamports as i128) <= 100_000, "SOL gained {sol_gained} vs {}", e.lamports);
    let all_sol = tight_before.amount_y + idle_before + backstop_before.amount_y;
    let if_backstop_counted = (all_sol as u128 * e.net as u128 / e.h as u128) as i128;
    assert!(sol_gained < if_backstop_counted - 1_000_000, "backstop SOL must not be paid out");
    // H shrank by net (± the backstop's per-bin rounding).
    assert!(env.holder_shares(&l).abs_diff(e.h - e.net) <= 70);
}

#[test]
fn redeem_works_with_the_active_bin_outside_tight() {
    // Keeper late: price has left the tight range. Redeem still removes the
    // holder's slice of tight (now one-sided) and pays SOL from what is there.
    let mut env = Env::initialized();
    let l = seeded(&mut env, 2, 3);
    let holder = l.payer.insecure_clone();
    let b = l.basket_state(&env);
    env.push_price_below(&l, &holder, b.tight.lower_bin_id - 5);
    let active = l.pool.active_id(&env);
    assert!(active < b.tight.lower_bin_id);
    let tight_before = env.tight_amounts(&l);
    assert_eq!(tight_before.amount_y, 0, "all of tight is shares once price is below it");
    assert!(tight_before.amount_x > 0);

    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let e = expect_for(&env, &l, gross);
    let supply_before = env.mint_supply(&l.share_mint);
    let sol_before = sol_of(&env, &holder.pubkey());
    env.redeem(&l, &holder, gross);

    let tight = env.tight_amounts(&l);
    let x_out = tight_before.amount_x - tight.amount_x;
    let bps = (e.net as u128 * BPS_TOTAL as u128 / e.h as u128) as u64;
    assert!(x_out.abs_diff(tight_before.amount_x * bps / BPS_TOTAL as u64) <= 70);
    assert_eq!(env.mint_supply(&l.share_mint), supply_before - e.net - x_out - e.idle_x_share, "withdrawn shares burned");
    // The SOL leg is net/H of the idle SOL only (tight holds none).
    let sol_gained = sol_of(&env, &holder.pubkey()) as i128 - sol_before as i128;
    assert!(sol_gained > e.lamports as i128 - 100_000 && sol_gained <= e.lamports as i128, "SOL gained {sol_gained} vs {}", e.lamports);
    for (i, mint) in l.mints.iter().enumerate() {
        assert_eq!(env.token_amount(&Env::ata(&holder.pubkey(), mint)), e.components[i], "component {i}");
    }
    assert!(env.holder_shares(&l).abs_diff(e.h - e.net) <= 1);
}

#[test]
fn redeem_is_never_gated_or_paused() {
    let mut env = Env::initialized();
    let now = env.now();
    let l = env.launch_with(2, 4, |a| a.gate = MintGate::WindowUntil { close_ts: now + DEFAULT_MIN_MINT_WINDOW_S });
    let plan = SeedPlan::default_for(&l);
    env.seed(&l, &plan);
    let holder = l.payer.insecure_clone();
    env.warp(DEFAULT_MIN_MINT_WINDOW_S + 60);
    env.set_paused(true);
    let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 4;
    env.redeem(&l, &holder, shares);
}

#[test]
fn redeem_validations() {
    let mut env = Env::initialized();
    let l = seeded(&mut env, 2, 5);
    let holder = l.payer.insecure_clone();
    let owned = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint));

    let ix = env.redeem_ix(&l, &holder.pubkey(), 0, &[]);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::ZeroAmount));
    // More than owned fails in the token program (insufficient funds = 0x1).
    let ix = env.redeem_ix(&l, &holder.pubkey(), owned + 1, &[]);
    assert!(env.send(&[ix], &holder, &[]).is_err());
    // Partial component set.
    let mut ix = env.redeem_ix(&l, &holder.pubkey(), owned / 2, &[]);
    let tail = env.pool_tail(&l).len();
    let cut = ix.accounts.len() - tail - 4;
    ix.accounts.drain(cut..cut + 4);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::ComponentCountMismatch));
    // Missing bin arrays: the tight leg cannot be removed.
    let mut ix = env.redeem_ix(&l, &holder.pubkey(), owned / 2, &[]);
    ix.accounts.truncate(ix.accounts.len() - tail);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::BinArrayMismatch));
    // A stranger cannot redeem the holder's shares.
    let stranger = env.fund(LAMPORTS);
    let mut ix = env.redeem_ix(&l, &holder.pubkey(), owned / 2, &[]);
    ix.accounts[0] = anchor_lang::solana_program::instruction::AccountMeta::new(stranger.pubkey(), true);
    assert!(env.send(&[ix], &stranger, &[]).is_err());
    // A foreign position for tight.
    let mut ix = env.redeem_ix(&l, &holder.pubkey(), owned / 2, &[]);
    let idx = ix.accounts.iter().position(|m| m.pubkey == l.tight_position(&env)).unwrap();
    ix.accounts[idx].pubkey = l.placeholder();
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::PositionMismatch));
}

#[test]
fn frozen_component_is_skipped_into_a_claim() {
    let mut env = Env::initialized();
    // Mints whose freeze authority is the admin (xStocks-style).
    let l = env.launch_fixed_freezable(3, 6);
    let plan = SeedPlan::default_for(&l);
    env.seed(&l, &plan);
    let holder = l.payer.insecure_clone();
    let h = env.holder_shares(&l);
    let vaults = env.vaults(&l);

    // Freeze the holder's ATA for component 1 (it exists from funding).
    let frozen_mint = l.mints[1];
    let holder_ata = Env::ata(&holder.pubkey(), &frozen_mint);
    env.freeze(&frozen_mint, &holder_ata);

    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let net = gross - redeem_fee(gross);
    let owed = (vaults[1] as u128 * net as u128 / h as u128) as u64;

    // Without a claim account the transaction reverts with ComponentFrozen.
    let ix = env.redeem_ix(&l, &holder.pubkey(), gross, &[]);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::ComponentFrozen));

    // With it, redeem succeeds, the frozen leg is recorded, the others paid.
    let claim = frozen_claim_pda(&l.share_mint, &holder.pubkey(), &frozen_mint);
    let ix = env.redeem_ix(&l, &holder.pubkey(), gross, &[claim]);
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
    let q = env.mint_quote(&l, &deposits);
    let h_now = env.holder_shares(&l);
    assert!(q.gross_shares.abs_diff(h_now / 2) <= 1, "{} vs {}", q.gross_shares, h_now / 2);
    env.mint(&l, &buyer, &deposits, 0, u64::MAX);

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
    let ix = env.redeem_ix(&l, &holder.pubkey(), gross2, &[claim]);
    env.send_ok(&[ix], &holder, &[]);
    let ix = env.redeem_ix(&l, &holder.pubkey(), gross2 / 2, &[claim]);
    env.send_ok(&[ix], &holder, &[]);
    let c: FrozenClaim = env.load(&claim);
    let p: Position = env.load(&position_pda(&l.share_mint, &frozen_mint));
    assert!(c.amount > 0 && c.amount == p.owed);
}

#[test]
fn two_step_redeem_for_large_books() {
    let mut env = Env::initialized();
    let l = seeded(&mut env, 9, 7);
    let holder = l.payer.insecure_clone();
    let h = env.holder_shares(&l);
    let vaults = env.vaults(&l);
    let tight_before = env.tight_amounts(&l);
    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let net = gross - redeem_fee(gross);

    // Step 1: burn + sleeve, components pending.
    let m = env.send_ok(&[env.redeem_begin_ix(&l, &holder.pubkey(), gross)], &holder, &[]);
    eprintln!("redeem_begin CU: {}", m.compute_units_consumed);
    let red: Redemption = env.load(&redemption_pda(&l.share_mint, &holder.pubkey()));
    assert_eq!((red.shares, red.position_count, red.paid_count), (net, 9, 0));
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.pending_redeem_shares, net);
    assert!(env.tight_amounts(&l).amount_y < tight_before.amount_y, "sleeve leg paid at begin");
    // H is unchanged while components are pending (burned shares still count).
    assert!(env.holder_shares(&l).abs_diff(h) <= 1);

    // A second begin for the same holder is blocked until this one settles.
    let ix = env.redeem_begin_ix(&l, &holder.pubkey(), 1_000);
    assert!(env.send(&[ix], &holder, &[]).is_err());

    // Step 2: components in two chunks; a repeated chunk is rejected.
    let m = env.send_ok(&[env.redeem_components_ix(&l, &holder.pubkey(), &l.book[..5], &[])], &holder, &[]);
    eprintln!("redeem_components(5) CU: {}", m.compute_units_consumed);
    let red: Redemption = env.load(&redemption_pda(&l.share_mint, &holder.pubkey()));
    assert_eq!(red.paid_count, 5);
    let ix = env.redeem_components_ix(&l, &holder.pubkey(), &l.book[4..6], &[]);
    env.send_expect_err(&[ix], &holder, &[], err(BasketError::DuplicateMint));
    env.send_ok(&[env.redeem_components_ix(&l, &holder.pubkey(), &l.book[5..], &[])], &holder, &[]);
    assert!(env.account(&redemption_pda(&l.share_mint, &holder.pubkey())).is_none(), "closed when complete");
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.pending_redeem_shares, 0);
    for (i, mint) in l.mints.iter().enumerate() {
        let expected = (vaults[i] as u128 * net as u128 / h as u128) as u64;
        assert!(env.token_amount(&Env::ata(&holder.pubkey(), mint)).abs_diff(expected) <= 1, "component {i}");
    }
    assert!(env.holder_shares(&l).abs_diff(h - net) <= 1);
}

#[test]
fn redeem_account_counts_and_cu_by_size() {
    let mut env = Env::initialized();
    for (i, n) in [2usize, 5, 8, 9].iter().enumerate() {
        let l = seeded(&mut env, *n, 10 + i as u64);
        let holder = l.payer.insecure_clone();
        let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
        let ix = env.redeem_ix(&l, &holder.pubkey(), shares, &[]);
        let accounts = ix.accounts.len();
        assert!(accounts <= 64, "N={n}: {accounts} accounts");
        let r = env.redeem(&l, &holder, shares);
        eprintln!("redeem N={n}: {accounts} accounts, {} CU", r.cu);
        if *n == 8 {
            // Widest single-tx redeem with a live backstop (its arrays ride along).
            env.place_and_fund_backstop(&l);
            let shares = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
            let ix = env.redeem_ix(&l, &holder.pubkey(), shares, &[]);
            let accounts = ix.accounts.len();
            let r = env.redeem(&l, &holder, shares);
            eprintln!("redeem N={n} + backstop: {accounts} accounts, {} CU", r.cu);
        }
    }
}
