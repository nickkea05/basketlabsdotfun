//! `create_basket`: creator-signed payload (D19), share mint + metadata
//! (D1, D2), book commitment and positions (D20), validation of §3/§4 bands.

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
    let l = env.launch_fixed(3, 1);

    let b: Basket = env.load(&l.basket);
    assert_eq!(b.creator, l.creator.pubkey());
    assert_eq!(b.payer, l.payer.pubkey());
    assert_eq!(b.share_mint, l.share_mint);
    assert_eq!(b.basket_type, BASKET_TYPE_FIXED);
    assert_eq!(b.profile, PROFILE_FIXED_INDEX);
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
    assert_eq!(b.fees.pool_fee_bps, 30, "index profile pool fee");
    assert_eq!(b.fees.perf_fee_bps, 0);
    assert_eq!(b.fees.mgmt_fee_bps_per_year, 0);
    assert_eq!(b.sleeve.r_bps, 700);
    assert_eq!(b.sleeve.step_r_bps, 2_500);
    assert_eq!(b.sleeve.price_range, PriceRange::Bounded);
    assert_eq!(b.gate, MintGate::Open);
    assert_eq!(b.pool, Pubkey::default());

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
fn replaying_the_same_payload_fails() {
    let mut env = Env::initialized();
    let l = env.launch_fixed(2, 7);
    let other_payer = env.fund(10 * LAMPORTS);
    let ixs = [creator_signature_ix(&l.creator, &l.args, 1), create_basket_ix(&other_payer.pubkey(), &l.args, &[])];
    // The share mint PDA already exists; `init` fails at the system program.
    assert!(env.send(&ixs, &other_payer, &[]).is_err());
}

#[test]
fn rejects_missing_or_wrong_signature() {
    let mut env = Env::initialized();
    let creator = Keypair::new();
    let payer = env.fund(10 * LAMPORTS);
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..2).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let bk = book(&mints, &equal_weights(2));
    let args = fixed_args(&creator.pubkey(), 1, &bk);

    // No precompile instruction at all.
    env.send_expect_err(&[create_basket_ix(&payer.pubkey(), &args, &bk)], &payer, &[], err(BasketError::BadSignature));

    // Signed by someone other than args.creator.
    let imposter = Keypair::new();
    let ixs = [creator_signature_ix(&imposter, &args, 1), create_basket_ix(&payer.pubkey(), &args, &bk)];
    env.send_expect_err(&ixs, &payer, &[], err(BasketError::BadSignature));

    // Creator signed a different payload (name changed after signing).
    let mut tampered = args.clone();
    tampered.name = "Not what I signed".into();
    let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &tampered, &bk)];
    // The precompile itself rejects: message bytes differ from what was signed.
    assert!(env.send(&ixs, &payer, &[]).is_err());

    // Right signature, but payload bound to another cluster.
    let mut foreign = args.clone();
    foreign.domain.cluster = 0;
    let ixs = [creator_signature_ix(&creator, &foreign, 1), create_basket_ix(&payer.pubkey(), &foreign, &bk)];
    env.send_expect_err(&ixs, &payer, &[], err(BasketError::BadDomain));
}

#[test]
fn validates_book_and_bands() {
    let mut env = Env::initialized();
    let creator = Keypair::new();
    let payer = env.fund(20 * LAMPORTS);
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..3).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints[..2]); // third mint stays off the list

    let two = book(&mints[..2], &equal_weights(2));
    let mut nonce = 100;
    let try_args = |env: &mut Env, args: CreateBasketArgs, chunk: &[PositionArg], code: u32| {
        let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &args, chunk)];
        env.send_expect_err(&ixs, &payer, &[], code);
    };

    // Not whitelisted.
    let bad = book(&[mints[0], mints[2]], &equal_weights(2));
    let mut a = fixed_args(&creator.pubkey(), nonce, &bad);
    nonce += 1;
    a.asset_count = 2;
    try_args(&mut env, a, &bad, err(BasketError::MintNotWhitelisted));

    // Weights over 10_000 inside the chunk.
    let over = book(&mints[..2], &[6_000, 6_000]);
    let a = fixed_args(&creator.pubkey(), nonce, &over);
    nonce += 1;
    try_args(&mut env, a, &over, err(BasketError::WeightsMustSumToTotal));

    // Weights under 10_000 at completion.
    let under = book(&mints[..2], &[4_000, 4_000]);
    let a = fixed_args(&creator.pubkey(), nonce, &under);
    nonce += 1;
    try_args(&mut env, a, &under, err(BasketError::WeightsMustSumToTotal));

    // Book hash does not match delivered positions (signed one order, sent another).
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    let reversed: Vec<PositionArg> = two.iter().rev().cloned().collect();
    a.book_hash = chain_hash(&two);
    try_args(&mut env, a, &reversed, err(BasketError::BookHashMismatch));

    // Duplicate mint.
    let dup = vec![two[0], two[0]];
    let a = fixed_args(&creator.pubkey(), nonce, &dup);
    nonce += 1;
    try_args(&mut env, a, &dup, err(BasketError::DuplicateMint));

    // Fixed with > 20 assets.
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    a.asset_count = 21;
    try_args(&mut env, a, &[], err(BasketError::TooManyAssets));

    // Sleeve outside the index band (5–10%).
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    a.sleeve_r_bps = 1_100;
    try_args(&mut env, a, &two, err(BasketError::SleeveOutOfBand));

    // Meme profile accepts 20% and gets the 1% pool fee.
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    a.fixed_kind = FIXED_KIND_MEME;
    a.sleeve_r_bps = 2_000;
    a.gate = MintGate::WindowUntil { close_ts: env.now() + 3_600 };
    let ixs = [creator_signature_ix(&creator, &a, 1), create_basket_ix(&payer.pubkey(), &a, &two)];
    env.send_ok(&ixs, &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint_pda(&creator.pubkey(), a.domain.nonce)));
    assert_eq!(b.profile, PROFILE_FIXED_MEME);
    assert_eq!(b.fees.pool_fee_bps, 100);
    assert_eq!(b.sleeve.price_range, PriceRange::Full);
    assert!(b.uses_launch_scheduler());

    // Perf fee on a non-Managed basket.
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    a.perf_fee_bps = 500;
    try_args(&mut env, a, &two, err(BasketError::FeeOutOfBounds));

    // Managed above the 20% cap; Managed at 15% is fine and copies mgmt fee.
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    a.basket_type = BASKET_TYPE_MANAGED;
    a.sleeve_r_bps = 1_200;
    a.perf_fee_bps = 2_100;
    try_args(&mut env, a.clone(), &two, err(BasketError::FeeOutOfBounds));
    a.domain.nonce = nonce;
    nonce += 1;
    a.perf_fee_bps = 1_500;
    a.gate = MintGate::Closed;
    a.schedule = Some(ReopenSchedule { anchor_ts: env.now(), period_s: 7 * 86_400, open_s: 86_400 });
    let ixs = [creator_signature_ix(&creator, &a, 1), create_basket_ix(&payer.pubkey(), &a, &two)];
    env.send_ok(&ixs, &payer, &[]);
    let b: Basket = env.load(&basket_pda(&share_mint_pda(&creator.pubkey(), a.domain.nonce)));
    assert_eq!(b.fees.perf_fee_bps, 1_500);
    assert_eq!(b.fees.mgmt_fee_bps_per_year, 100);
    assert_eq!(b.profile, PROFILE_MANAGED);
    assert!(b.gate_open_at(env.now()), "schedule opens at anchor");
    assert!(!b.gate_open_at(env.now() + 2 * 86_400), "closed between windows");

    // Mirror needs a hosts hash; Strategy needs a strategy hash.
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    a.basket_type = BASKET_TYPE_MIRROR;
    a.sleeve_r_bps = 1_000;
    try_args(&mut env, a, &two, err(BasketError::InvalidArgument));
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    nonce += 1;
    a.basket_type = BASKET_TYPE_STRATEGY;
    a.sleeve_r_bps = 1_200;
    try_args(&mut env, a, &two, err(BasketError::InvalidArgument));

    // Window that already closed.
    let mut a = fixed_args(&creator.pubkey(), nonce, &two);
    a.gate = MintGate::WindowUntil { close_ts: env.now() - 1 };
    try_args(&mut env, a, &two, err(BasketError::InvalidGate));
}

#[test]
fn paused_blocks_creation() {
    let mut env = Env::initialized();
    let admin = env.admin.insecure_clone();
    let ix = env.admin_ix(anchor_lang::InstructionData::data(&basket::instruction::SetPaused { paused: true }));
    env.send_ok(&[ix], &admin, &[]);

    let creator = Keypair::new();
    let payer = env.fund(10 * LAMPORTS);
    let admin_key = env.admin.pubkey();
    let mints: Vec<Pubkey> = (0..2).map(|_| env.create_mint(6, &admin_key)).collect();
    env.whitelist(&mints);
    let bk = book(&mints, &equal_weights(2));
    let args = fixed_args(&creator.pubkey(), 1, &bk);
    let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &args, &bk)];
    env.send_expect_err(&ixs, &payer, &[], err(BasketError::Paused));
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
