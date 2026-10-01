//! `create_basket`: the deployer signs and launches (change order §2.2):
//! share mint + metadata (D1, D2), DLMM pool with `collect_fee_mode = OnlyY`,
//! 1 SOL deposit, optional keeper tier attestation (Q10), book commitment
//! and positions (D20), §3/§4 band validation, bounded mint window (Q12).

mod common;

use anchor_lang::prelude::Pubkey;
use anchor_lang::AccountDeserialize;
use anchor_spl::token::spl_token;
use basket::constants::*;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_keypair::Keypair;
use solana_program_pack::Pack;
use solana_signer::Signer;

#[test]
fn creates_fixed_basket_with_full_book_in_one_tx() {
    let mut env = Env::initialized();
    let deposit = env.config().deposit_lamports;
    let l = env.launch_fixed(3, 1);

    let b: Basket = env.load(&l.basket);
    assert_eq!(b.creator, l.creator.pubkey());
    assert_eq!(b.payer, l.payer.pubkey(), "deployer is creator and payer");
    assert_eq!(b.share_mint, l.share_mint);
    assert_eq!(b.basket_type, BASKET_TYPE_FIXED);
    assert_eq!(b.profile, PROFILE_FIXED_INDEX);
    assert_eq!(b.creator_tier, 0, "no attestation -> base tier");
    assert_eq!(b.name, "Solana Blue");
    assert_eq!(b.symbol, "BLUE");
    assert_eq!(b.asset_count, 3);
    assert_eq!(b.position_count, 3);
    assert!(b.complete, "all positions delivered -> complete");
    assert!(!b.seeded);
    assert_eq!(b.book_hash, chain_hash(&l.book));
    assert_eq!(b.book_acc, b.book_hash);
    assert_eq!(b.weight_acc, BPS_TOTAL);
    assert_eq!(b.fees.mint_fee_bps, 100);
    assert_eq!(b.fees.redeem_fee_bps, 25);
    assert_eq!(b.fees.creator_split_bps, 2_000, "base tier creator line");
    assert_eq!(b.fees.buyback_split_bps, 5_000);
    assert_eq!(b.fees.team_split_bps, 2_500);
    assert_eq!(b.fees.prize_split_bps, 500);
    assert_eq!(b.fees.perf_fee_bps, 0);
    assert_eq!(b.fees.mgmt_fee_bps_per_year, 0);
    assert_eq!(b.sleeve.r_bps, 700);
    assert_eq!(b.sleeve.step_r_bps, 2_500);
    assert_eq!(b.gate, l.args.gate);
    assert!(matches!(b.gate, MintGate::WindowUntil { .. }), "harness default is a 48 h window");

    // Pool: index preset, bin step 25, 0.25 %, OnlyY, opened at the launch bin.
    assert_eq!(b.pool.lb_pair, l.pool.lb_pair);
    assert_eq!(b.pool.preset.bin_step, 25);
    assert_eq!(b.pool.preset.base_fee_bps, 25);
    assert_eq!(b.pool.launch_active_id, LAUNCH_ACTIVE_ID);
    assert!(l.pool.exists(&env), "DLMM pool created by CPI");
    let pair = l.pool.load(&env);
    assert_eq!(pair.active_id, LAUNCH_ACTIVE_ID);
    assert_eq!(pair.bin_step, 25);
    assert_eq!(pair.token_x_mint, l.share_mint, "X = shares");
    assert_eq!(pair.token_y_mint, WSOL, "Y = wSOL");
    assert_eq!(pair.parameters.collect_fee_mode, basket::dlmm::COLLECT_FEE_MODE_ONLY_Y, "all swap fees in SOL");
    assert!(!b.tight.is_set() && !b.backstop.is_set(), "positions are placed at seed / by the keeper");
    assert_eq!(b.backstop_state, BACKSTOP_UNPLACED);

    // Deposit: held on the Basket account, pool rent already spent from it.
    assert_eq!(b.deposit_lamports, deposit);
    assert!(b.deposit_spent_lamports > 0, "pool rent is non-refundable");
    assert!(b.deposit_spent_lamports < deposit / 2, "pool rent {} should be well under half the deposit", b.deposit_spent_lamports);
    let basket_rent = env.svm.minimum_balance_for_rent_exemption(env.account(&l.basket).unwrap().data.len());
    assert_eq!(env.lamports(&l.basket), basket_rent + deposit - b.deposit_spent_lamports);
    eprintln!("pool rent from deposit: {} lamports", b.deposit_spent_lamports);

    // Share mint: classic SPL, 6 dp, authority = share_auth, no freeze authority.
    let mint_acc = env.account(&l.share_mint).unwrap();
    assert_eq!(mint_acc.owner, spl_token::ID);
    let mint = spl_token::state::Mint::unpack(&mint_acc.data).unwrap();
    assert_eq!(mint.decimals, SHARE_DECIMALS);
    assert_eq!(mint.supply, 0);
    assert_eq!(mint.mint_authority.unwrap(), share_auth_pda(&l.share_mint));
    assert!(mint.freeze_authority.is_none());

    // Metaplex metadata exists and names the token.
    let md = env.account(&metadata_pda(&l.share_mint)).expect("metadata created");
    assert_eq!(md.owner, METAPLEX_ID);
    let text = String::from_utf8_lossy(&md.data);
    assert!(text.contains("Solana Blue") && text.contains("BLUE"));

    // Positions and vaults.
    for (i, p) in l.book.iter().enumerate() {
        let pos: Position = env.load(&position_pda(&l.share_mint, &p.mint));
        assert_eq!(pos.basket, l.basket);
        assert_eq!(pos.mint, p.mint);
        assert_eq!(pos.weight_bps, p.weight_bps);
        assert_eq!(pos.token_program, spl_token::ID);
        assert_eq!(pos.decimals, 6);
        assert_eq!(pos.index, i as u16);
        assert_eq!(pos.vault, Env::ata(&l.basket, &p.mint));
        let vault = env.account(&pos.vault).expect("vault ATA");
        let ta = spl_token::state::Account::unpack(&vault.data).unwrap();
        assert_eq!(ta.owner, l.basket);
        assert_eq!(ta.mint, p.mint);
    }
}

#[test]
fn same_deployer_and_nonce_cannot_launch_twice() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 7);
    // The share mint PDA already exists; `init` fails at the system program.
    let ix = create_basket_ix(&l.payer.pubkey(), &l.args, &[], None);
    assert!(env.send(&[ix], &l.payer, &[]).is_err());
    // A different nonce from the same wallet is fine.
    let mut args = l.args.clone();
    args.nonce = 8;
    let ix = create_basket_ix(&l.payer.pubkey(), &args, &l.book, None);
    env.send_ok(&[ix], &l.payer, &[]);
}

#[test]
fn deployer_needs_a_wsol_ata() {
    // DLMM reads the funder's Y token account at pool init; the frontend
    // creates it in the same transaction. Without it the launch fails.
    let mut env = Env::initialized();
    let payer = env.fund(50 * LAMPORTS);
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..2).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let bk = book(&mints, &equal_weights(2));
    let args = fixed_args(1, &bk, env.now());
    let ix = create_basket_ix(&payer.pubkey(), &args, &bk, None);
    assert!(env.send(&[ix.clone()], &payer, &[]).is_err(), "no wSOL ATA");
    let create = anchor_spl::associated_token::spl_associated_token_account::instruction::create_associated_token_account_idempotent(
        &payer.pubkey(),
        &payer.pubkey(),
        &WSOL,
        &spl_token::ID,
    );
    env.send_ok(&[create, ix], &payer, &[]);
}

#[test]
fn deposit_is_charged_to_the_deployer() {
    let mut env = Env::initialized();
    let payer = env.new_deployer();
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..2).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let bk = book(&mints, &equal_weights(2));
    let args = fixed_args(1, &bk, env.now());
    let before = env.lamports(&payer.pubkey());
    let ix = create_basket_ix(&payer.pubkey(), &args, &bk, None);
    env.send_ok(&[ix], &payer, &[]);
    let spent = before - env.lamports(&payer.pubkey());
    let deposit = env.config().deposit_lamports;
    assert!(spent > deposit, "deposit + rents: spent {spent}");
    // Everything but the deposit is account rent the deployer gets back at close
    // (Basket, mint, metadata, positions, vaults) or DLMM pool rent (from the deposit).
    assert!(spent < deposit + LAMPORTS, "rents stay well under 1 SOL: spent {spent}");
    eprintln!("launch cost {} lamports incl. {} deposit", spent, deposit);

    // Too poor for the deposit: fails cleanly.
    let poor = env.fund(LAMPORTS / 2);
    env.create_ata(&poor.pubkey(), &WSOL);
    let ix = create_basket_ix(&poor.pubkey(), &fixed_args(2, &bk, env.now()), &bk, None);
    assert!(env.send(&[ix], &poor, &[]).is_err());
}

#[test]
fn tier_attestation_sets_the_creator_line() {
    let mut env = Env::initialized();
    env.update_config(ConfigUpdate { creator_tier_bps: Some([2_000, 2_500, 3_000, 3_500]), ..Default::default() });

    // Tier 2, keeper-signed, in date.
    let l = env.launch_with_tier(2, 1, 2);
    let b: Basket = env.load(&l.basket);
    assert_eq!(b.creator_tier, 2);
    assert_eq!(b.fees.creator_split_bps, 3_000);
    assert_eq!(b.fees.buyback_split_bps, 5_000, "other lines are the config proportions; FeeSchedule::split shares the remainder");

    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..2).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let bk = book(&mints, &equal_weights(2));
    let payer = env.new_deployer();
    let mut nonce = 10;
    let args_for = |env: &Env, nonce: u64| fixed_args(nonce, &bk, env.now());

    // Expired attestation: accepted, tier 0.
    let a = args_for(&env, nonce);
    let (sig_ix, att) = tier_attestation_for(&env.keeper, 2, &payer.pubkey(), 3, env.slot().saturating_sub(1));
    env.send_ok(&[sig_ix, create_basket_ix(&payer.pubkey(), &a, &bk, Some(att))], &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint_pda(&payer.pubkey(), nonce)));
    assert_eq!(b.creator_tier, 0);
    assert_eq!(b.fees.creator_split_bps, 2_000);
    nonce += 1;

    // Signed by a non-keeper.
    let a = args_for(&env, nonce);
    let stranger = Keypair::new();
    let (sig_ix, att) = tier_attestation_for(&stranger, 2, &payer.pubkey(), 3, env.slot() + 100);
    env.send_expect_err(&[sig_ix, create_basket_ix(&payer.pubkey(), &a, &bk, Some(att))], &payer, &[], err(BasketError::Unauthorized));

    // Attestation argument without the precompile instruction.
    let (_, att) = env.tier_attestation(&payer.pubkey(), 3);
    env.send_expect_err(&[create_basket_ix(&payer.pubkey(), &a, &bk, Some(att))], &payer, &[], err(BasketError::BadSignature));

    // Signed for someone else.
    let (sig_ix, att) = env.tier_attestation(&Keypair::new().pubkey(), 3);
    env.send_expect_err(&[sig_ix, create_basket_ix(&payer.pubkey(), &a, &bk, Some(att))], &payer, &[], err(BasketError::BadSignature));

    // Signed for another cluster.
    let (sig_ix, att) = tier_attestation_for(&env.keeper, 0, &payer.pubkey(), 3, env.slot() + 100);
    env.send_expect_err(&[sig_ix, create_basket_ix(&payer.pubkey(), &a, &bk, Some(att))], &payer, &[], err(BasketError::BadSignature));

    // Tier out of range.
    let (sig_ix, att) = env.tier_attestation(&payer.pubkey(), TIER_COUNT as u8);
    env.send_expect_err(&[sig_ix, create_basket_ix(&payer.pubkey(), &a, &bk, Some(att))], &payer, &[], err(BasketError::InvalidArgument));

    // Attestation for the top tier signed by the keeper: accepted.
    let (sig_ix, att) = env.tier_attestation(&payer.pubkey(), 3);
    env.send_ok(&[sig_ix, create_basket_ix(&payer.pubkey(), &a, &bk, Some(att))], &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint_pda(&payer.pubkey(), nonce)));
    assert_eq!(b.creator_tier, 3);
    assert_eq!(b.fees.creator_split_bps, 3_500);
}

#[test]
fn validates_book_and_bands() {
    let mut env = Env::initialized();
    let payer = env.new_deployer();
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..3).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints[..2]); // third mint stays off the list

    let two = book(&mints[..2], &equal_weights(2));
    let mut nonce = 100;
    let try_args = |env: &mut Env, args: CreateBasketArgs, chunk: &[PositionArg], code: u32| {
        env.send_expect_err(&[create_basket_ix(&payer.pubkey(), &args, chunk, None)], &payer, &[], code);
    };

    // Not whitelisted.
    let bad = book(&[mints[0], mints[2]], &equal_weights(2));
    let mut a = fixed_args(nonce, &bad, env.now());
    nonce += 1;
    a.asset_count = 2;
    try_args(&mut env, a, &bad, err(BasketError::MintNotWhitelisted));

    // Weights over 10_000 inside the chunk.
    let over = book(&mints[..2], &[6_000, 6_000]);
    let a = fixed_args(nonce, &over, env.now());
    nonce += 1;
    try_args(&mut env, a, &over, err(BasketError::WeightsMustSumToTotal));

    // Weights under 10_000 at completion.
    let under = book(&mints[..2], &[4_000, 4_000]);
    let a = fixed_args(nonce, &under, env.now());
    nonce += 1;
    try_args(&mut env, a, &under, err(BasketError::WeightsMustSumToTotal));

    // Book hash does not match delivered positions (committed one order, sent another).
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    let reversed: Vec<PositionArg> = two.iter().rev().cloned().collect();
    a.book_hash = chain_hash(&two);
    try_args(&mut env, a, &reversed, err(BasketError::BookHashMismatch));

    // Duplicate mint.
    let dup = vec![two[0], two[0]];
    let a = fixed_args(nonce, &dup, env.now());
    nonce += 1;
    try_args(&mut env, a, &dup, err(BasketError::DuplicateMint));

    // Fixed with > 20 assets.
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    a.asset_count = 21;
    try_args(&mut env, a, &[], err(BasketError::TooManyAssets));

    // Sleeve outside the index band (5–10%).
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    a.sleeve_r_bps = 1_100;
    try_args(&mut env, a, &two, err(BasketError::SleeveOutOfBand));

    // Meme profile accepts 20% and gets the meme pool preset (bin step 100, 1 %).
    let mut a = fixed_args(nonce, &two, env.now());
    a.fixed_kind = FIXED_KIND_MEME;
    a.sleeve_r_bps = 2_000;
    env.send_ok(&[create_basket_ix(&payer.pubkey(), &a, &two, None)], &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint_pda(&payer.pubkey(), nonce)));
    nonce += 1;
    assert_eq!(b.profile, PROFILE_FIXED_MEME);
    assert_eq!(b.pool.preset.bin_step, 100);
    assert_eq!(b.pool.preset.base_fee_bps, 100);
    assert_eq!(b.pool.preset.reset_confirm_checks, 1);
    assert_eq!(pool_for(&b.share_mint, 100).load(&env).bin_step, 100);

    // Perf fee on a non-Managed basket.
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    a.perf_fee_bps = 500;
    try_args(&mut env, a, &two, err(BasketError::FeeOutOfBounds));

    // Managed above the 20% cap; Managed at 15% is fine and copies mgmt fee.
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    a.basket_type = BASKET_TYPE_MANAGED;
    a.sleeve_r_bps = 1_200;
    a.perf_fee_bps = 2_100;
    try_args(&mut env, a.clone(), &two, err(BasketError::FeeOutOfBounds));
    a.nonce = nonce;
    nonce += 1;
    a.perf_fee_bps = 1_500;
    a.gate = MintGate::Closed;
    a.schedule = Some(ReopenSchedule { anchor_ts: env.now(), period_s: 7 * 86_400, open_s: 86_400 });
    env.send_ok(&[create_basket_ix(&payer.pubkey(), &a, &two, None)], &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint_pda(&payer.pubkey(), a.nonce)));
    assert_eq!(b.fees.perf_fee_bps, 1_500);
    assert_eq!(b.fees.mgmt_fee_bps_per_year, 100);
    assert_eq!(b.profile, PROFILE_MANAGED);
    assert_eq!(b.pool.preset.bin_step, 50, "Managed takes the Mirror/Strategy preset");
    assert!(b.gate_open_at(env.now()), "schedule opens at anchor");
    assert!(!b.gate_open_at(env.now() + 2 * 86_400), "closed between windows");

    // Mirror needs a hosts hash; Strategy needs a strategy hash.
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    a.basket_type = BASKET_TYPE_MIRROR;
    a.sleeve_r_bps = 1_000;
    try_args(&mut env, a, &two, err(BasketError::InvalidArgument));
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    a.basket_type = BASKET_TYPE_STRATEGY;
    a.sleeve_r_bps = 1_200;
    try_args(&mut env, a, &two, err(BasketError::InvalidArgument));

    // Fixed with a non-zero fixed_kind for a non-Fixed type.
    let mut a = fixed_args(nonce, &two, env.now());
    nonce += 1;
    a.basket_type = BASKET_TYPE_MANAGED;
    a.fixed_kind = FIXED_KIND_MEME;
    a.sleeve_r_bps = 1_200;
    try_args(&mut env, a, &two, err(BasketError::InvalidBasketType));

    // Book hash committed to fewer assets than asset_count: incomplete after the chunk.
    let mut a = fixed_args(nonce, &two, env.now());
    a.asset_count = 3;
    env.send_ok(&[create_basket_ix(&payer.pubkey(), &a, &two, None)], &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint_pda(&payer.pubkey(), nonce)));
    assert!(!b.complete);
    assert_eq!(b.position_count, 2);
}

#[test]
fn mint_window_is_bounded_and_open_is_gated() {
    let mut env = Env::initialized();
    let payer = env.new_deployer();
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..2).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let two = book(&mints, &equal_weights(2));
    let w = env.config().mint_window;
    assert_eq!((w.default_s, w.min_s, w.max_s), (48 * 3_600, 24 * 3_600, 72 * 3_600));
    let mut nonce = 1;
    let mut try_gate = |env: &mut Env, gate: MintGate, code: Option<u32>| {
        let mut a = fixed_args(nonce, &two, env.now());
        nonce += 1;
        a.gate = gate;
        let ix = create_basket_ix(&payer.pubkey(), &a, &two, None);
        match code {
            Some(c) => env.send_expect_err(&[ix], &payer, &[], c),
            None => {
                env.send_ok(&[ix], &payer, &[]);
            }
        }
    };
    let now = env.now();
    try_gate(&mut env, MintGate::WindowUntil { close_ts: now - 1 }, Some(err(BasketError::InvalidGate)));
    try_gate(&mut env, MintGate::WindowUntil { close_ts: now + w.min_s - 1 }, Some(err(BasketError::InvalidGate)));
    try_gate(&mut env, MintGate::WindowUntil { close_ts: now + w.max_s + 1 }, Some(err(BasketError::InvalidGate)));
    try_gate(&mut env, MintGate::WindowUntil { close_ts: now + w.min_s }, None);
    try_gate(&mut env, MintGate::WindowUntil { close_ts: now + w.max_s }, None);
    try_gate(&mut env, MintGate::Closed, None);
    // Open ships disabled; admin can allow it.
    try_gate(&mut env, MintGate::Open, Some(err(BasketError::InvalidGate)));
    env.update_config(ConfigUpdate { allow_open_gate: Some(true), ..Default::default() });
    try_gate(&mut env, MintGate::Open, None);
    // A schedule cannot be combined with Open.
    let mut a = fixed_args(nonce, &two, env.now());
    a.gate = MintGate::Open;
    a.schedule = Some(ReopenSchedule { anchor_ts: env.now(), period_s: 7 * 86_400, open_s: 86_400 });
    env.send_expect_err(&[create_basket_ix(&payer.pubkey(), &a, &two, None)], &payer, &[], err(BasketError::InvalidGate));
}

#[test]
fn paused_blocks_creation() {
    let mut env = Env::initialized();
    env.set_paused(true);
    let payer = env.new_deployer();
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..2).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let bk = book(&mints, &equal_weights(2));
    let args = fixed_args(1, &bk, env.now());
    env.send_expect_err(&[create_basket_ix(&payer.pubkey(), &args, &bk, None)], &payer, &[], err(BasketError::Paused));
}

#[test]
fn share_auth_pda_is_the_portfolio_marker() {
    // D2: derive PDA(["share_auth", mint]) and compare with the mint authority.
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 3);
    let mint_acc = env.account(&l.share_mint).unwrap();
    let mint = spl_token::state::Mint::unpack(&mint_acc.data).unwrap();
    assert_eq!(mint.mint_authority.unwrap(), share_auth_pda(&l.share_mint));
    // An unrelated mint does not match.
    let admin_key = env.admin.pubkey();
    let other = env.create_mint(6, &admin_key);
    let other_acc = env.account(&other).unwrap();
    let other_mint = spl_token::state::Mint::unpack(&other_acc.data).unwrap();
    assert_ne!(other_mint.mint_authority.unwrap(), share_auth_pda(&other));
    let _ = Basket::try_deserialize(&mut &env.account(&l.basket).unwrap().data[..]).unwrap();
}
