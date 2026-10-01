//! `initialize_config`, `update_config`, `rotate_keepers`, `set_paused`.

mod common;

use anchor_lang::InstructionData;
use anchor_lang::ToAccountMetas;
use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer;

#[test]
fn initializes_with_defaults() {
    let env = Env::initialized();
    let cfg: Config = env.load(&config_pda());
    assert_eq!(cfg.admin, env.admin.pubkey());
    assert_eq!(cfg.treasury, env.treasury.pubkey());
    assert_eq!(cfg.whitelist_authority, env.admin.pubkey());
    assert_eq!(cfg.keepers, vec![env.keeper.pubkey()]);
    assert_eq!(cfg.cluster, 2);
    assert!(!cfg.paused);
    assert_eq!(cfg.fees, FeeDefaults::default());
    assert_eq!(cfg.fees.mint_fee_bps, 100);
    assert_eq!(cfg.fees.redeem_fee_bps, 25);
    // Change order §4: creator 20 / buyback 50 / team 25 / prizes 5.
    assert_eq!(
        (cfg.fees.creator_split_bps, cfg.fees.buyback_split_bps, cfg.fees.team_split_bps, cfg.fees.prize_split_bps),
        (2_000, 5_000, 2_500, 500)
    );
    assert_eq!(cfg.fees.creation_fee_lamports, LAMPORTS / 10);
    assert_eq!(cfg.sleeve.step_r_bps, 2_500);
    assert_eq!(cfg.sleeve.step_threshold_lamports, 20 * LAMPORTS);
    assert_eq!(cfg.sleeve.profiles[basket::constants::PROFILE_FIXED_INDEX as usize].default_r_bps, 700);
    assert_eq!(cfg.sleeve.profiles[basket::constants::PROFILE_FIXED_MEME as usize].default_r_bps, 2_000);
    assert_eq!(cfg.managed.timelock_s, 24 * 3600);
    assert_eq!(cfg.managed.turnover_cap_bps, 3_000);

    // Change order §2–§5 / progress §6.4a.
    assert_eq!(cfg.team_wallet, env.treasury.pubkey(), "devnet placeholder until the Squads vault exists");
    assert_eq!(cfg.deposit_lamports, LAMPORTS);
    assert_eq!(cfg.mint_window, MintWindow::default());
    assert_eq!(cfg.mint_window.default_s, 48 * 3600);
    assert!(!cfg.allow_open_gate);
    assert_eq!(cfg.bskt_mint, None);
    assert_eq!(cfg.creator_tier_bps, [2_000; basket::constants::TIER_COUNT]);
    assert_eq!(cfg.swap_programs, vec![basket::constants::JUPITER_V6_ID]);
    let p = &cfg.pools;
    assert_eq!(p.recenter_trigger_bps, 7_000);
    assert_eq!(p.recenter_min_interval_s, 300);
    assert_eq!(p.backstop_slice_bps, 2_000);
    let index = p.presets[basket::constants::PROFILE_FIXED_INDEX as usize];
    assert_eq!((index.bin_step, index.base_fee_bps, index.tight_half_width_bins, index.backstop_half_width_bins), (25, 25, 31, 135));
    assert_eq!(index.reset_confirm_checks, 3);
    let mirror = p.presets[basket::constants::PROFILE_MIRROR as usize];
    assert_eq!((mirror.bin_step, mirror.base_fee_bps, mirror.tight_half_width_bins, mirror.backstop_half_width_bins), (50, 50, 23, 81));
    assert_eq!(p.presets[basket::constants::PROFILE_STRATEGY as usize], mirror);
    assert_eq!(p.presets[basket::constants::PROFILE_MANAGED as usize], mirror);
    let meme = p.presets[basket::constants::PROFILE_FIXED_MEME as usize];
    assert_eq!((meme.bin_step, meme.base_fee_bps, meme.tight_half_width_bins, meme.backstop_half_width_bins), (100, 100, 14, 53));
    assert_eq!(meme.reset_confirm_checks, 1);
    assert!(p.presets.iter().all(|p| p.backstop_max_arrays == basket::constants::MAX_BACKSTOP_ARRAYS));
    assert_eq!(cfg.prizes.epoch_s, PrizeParams::default().epoch_s);
    assert!(cfg.prizes.validate());

    let wl: Whitelist = env.load(&whitelist_pda());
    assert!(wl.entries.is_empty());
}

#[test]
fn cannot_initialize_twice() {
    let mut env = Env::initialized();
    assert!(env.initialize_config(ConfigUpdate::default()).is_err());
}

#[test]
fn rejects_bad_fee_split_on_init() {
    let mut env = Env::new();
    let mut fees = FeeDefaults::default();
    fees.buyback_split_bps = 6_000; // sums to 11_000
    let r = env.initialize_config(ConfigUpdate { fees: Some(fees), ..Default::default() });
    assert!(format!("{:?}", r.unwrap_err().err).contains(&format!("Custom({})", err(BasketError::FeeOutOfBounds))));
}

#[test]
fn update_validates_the_change_order_fields() {
    let mut env = Env::initialized();
    let admin = env.admin.insecure_clone();
    let send = |env: &mut Env, update: ConfigUpdate, expect: Option<u32>| {
        let ix = env.admin_ix(basket::instruction::UpdateConfig { update }.data());
        match expect {
            None => {
                env.send_ok(&[ix], &admin, &[]);
            }
            Some(code) => env.send_expect_err(&[ix], &admin, &[], code),
        }
    };

    // Pool presets: tight must fit one position, backstop wider than tight, ≤ 4 arrays.
    let mut pools = PoolDefaults::default();
    pools.presets[0].tight_half_width_bins = 35; // 71 bins > 70
    send(&mut env, ConfigUpdate { pools: Some(pools), ..Default::default() }, Some(err(BasketError::InvalidArgument)));
    let mut pools = PoolDefaults::default();
    pools.presets[0].backstop_half_width_bins = pools.presets[0].tight_half_width_bins;
    send(&mut env, ConfigUpdate { pools: Some(pools), ..Default::default() }, Some(err(BasketError::InvalidArgument)));
    let mut pools = PoolDefaults::default();
    pools.presets[0].backstop_max_arrays = basket::constants::MAX_BACKSTOP_ARRAYS + 1;
    send(&mut env, ConfigUpdate { pools: Some(pools), ..Default::default() }, Some(err(BasketError::InvalidArgument)));
    let mut pools = PoolDefaults::default();
    pools.backstop_slice_bps = 10_000;
    send(&mut env, ConfigUpdate { pools: Some(pools), ..Default::default() }, Some(err(BasketError::InvalidArgument)));
    let mut pools = PoolDefaults::default();
    pools.recenter_trigger_bps = 8_000;
    pools.recenter_min_interval_s = 60;
    send(&mut env, ConfigUpdate { pools: Some(pools), ..Default::default() }, None);
    assert_eq!(env.config().pools.recenter_trigger_bps, 8_000);

    // Mint window: min ≤ default ≤ max.
    let w = MintWindow { default_s: 3600, min_s: 7200, max_s: 86400 };
    send(&mut env, ConfigUpdate { mint_window: Some(w), ..Default::default() }, Some(err(BasketError::InvalidGate)));
    let w = MintWindow { default_s: 7200, min_s: 3600, max_s: 86400 };
    send(&mut env, ConfigUpdate { mint_window: Some(w), ..Default::default() }, None);

    // Prize split must sum to 100 % over the recipient count.
    let mut prizes = PrizeParams::default();
    prizes.split_bps[0] += 1;
    send(&mut env, ConfigUpdate { prizes: Some(prizes), ..Default::default() }, Some(err(BasketError::InvalidArgument)));
    let mut prizes = PrizeParams::default();
    prizes.max_recipients = 0;
    send(&mut env, ConfigUpdate { prizes: Some(prizes), ..Default::default() }, Some(err(BasketError::InvalidArgument)));

    // Tier ladder entries are bps.
    let mut tiers = [2_000u16; basket::constants::TIER_COUNT];
    tiers[3] = 10_001;
    send(&mut env, ConfigUpdate { creator_tier_bps: Some(tiers), ..Default::default() }, Some(err(BasketError::FeeOutOfBounds)));
    tiers[3] = 3_500;
    send(&mut env, ConfigUpdate { creator_tier_bps: Some(tiers), ..Default::default() }, None);
    assert_eq!(env.config().creator_bps_for_tier(3), 3_500);
    assert_eq!(env.config().creator_bps_for_tier(200), 3_500, "tiers above the ladder clamp to the top");

    // Deposit, team wallet, open-gate flag.
    let team = Pubkey::new_unique();
    send(
        &mut env,
        ConfigUpdate { deposit_lamports: Some(2 * LAMPORTS), team_wallet: Some(team), allow_open_gate: Some(true), ..Default::default() },
        None,
    );
    let cfg = env.config();
    assert_eq!((cfg.deposit_lamports, cfg.team_wallet, cfg.allow_open_gate), (2 * LAMPORTS, team, true));
}

#[test]
fn bskt_mint_is_set_exactly_once() {
    let mut env = Env::initialized();
    let admin = env.admin.insecure_clone();
    let mint = Pubkey::new_unique();
    let stranger = env.fund(LAMPORTS);
    let ix = Instruction {
        program_id: basket::id(),
        accounts: basket::accounts::AdminOnly { admin: stranger.pubkey(), config: config_pda() }.to_account_metas(None),
        data: basket::instruction::SetBsktMint { mint }.data(),
    };
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));

    let ix = env.admin_ix(basket::instruction::SetBsktMint { mint }.data());
    env.send_ok(&[ix], &admin, &[]);
    assert_eq!(env.config().bskt_mint, Some(mint));
    let ix = env.admin_ix(basket::instruction::SetBsktMint { mint: Pubkey::new_unique() }.data());
    env.send_expect_err(&[ix], &admin, &[], err(BasketError::BsktMintState));
    assert_eq!(env.config().bskt_mint, Some(mint));
}

#[test]
fn admin_updates_config_and_others_cannot() {
    let mut env = Env::initialized();
    let new_treasury = Pubkey::new_unique();
    let ix = env.admin_ix(
        basket::instruction::UpdateConfig {
            update: ConfigUpdate { treasury: Some(new_treasury), ..Default::default() },
        }
        .data(),
    );
    let admin = env.admin.insecure_clone();
    env.send_ok(&[ix], &admin, &[]);
    let cfg: Config = env.load(&config_pda());
    assert_eq!(cfg.treasury, new_treasury);

    // Someone else, same instruction shape.
    let stranger = env.fund(LAMPORTS);
    let ix = Instruction {
        program_id: basket::id(),
        accounts: basket::accounts::AdminOnly { admin: stranger.pubkey(), config: config_pda() }.to_account_metas(None),
        data: basket::instruction::UpdateConfig {
            update: ConfigUpdate { treasury: Some(Pubkey::new_unique()), ..Default::default() },
        }
        .data(),
    };
    env.send_expect_err(&[ix], &stranger, &[], err(BasketError::Unauthorized));
}

#[test]
fn rotate_keepers_and_pause() {
    let mut env = Env::initialized();
    let k1 = Pubkey::new_unique();
    let k2 = Pubkey::new_unique();
    let admin = env.admin.insecure_clone();
    let ix = env.admin_ix(basket::instruction::RotateKeepers { keepers: vec![k1, k2] }.data());
    env.send_ok(&[ix], &admin, &[]);
    let cfg: Config = env.load(&config_pda());
    assert_eq!(cfg.keepers, vec![k1, k2]);
    assert!(cfg.is_keeper(&k1) && !cfg.is_keeper(&env.keeper.pubkey()));

    let ix = env.admin_ix(basket::instruction::SetPaused { paused: true }.data());
    env.send_ok(&[ix], &admin, &[]);
    let cfg: Config = env.load(&config_pda());
    assert!(cfg.paused);

    // Too many keepers.
    let many: Vec<Pubkey> = (0..9).map(|_| Pubkey::new_unique()).collect();
    let ix = env.admin_ix(basket::instruction::RotateKeepers { keepers: many }.data());
    env.send_expect_err(&[ix], &admin, &[], err(BasketError::InvalidArgument));
}

#[test]
fn update_validates_sleeve_band() {
    let mut env = Env::initialized();
    let mut sleeve = SleeveDefaults::default();
    sleeve.profiles[0].min_r_bps = 1_200; // min > default
    let ix = env.admin_ix(
        basket::instruction::UpdateConfig { update: ConfigUpdate { sleeve: Some(sleeve), ..Default::default() } }
            .data(),
    );
    let admin = env.admin.insecure_clone();
    env.send_expect_err(&[ix], &admin, &[], err(BasketError::SleeveOutOfBand));
}
