//! Spike for the change order (docs/program-progress.md §6.3 step 1): drive the
//! mainnet DLMM binary in LiteSVM through the exact call sequence the basket
//! program will make via CPI — pool → two PDA positions (tight + backstop) →
//! bin arrays → add liquidity → swaps that leave the tight range → remove →
//! claim → close — and record CU and rent so the design numbers in §6.2 are
//! measured rather than assumed.

mod common;

use common::*;
use solana_signer::Signer;

const BIN_STEP: u16 = 100; // 1 %
const FEE_BPS: u64 = 100; // 1 %
const SHARE: u64 = 1_000_000; // 6 dp

#[test]
fn dlmm_two_position_lifecycle() {
    let mut env = Env::new();
    // LiteSVM starts at unix_timestamp 0; DLMM's lock check on the active bin
    // treats `now == lock_release_point == 0` as locked. Real clusters never
    // sit at 0, so start the clock at a plausible time.
    env.warp(1_790_000_000);
    let user = env.fund(200 * LAMPORTS);
    let mint_x = env.create_mint(6, &env.admin.pubkey());
    env.mint_to(&mint_x, &user.pubkey(), 1_000_000 * SHARE);
    env.wrap_sol(&user, 150 * LAMPORTS);

    // active_id 0 → 1 lamport per base unit = 0.001 SOL per share.
    let pool = env.create_dlmm_pool(&user, mint_x, BIN_STEP, FEE_BPS, 0);
    let lb = pool.load(&env);
    assert_eq!(lb.active_id, 0);
    assert_eq!(lb.bin_step, BIN_STEP);
    assert_eq!(lb.token_x_mint, mint_x);
    assert_eq!(lb.token_y_mint, WSOL);
    let pool_rent = env.lamports(&pool.lb_pair) + env.lamports(&pool.reserve_x) + env.lamports(&pool.reserve_y) + env.lamports(&pool.oracle);
    eprintln!("pool rent (lb_pair+reserves+oracle) = {} lamports", pool_rent);

    // Backstop [−140, +162] (¼× … 5×), tight [−12, +12].
    let (bs_lo, bs_hi) = (-140, 162);
    let (t_lo, t_hi) = (-12, 12);
    let cus = env.ensure_bin_arrays(&pool, &user, bs_lo, bs_hi);
    eprintln!("bin arrays created: {} CU each: {:?}", cus.len(), cus);
    assert_eq!(cus.len(), 5);
    let array_rent = env.lamports(&pool.bin_array(0));
    eprintln!("bin array rent = {} lamports", array_rent);

    let (tight, cu) = env.init_position_pda(&pool, &user, &user, t_lo, t_hi - t_lo + 1);
    eprintln!("init tight position CU {}", cu);
    let (backstop, cu) = env.init_position_pda(&pool, &user, &user, bs_lo, bs_hi - bs_lo + 1);
    eprintln!("init backstop position CU {}", cu);
    eprintln!(
        "position rent tight = {} backstop = {} lamports",
        env.lamports(&tight),
        env.lamports(&backstop)
    );
    let p = load_position(&env, &backstop);
    assert_eq!((p.lower_bin_id, p.upper_bin_id), (bs_lo, bs_hi));
    assert_eq!(p.owner, user.pubkey());

    // Sleeve: 12 SOL + 12 000 shares into tight, 3 SOL + 3 000 shares into backstop.
    let ix = env.add_liquidity_spot_ix(&pool, &tight, &user.pubkey(), 12_000 * SHARE, 12 * LAMPORTS, 0, t_lo, t_hi);
    let m = env.send_ok(&[ix], &user, &[]);
    eprintln!("add liquidity tight (25 bins) CU {}", m.compute_units_consumed);
    // One call over 303 bins runs the DLMM program out of heap; chunk per bin array.
    let cus = env.add_liquidity_chunked(&pool, &backstop, &user, 3_000 * SHARE, 3 * LAMPORTS, 0, bs_lo, bs_hi);
    eprintln!("add liquidity backstop (303 bins, {} chunks) CU {:?}", cus.len(), cus);
    let (rx, ry) = pool.reserves(&env);
    let x_left = env.token_amount(&Env::ata(&user.pubkey(), &mint_x)) - (1_000_000 - 15_000) * SHARE;
    eprintln!("reserves x={} y={}; undeposited x={} (strategy leftover)", rx, ry, x_left);
    assert!(rx >= 14_000 * SHARE && rx <= 15_000 * SHARE, "x reserve {rx}");
    assert!(ry >= 14 * LAMPORTS + LAMPORTS * 9 / 10 && ry <= 15 * LAMPORTS, "y reserve {ry}");

    // Trader buys 1 SOL of shares: stays inside tight, price ticks up.
    let trader = env.fund(100 * LAMPORTS);
    env.create_ata(&trader.pubkey(), &mint_x);
    env.wrap_sol(&trader, 60 * LAMPORTS);
    let ix = env.swap_ix(&pool, &trader.pubkey(), false, LAMPORTS, 0, -1, 14);
    let m = env.send_ok(&[ix], &trader, &[]);
    let got = env.token_amount(&Env::ata(&trader.pubkey(), &mint_x));
    let active = pool.active_id(&env);
    eprintln!("1 SOL buy: got {} shares, active_id {} CU {}", got / SHARE, active, m.compute_units_consumed);
    assert!(active >= 0 && active <= 2, "1 SOL should move ≤ 2 bins on a 12 SOL tight");
    assert!(got > 950 * SHARE, "≈1 % fee + ≤ 2 % tick");

    // Keeper is late: a 14 SOL buy walks through the whole tight ask side
    // (≈12.7 SOL of shares) and keeps filling from the backstop. A buy larger
    // than *all* asks in the pool (25 SOL here) fails with
    // `BitmapExtensionAccountIsNotProvided` — that is what "stops quoting"
    // looks like on DLMM, and it is the backstop's job to keep it far away.
    let ix = env.swap_ix(&pool, &trader.pubkey(), false, 25 * LAMPORTS, 0, -1, 162);
    env.send_expect_err(&[ix], &trader, &[], 6036);
    let ix = env.swap_ix(&pool, &trader.pubkey(), false, 14 * LAMPORTS, 0, -1, 162);
    let m = env.send_ok(&[ix], &trader, &[]);
    let active = pool.active_id(&env);
    eprintln!("14 SOL buy: active_id {} CU {}", active, m.compute_units_consumed);
    assert!(active > t_hi, "price left the tight range ({active})");
    assert!(active < bs_hi, "still inside the backstop ({active})");
    let (tx, ty) = pool.bin_amounts(&env, t_lo, t_hi);
    eprintln!("tight bins now hold x={} y={}", tx, ty);
    assert_eq!(tx, 0, "tight ask side fully consumed");

    // Another buy above the tight range fills purely from the backstop.
    let before = env.token_amount(&Env::ata(&trader.pubkey(), &mint_x));
    let ix = env.swap_ix(&pool, &trader.pubkey(), false, LAMPORTS, 0, active - 1, bs_hi);
    env.send_ok(&[ix], &trader, &[]);
    let after = env.token_amount(&Env::ata(&trader.pubkey(), &mint_x));
    eprintln!("1 SOL buy from backstop only: {} shares", (after - before) / SHARE);
    assert!(after > before);

    // Recenter step 1: withdraw all of tight (now all SOL) back to the owner.
    let y_before = env.token_amount(&Env::ata(&user.pubkey(), &WSOL));
    let ix = env.remove_liquidity_range_ix(&pool, &tight, &user.pubkey(), t_lo, t_hi, 10_000);
    let m = env.send_ok(&[ix], &user, &[]);
    let y_after = env.token_amount(&Env::ata(&user.pubkey(), &WSOL));
    eprintln!("remove tight (all) CU {} → {} lamports back", m.compute_units_consumed, y_after - y_before);
    assert!(y_after - y_before > 12 * LAMPORTS, "SOL side grew from selling asks");
    let (tx, ty) = pool.bin_amounts(&env, t_lo, t_hi);
    // The backstop also has liquidity in these bins; only tight's share left.
    eprintln!("tight-range bins after withdraw: x={} y={} (backstop's share)", tx, ty);

    // Fees accrued to the tight position are claimable.
    let ix = env.claim_fee_ix(&pool, &tight, &user.pubkey(), t_lo, t_hi);
    let m = env.send_ok(&[ix], &user, &[]);
    eprintln!("claim_fee2 tight CU {}", m.compute_units_consumed);

    // Recenter step 2: close the emptied position (rent back) and open a new
    // one around the current active bin, then re-add the withdrawn SOL.
    let rent_before = env.lamports(&user.pubkey());
    let ix = env.dlmm_close_position_ix(&tight, &user.pubkey(), &user.pubkey());
    let m = env.send_ok(&[ix], &user, &[]);
    eprintln!("close_position2 CU {}", m.compute_units_consumed);
    assert!(env.account(&tight).is_none());
    assert!(env.lamports(&user.pubkey()) > rent_before, "position rent refunded");

    let active = pool.active_id(&env);
    let (n_lo, n_hi) = (active - 12, active + 12);
    let (new_tight, _) = env.init_position_pda(&pool, &user, &user, n_lo, 25);
    let (_, y_bs) = pool.bin_amounts(&env, n_lo, active);
    let ix = env.add_liquidity_ix(
        &pool,
        &new_tight,
        &user.pubkey(),
        0,
        12 * LAMPORTS,
        active,
        n_lo,
        n_hi,
        basket::dlmm::lb_clmm::types::StrategyType::SpotImBalanced,
    );
    let m = env.send_ok(&[ix], &user, &[]);
    eprintln!("re-add tight one-sided (SOL only, imbalanced) CU {}", m.compute_units_consumed);
    let (_, ty) = pool.bin_amounts(&env, n_lo, active);
    eprintln!("bid side below active: {} lamports (backstop had {})", ty, y_bs);
    assert!(ty - y_bs >= 12 * LAMPORTS * 99 / 100, "bid side re-placed below active ({ty})");

    // What locks the active bin: our own add in this slot, or any swap? Move to
    // a fresh slot (so the add is old), swap a little, then try to withdraw.
    env.warp(1);
    let ix = env.swap_ix(&pool, &trader.pubkey(), true, 10 * SHARE, 0, active - 13, active + 13);
    env.send_ok(&[ix], &trader, &[]);
    let ix = env.remove_liquidity_range_ix(&pool, &new_tight, &user.pubkey(), n_lo, n_hi, 10_000);
    let r = env.send(&[ix], &user, &[]);
    eprintln!(
        "remove new tight in the same slot as a stranger's swap: {:?}",
        r.as_ref().map(|m| m.compute_units_consumed).map_err(|e| e.err.clone())
    );
    env.warp(1);
    let ix = env.remove_liquidity_range_ix(&pool, &new_tight, &user.pubkey(), n_lo, n_hi, 10_000);
    let r2 = env.send(&[ix], &user, &[]);
    eprintln!(
        "remove new tight one slot later: {:?}",
        r2.as_ref().map(|m| m.compute_units_consumed).map_err(|e| e.err.clone())
    );
    if r.is_ok() {
        // Then put it back so the rest of the flow is unchanged.
        let ix = env.add_liquidity_ix(&pool, &new_tight, &user.pubkey(), 0, 12 * LAMPORTS, pool.active_id(&env), n_lo, n_hi, basket::dlmm::lb_clmm::types::StrategyType::SpotImBalanced);
        env.send_ok(&[ix], &user, &[]);
    }

    // Backstop: withdraw all, claim, close — the close path.
    // Lesson recorded while getting here: a position cannot remove liquidity
    // from the *active* bin in the same slot it added liquidity there
    // (`LiquidityLocked`, 6055 — DLMM's JIT guard, position-scoped; a stranger's
    // swap does not trigger it, see above). Everything here is ≥ 1 slot old.
    // A whole-range remove over 303 bins costs ≈ 1.2M CU (≈ 260–330k per
    // 70-bin chunk), so the basket program must chunk it per bin array.
    let ix = env.remove_liquidity_range_ix(&pool, &backstop, &user.pubkey(), bs_lo, bs_hi, 10_000);
    let m = env.send_ok(&[ix], &user, &[]);
    eprintln!("remove backstop whole range (303 bins) CU {}", m.compute_units_consumed);
    let (bx, by) = pool.reserves(&env);
    eprintln!("reserves after full withdraw x={} y={}", bx, by);
    assert!(bx < 2 * SHARE, "only per-bin rounding dust left ({bx})");
    let ix = env.claim_fee_ix(&pool, &backstop, &user.pubkey(), bs_lo, bs_hi);
    let m = env.send_ok(&[ix], &user, &[]);
    eprintln!("claim_fee2 backstop CU {}", m.compute_units_consumed);
    let ix = env.dlmm_close_position_ix(&backstop, &user.pubkey(), &user.pubkey());
    env.send_ok(&[ix], &user, &[]);
    assert!(env.account(&backstop).is_none());
    // Bin arrays stay (no permissionless close).
    assert!(env.account(&pool.bin_array(2)).is_some());
}
