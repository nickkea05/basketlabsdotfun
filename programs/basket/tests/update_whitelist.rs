//! `update_whitelist`: sorted insert, update, remove, realloc, authority.

mod common;

use anchor_lang::prelude::Pubkey;
use anchor_lang::solana_program::instruction::Instruction;
use anchor_lang::{InstructionData, ToAccountMetas};
use basket::error::BasketError;
use basket::state::*;
use common::*;
use solana_signer::Signer;

fn whitelist_ix(authority: &Pubkey, add: Vec<WhitelistEntry>, remove: Vec<Pubkey>) -> Instruction {
    Instruction {
        program_id: basket::id(),
        accounts: basket::accounts::UpdateWhitelist {
            authority: *authority,
            config: config_pda(),
            whitelist: whitelist_pda(),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: basket::instruction::UpdateWhitelist { add, remove }.data(),
    }
}

#[test]
fn adds_sorted_updates_tier_and_removes() {
    let mut env = Env::initialized();
    let admin = env.admin.insecure_clone();
    let mints: Vec<Pubkey> = (0..5).map(|_| Pubkey::new_unique()).collect();

    let add = mints.iter().map(|m| WhitelistEntry { mint: *m, tier: 1 }).collect();
    env.send_ok(&[whitelist_ix(&admin.pubkey(), add, vec![])], &admin, &[]);

    let wl: Whitelist = env.load(&whitelist_pda());
    assert_eq!(wl.entries.len(), 5);
    let mut sorted = wl.entries.iter().map(|e| e.mint.to_bytes()).collect::<Vec<_>>();
    sorted.sort();
    assert_eq!(sorted, wl.entries.iter().map(|e| e.mint.to_bytes()).collect::<Vec<_>>());
    for m in &mints {
        assert_eq!(wl.find(m).unwrap().tier, 1);
    }
    assert!(!wl.contains(&Pubkey::new_unique()));
    let size_after_5 = env.account(&whitelist_pda()).unwrap().data.len();
    assert_eq!(size_after_5, Whitelist::space_for(5));

    // Update one tier, remove two, add one: net 4 entries.
    let add = vec![WhitelistEntry { mint: mints[0], tier: 3 }, WhitelistEntry { mint: Pubkey::new_unique(), tier: 0 }];
    env.send_ok(&[whitelist_ix(&admin.pubkey(), add, vec![mints[1], mints[2]])], &admin, &[]);
    let wl: Whitelist = env.load(&whitelist_pda());
    assert_eq!(wl.entries.len(), 4);
    assert_eq!(wl.find(&mints[0]).unwrap().tier, 3);
    assert!(!wl.contains(&mints[1]) && !wl.contains(&mints[2]));
}

#[test]
fn whitelist_authority_can_be_delegated() {
    let mut env = Env::initialized();
    let admin = env.admin.insecure_clone();
    let hot = env.fund(LAMPORTS);
    let m = Pubkey::new_unique();

    // Not yet delegated.
    env.send_expect_err(
        &[whitelist_ix(&hot.pubkey(), vec![WhitelistEntry { mint: m, tier: 0 }], vec![])],
        &hot,
        &[],
        err(BasketError::Unauthorized),
    );

    let ix = env.admin_ix(
        basket::instruction::UpdateConfig {
            update: ConfigUpdate { whitelist_authority: Some(hot.pubkey()), ..Default::default() },
        }
        .data(),
    );
    env.send_ok(&[ix], &admin, &[]);
    env.send_ok(&[whitelist_ix(&hot.pubkey(), vec![WhitelistEntry { mint: m, tier: 0 }], vec![])], &hot, &[]);
    let wl: Whitelist = env.load(&whitelist_pda());
    assert!(wl.contains(&m));
    // Admin still can.
    env.send_ok(&[whitelist_ix(&admin.pubkey(), vec![], vec![m])], &admin, &[]);
    let wl: Whitelist = env.load(&whitelist_pda());
    assert!(!wl.contains(&m));
}

#[test]
fn large_batch_reallocs() {
    let mut env = Env::initialized();
    let admin = env.admin.insecure_clone();
    // 25 entries per tx keeps the instruction under the tx size limit.
    for _ in 0..4 {
        let add = (0..25).map(|_| WhitelistEntry { mint: Pubkey::new_unique(), tier: 0 }).collect();
        env.send_ok(&[whitelist_ix(&admin.pubkey(), add, vec![])], &admin, &[]);
    }
    let wl: Whitelist = env.load(&whitelist_pda());
    assert_eq!(wl.entries.len(), 100);
    assert_eq!(env.account(&whitelist_pda()).unwrap().data.len(), Whitelist::space_for(100));
}
