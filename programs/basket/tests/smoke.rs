//! Workspace smoke test: the program and its fixtures load into an
//! in-process SVM and are executable. Real instruction tests live beside this
//! file, one per instruction.

mod common;

use common::*;

#[test]
fn programs_load_into_svm() {
    let env = Env::new();
    for id in [basket::id(), METAPLEX_ID, CP_AMM_ID] {
        let account = env.svm.get_account(&id).expect("program account exists");
        assert!(account.executable, "{id} not executable");
    }
}

#[test]
fn constants_match_the_js_schema() {
    assert_eq!(basket::constants::BPS_TOTAL, 10_000);
    assert_eq!(basket::constants::BASKET_TYPE_FIXED, 0);
    assert_eq!(basket::constants::BASKET_TYPE_MIRROR, 1);
    assert_eq!(basket::constants::BASKET_TYPE_MANAGED, 2);
    assert_eq!(basket::constants::BASKET_TYPE_STRATEGY, 3);
    assert_eq!(basket::constants::MAX_ASSETS_FIXED, 20);
    assert_eq!(basket::constants::NAME_MAX, 32);
    assert_eq!(basket::constants::SYMBOL_MAX, 10);
}
