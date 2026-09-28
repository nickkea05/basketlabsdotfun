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
    assert_eq!(
        cfg.fees.holder_split_bps + cfg.fees.creator_split_bps + cfg.fees.protocol_split_bps,
        10_000
    );
    assert_eq!(cfg.sleeve.step_r_bps, 2_500);
    assert_eq!(cfg.sleeve.step_threshold_lamports, 20 * LAMPORTS);
    assert_eq!(cfg.sleeve.profiles[basket::constants::PROFILE_FIXED_INDEX as usize].default_r_bps, 700);
    assert_eq!(cfg.sleeve.profiles[basket::constants::PROFILE_FIXED_MEME as usize].default_r_bps, 2_000);
    assert_eq!(cfg.managed.timelock_s, 24 * 3600);
    assert_eq!(cfg.managed.turnover_cap_bps, 3_000);

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
    fees.holder_split_bps = 5_000; // sums to 11_000
    let r = env.initialize_config(ConfigUpdate { fees: Some(fees), ..Default::default() });
    assert!(format!("{:?}", r.unwrap_err().err).contains(&format!("Custom({})", err(BasketError::FeeOutOfBounds))));
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
