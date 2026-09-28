//! `sweep_fees`: split everything the FeeVault has accumulated since the
//! last sweep (shares from mint/redeem fees, SOL from pool fees) into
//! holders 40% (stays in the vault, reserved for rewards) / creator 30% /
//! protocol 30%, per the schedule frozen into the basket (§4).

mod common;

use anchor_lang::prelude::*;
use basket::constants::*;
use basket::state::*;
use common::*;
use solana_signer::Signer as _;

fn seeded(env: &mut Env, n: usize, nonce: u64) -> (Launched, SeededBasket) {
    let l = env.launch_fixed(n, nonce);
    let plan = SeedPlan::default_for(&l);
    let s = env.seed(&l, &plan);
    (l, s)
}

fn split(total: u64, fees: &FeeSchedule) -> (u64, u64, u64) {
    let holder = total * fees.holder_split_bps as u64 / BPS_TOTAL as u64;
    let creator = total * fees.creator_split_bps as u64 / BPS_TOTAL as u64;
    (holder, creator, total - holder - creator)
}

#[test]
fn sweeps_shares_and_sol_by_the_basket_split() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 3, 1);
    let b: Basket = env.load(&l.basket);
    assert_eq!((b.fees.holder_split_bps, b.fees.creator_split_bps, b.fees.protocol_split_bps), (4_000, 3_000, 3_000));
    let fee_vault = fee_vault_pda(&l.share_mint);
    let fee_share_ata = Env::ata(&fee_vault, &l.share_mint);

    // Shares: the 1% seed mint fee. SOL: pool fees from a trade, claimed.
    let shares_total = env.token_amount(&fee_share_ata);
    assert_eq!(shares_total, SeedPlan::default_for(&l).initial_shares / 100);
    let trader = env.fund(10 * LAMPORTS);
    // (0.3 SOL sleeve with an 8× ceiling: a 2 SOL buy would run off the range.)
    env.buy_shares(&l, &s, &trader, LAMPORTS / 2);
    let sol_total = env.claim_pool_fees(&l, &s, &trader);
    assert!(sol_total > 0);

    let creator = l.creator.pubkey();
    env.svm.airdrop(&creator, LAMPORTS).unwrap();
    let treasury = env.treasury.pubkey();
    let creator_sol_before = env.lamports(&creator);
    let treasury_sol_before = env.lamports(&treasury);
    let treasury_shares_before = env.token_amount(&Env::ata(&treasury, &l.share_mint));

    let caller = env.fund(LAMPORTS);
    let cu = env.sweep_fees(&l, &caller);
    println!("sweep_fees CU: {cu}");

    let (hs, cs, ps) = split(shares_total, &b.fees);
    let (hl, cl, pl) = split(sol_total, &b.fees);
    assert_eq!(env.token_amount(&Env::ata(&creator, &l.share_mint)), cs, "creator shares");
    assert_eq!(env.token_amount(&Env::ata(&treasury, &l.share_mint)), treasury_shares_before + ps, "protocol shares");
    assert_eq!(env.token_amount(&fee_share_ata), hs, "holder shares stay in the vault");
    assert_eq!(env.lamports(&creator) - creator_sol_before, cl, "creator SOL");
    assert_eq!(env.lamports(&treasury) - treasury_sol_before, pl, "protocol SOL");
    assert_eq!(env.fee_vault_free_lamports(&l), hl, "holder SOL stays in the vault");

    let v = env.fee_vault(&l);
    assert_eq!((v.holder_reserve_shares, v.holder_reserve_lamports), (hs, hl));
    assert_eq!((v.swept_creator_shares, v.swept_creator_lamports), (cs, cl));
    assert_eq!((v.swept_protocol_shares, v.swept_protocol_lamports), (ps, pl));
    // Protocol is never the largest line.
    assert!(ps <= hs && pl <= hl);
}

#[test]
fn sweep_moves_only_what_arrived_since_the_last_sweep() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 2, 2);
    let creator = l.creator.pubkey();
    let caller = env.fund(LAMPORTS);
    env.sweep_fees(&l, &caller);
    let v1 = env.fee_vault(&l);
    let creator_shares_1 = env.token_amount(&Env::ata(&creator, &l.share_mint));
    let creator_sol_1 = env.lamports(&creator);

    // Nothing new: a second sweep is a no-op.
    env.sweep_fees(&l, &caller);
    let v2 = env.fee_vault(&l);
    assert_eq!(v2.holder_reserve_shares, v1.holder_reserve_shares);
    assert_eq!(v2.swept_creator_shares, v1.swept_creator_shares);
    assert_eq!(env.token_amount(&Env::ata(&creator, &l.share_mint)), creator_shares_1);
    assert_eq!(env.lamports(&creator), creator_sol_1);

    // A redeem adds 0.25% of the redeemed shares; only that is split.
    let holder = l.payer.insecure_clone();
    let gross = env.token_amount(&Env::ata(&holder.pubkey(), &l.share_mint)) / 2;
    let fee = gross * DEFAULT_REDEEM_FEE_BPS as u64 / BPS_TOTAL as u64;
    env.redeem(&l, &s, &holder, gross);
    env.sweep_fees(&l, &caller);
    let b: Basket = env.load(&l.basket);
    let (hs, cs, ps) = split(fee, &b.fees);
    let v3 = env.fee_vault(&l);
    assert_eq!(v3.holder_reserve_shares, v1.holder_reserve_shares + hs);
    assert_eq!(v3.swept_creator_shares, v1.swept_creator_shares + cs);
    assert_eq!(v3.swept_protocol_shares, v1.swept_protocol_shares + ps);
    assert_eq!(env.token_amount(&Env::ata(&creator, &l.share_mint)), creator_shares_1 + cs);
    // SOL side untouched (no pool fees claimed).
    assert_eq!(env.lamports(&creator), creator_sol_1);
    assert_eq!(v3.holder_reserve_lamports, v1.holder_reserve_lamports);
}

#[test]
fn sweep_uses_the_schedule_frozen_into_the_basket() {
    let mut env = Env::initialized();
    let (l, _s) = seeded(&mut env, 2, 3);
    // Admin changes the defaults for future baskets: 50/25/25.
    let mut fees = FeeDefaults::default();
    fees.holder_split_bps = 5_000;
    fees.creator_split_bps = 2_500;
    fees.protocol_split_bps = 2_500;
    let ix = env.admin_ix(
        anchor_lang::InstructionData::data(&basket::instruction::UpdateConfig {
            update: ConfigUpdate { fees: Some(fees), ..Default::default() },
        }),
    );
    let admin = env.admin.insecure_clone();
    env.send_ok(&[ix], &admin, &[]);

    let fee_vault = fee_vault_pda(&l.share_mint);
    let total = env.token_amount(&Env::ata(&fee_vault, &l.share_mint));
    let caller = env.fund(LAMPORTS);
    env.sweep_fees(&l, &caller);
    let b: Basket = env.load(&l.basket);
    let (hs, cs, _) = split(total, &b.fees);
    assert_eq!(b.fees.creator_split_bps, 3_000);
    assert_eq!(env.token_amount(&Env::ata(&l.creator.pubkey(), &l.share_mint)), cs);
    assert_eq!(env.fee_vault(&l).holder_reserve_shares, hs);
}

#[test]
fn creator_sol_is_held_back_while_it_would_leave_their_wallet_below_rent() {
    let mut env = Env::initialized();
    let (l, s) = seeded(&mut env, 2, 5);
    let creator = l.creator.pubkey();
    assert_eq!(env.lamports(&creator), 0, "creator never funded a wallet");
    let trader = env.fund(10 * LAMPORTS);
    env.buy_shares(&l, &s, &trader, LAMPORTS / 2);
    let sol_total = env.claim_pool_fees(&l, &s, &trader);
    let b: Basket = env.load(&l.basket);
    let (hl, cl, pl) = split(sol_total, &b.fees);
    assert!(cl < 890_880, "test premise: creator line is below rent exemption");

    // The sweep does not revert; the creator's SOL waits in the vault.
    let caller = env.fund(LAMPORTS);
    let treasury_before = env.lamports(&env.treasury.pubkey());
    env.sweep_fees(&l, &caller);
    assert_eq!(env.lamports(&creator), 0);
    assert_eq!(env.lamports(&env.treasury.pubkey()) - treasury_before, pl);
    let v = env.fee_vault(&l);
    assert_eq!((v.creator_owed_lamports, v.swept_creator_lamports, v.holder_reserve_lamports), (cl, 0, hl));
    assert_eq!(env.fee_vault_free_lamports(&l), hl + cl);
    // Shares still go out: a token account has its own rent.
    assert!(env.token_amount(&Env::ata(&creator, &l.share_mint)) > 0);

    // Once the wallet exists the backlog is paid with the next sweep.
    env.svm.airdrop(&creator, LAMPORTS).unwrap();
    env.buy_shares(&l, &s, &trader, LAMPORTS / 10);
    let more = env.claim_pool_fees(&l, &s, &trader);
    let (_, cl2, _) = split(more, &b.fees);
    env.sweep_fees(&l, &caller);
    assert_eq!(env.lamports(&creator), LAMPORTS + cl + cl2);
    let v = env.fee_vault(&l);
    assert_eq!((v.creator_owed_lamports, v.swept_creator_lamports), (0, cl + cl2));
}

#[test]
fn sweep_rejects_wrong_creator_or_treasury() {
    let mut env = Env::initialized();
    let (l, _s) = seeded(&mut env, 2, 4);
    let caller = env.fund(LAMPORTS);
    let good = env.sweep_fees_ix(&l, &caller.pubkey());
    let stranger = Pubkey::new_unique();

    let mut ix = good.clone();
    ix.accounts[6].pubkey = stranger; // creator
    ix.accounts[7].pubkey = Env::ata(&stranger, &l.share_mint);
    assert!(env.send(&[ix], &caller, &[]).is_err());

    let mut ix = good.clone();
    ix.accounts[8].pubkey = stranger; // treasury
    ix.accounts[9].pubkey = Env::ata(&stranger, &l.share_mint);
    assert!(env.send(&[ix], &caller, &[]).is_err());
}
