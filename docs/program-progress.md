# basketlabs.fun — program build progress (handoff)

Living log for the `basket` Anchor program build. Spec is
`docs/program-build-confirmation.md` (D1–D20, §1–§9) **as amended by
`docs/change-order-liquidity-and-fees.md`** (2026-09-30; wins on conflict). Rule of
the build: tests written before handlers, LiteSVM, one test file per instruction, and
any design question the spec leaves open gets **asked, not decided** (§6 below).

Last updated: 2026-09-30, late (change order **built**: §6.3 steps 1–11 done, all
tests green on the DLMM fixture, `anchor build` exit 0, pushed `858434b`). §3b is the
"as built" for the change order, §4 the re-measured budgets, **§6.5 the decisions I had
to take that the change order does not cover — read that first.**

---

## 1. Where things stand

| Phase | Scope | State |
| --- | --- | --- |
| 0 | Workspace, spike (`declare_program!(cp_amm)`, anchor-spl metadata, U256 math under SBF) | done |
| 1 | `initialize_config`, `update_whitelist`, `create_basket` (ed25519 introspection, chain-hash book, replay binding), `add_positions` | done, green |
| 2 | `seed` + `mint` with DAMM v2 CPI, sleeve step rule, treasury-share accounting | done, green |
| 3 | `redeem`, `redeem_begin` / `redeem_components`, `claim_frozen` (FrozenClaim) | done, green (`de7a4b1`) |
| 4 | FeeVault, `claim_pool_fees`, `sweep_fees`, 40/30/30 split | done, green (`4abcb7d`) |
| 5 | `crystallize` + HWM, management fee (gates/schedules already enforced in mint) | done, green (`9b952a4`) |
| 6 | `submit_book` / `open_position` / `apply_book` / `execute_swap` / `close_position` / `finalize_rebalance` | done, green (`ccf2364`) |
| 7 | `post_rewards_root` / `distribute_rewards` / `close_rewards_root`, `lock_creator_shares` / `unlock_creator_shares`, `close_positions` / `close_basket` (pause/admin were done in phase 1) | done, green (`1922db4`) |
| 7b | **Change order** (DAMM v2 → DLMM `tight` + `backstop`, deployer-signed create + 1 SOL deposit, fee split 20/50/25/5, rewards removed, Window→Closed default, buyback + prize vaults, keeper ixs) | done, green (`858434b`); §3b, §6.5 |
| 8 | Devnet: deploy, CU measurements on a real validator, batch UX end to end, indexer event coverage | not started — after Nick signs off §6.5 |
| wrap | README/TODO refresh, present §5 to Nick | done; §5 answered 2026-09-30 |

Last green run: **115/115** in 24 files — unit 7, add_positions 6, apply_book 3,
backstop 6, buyback 3, claim_pool_fees 6, close_basket 4, create_basket 9,
creator_lock 2, crystallize 6, dlmm_spike 1, execute_swap 4, finalize_rebalance 4,
initialize_config 8, mint 9, prizes 3, recenter_tight 4, redeem 8, seed 5,
settle_fees 2, smoke 2, submit_book 5, sweep_fees 5, update_whitelist 3.

## 2. Restart checklist

From `c:\dev\ponsclone\basketfun`:

1. Move stale test exes out of the way (McAfee holds them → `LNK1104`):
   `Get-ChildItem target\debug\deps\*.exe | Move-Item -Destination $env:TEMP\stale -Force`
2. `anchor build` — exit 0 only once every file in `tests/` compiles (the IDL step
   builds them). If it fails with a link error, repeat step 1 and rerun.
3. `cargo.exe test -p basket` (always `cargo.exe`: a stray 0-byte
   `C:\Windows\system32\cargo` triggers an "open with" popup). Tests pass in
   parallel; a full run is ~70 s.
4. Fixtures: `.\scripts\fetch-fixtures.ps1` dumps `mpl_token_metadata.so` and
   `lb_clmm.so` (DLMM). The DLMM IDL is `idls/lb_clmm.json` (dlmm-sdk
   `idls/dlmm.json`, program v0.12.0; the on-chain IDL account does not exist,
   `anchor idl fetch` fails).
5. PowerShell 5.1 gotcha: `Set-Content -Encoding utf8` writes a BOM and rustc rejects
   it; write files with `[System.IO.File]::WriteAllText(path, text, UTF8Encoding $false)`.
6. Next up: Nick confirms or overturns **§6.5**, then Phase 8 (devnet). Devnet needs
   a program keypair outside the repo, `anchor keys sync`, Meteora DLMM on devnet
   (same program id as mainnet) + Jupiter (no devnet deployment — rebalance tests
   there use the DLMM venue path as LiteSVM does), and the TS client in
   `packages/solana` generated from the IDL.

## 3. Design as built (phases 3–6)

### Redeem (Phase 3)
- Gross `shares` from holder; fee (`fees.redeem_fee_bps`) transferred as shares to
  the FeeVault share ATA, net burned. No gate, no pause check (D9).
- `H = supply − position_shares (rounded UP) + pending_redeem_shares`. Round-up
  mirrors cp-amm's deposit rounding so `H` is exact.
- Pool leg: `remove_liquidity(L·net/H)`, `token_b_account` = holder wSOL ATA
  (created idempotently, closed to SOL if we created it), `token_a_account` = basket
  share ATA, then burned (treasury shares).
- Components: `available·net/H` per leg (`available = vault − Position.owed`).
  Frozen vault or frozen holder ATA → `FrozenClaim` PDA (rent from holder),
  `Position.owed += amount`; `claim_frozen` pays it later.
- `redeem_begin` / `redeem_components(count)` for big books via `Redemption` PDA;
  `pending_redeem_shares` keeps the denominator exact between the two.

### Fees (Phase 4)
- Mint/redeem fee shares sit unsplit in the FeeVault share ATA. Pool swap fees
  (`OnlyB` = SOL side) are claimed by `claim_pool_fees` (anyone) through a temporary
  wSOL ATA that is closed onto the FeeVault PDA as native SOL; rent back to cranker.
- `sweep_fees` (anyone) splits new shares + SOL by the basket-frozen 40/30/30;
  holder line stays in the vault as `holder_reserve_*` for `distribute_rewards`;
  creator SOL is held back in `creator_owed_lamports` if paying it would leave the
  creator wallet below rent exemption (a sweep never reverts on creator state).

### Managed fees (Phase 5)
- `crystallize(nav_lamports_per_share)`: keeper-signed NAV attestation (lamports per
  whole share, `SHARE_DECIMALS = 6`), once per `crystallize_period_s`.
- Management fee `s = H·f/(1−f)`, `f = bps·elapsed/year`, accrued at `crystallize`
  and `apply_book` (`accrue_mgmt_fee`).
- Performance fee `s = perf·(nav−hwm)·H/(nav − perf·(nav−hwm))` (dilution-correct);
  HWM moves to the post-fee `diluted_nav`. `seed` sets the opening HWM to
  `(implied_total − creation_fee)·1e6/initial_shares`.

### Books / rebalance (Phase 6)
- `PendingBook { payer, submitted_at, ready_at, book_hash, book: Vec<PositionArg> ≤ 64 }`
  holds the target list for every mutable type. Rent to the submitter, refunded at
  `finalize_rebalance`; a replaced book keeps the original payer.
- `submit_book`: Managed → creator, others → keeper; Fixed → `ImmutableBasket`.
  Validates weights (>0, no dupes, Σ = 10_000, whitelisted). Refused while a window
  is open; an *expired* window is abandoned and the new target starts `seq + 1`.
  Optional remaining `[mint, position, vault]` triplets create Positions for entering
  mints (weights from the book, `index = position_count++`). Mirror/Strategy open the
  window immediately (`window_end = now + config.rebalance_window_s`); Managed sets
  `ready_at = now + timelock`.
- `open_position` (anyone): same Position creation for a pending-book mint that was
  not passed at submit. `OffBook` if not in the pending book, `DuplicateMint` if it
  exists.
- `apply_book(current_book)`: Managed only, anyone may call after `ready_at`. Caller
  supplies the live book, checked against `book_hash`. Turnover `Σ|Δw|/2` against
  `managed.turnover_cap_bps` per `turnover_window_s` (window starts at first apply,
  resets when expired). Management fee accrues (pre-trade H). Window opens.
- `execute_swap(amount_in, min_amount_out, data)`: keeper, inside the window, venue in
  `Config.swap_programs` (default Jupiter v6; tests allow cp-amm). Remaining accounts
  are re-issued as the inner instruction with the basket PDA as signer. Guards:
  no other basket-owned token account in the inner account list
  (`UnexpectedAccountInSwap`), basket lamports may not fall, `in_vault` drops by at
  most `amount_in`, `out_vault` rises by at least `min_amount_out`, out mint must be
  in the target (`OffBook`), cumulative per-position sell cap
  `(w_old − w_new)/w_old + tolerance` (tolerance alone when weight rises; 100% when
  leaving) on `vault_before + sold_so_far`, scoped by `Position.rebalance_seq`.
- `close_position`: keeper, rebalance active, mint not in target, vault and `owed`
  empty; closes vault + Position, rent → `basket.payer`, `position_count −= 1`.
- `finalize_rebalance(entries)`: keeper, chunkable. Remaining `[position]` per entry;
  sets `weight_bps`/`index`, walks `acc_hash/acc_count/acc_weight`. Over-count →
  `BookHashMismatch`. Last chunk: `position_count == target_count` (else
  `PositionsNotClosed`), `acc_hash == target_hash`, `acc_weight == 10_000`; then
  `book_hash`/`book_acc`/`asset_count` flip, `rebalance.active = false`, PendingBook
  closed to its payer. A wrong earlier chunk can only be recovered by letting the
  window expire and resubmitting (acc is reset by `submit_book`).
- `mint` is refused while `rebalance.active` (`RebalanceActive`); `redeem` is not.

### Rewards, creator lock, close (Phase 7)
- `post_rewards_root(epoch, root, reward_mint, total_amount, leaf_count)`: keeper,
  creates `RewardsRoot ["rewards", share_mint, epoch_le]` (bitmap sized to
  `leaf_count`, ≤ 65_536). `reward_mint` is the share mint or the native mint; the
  amount is debited from `FeeVault.holder_reserve_{shares,lamports}` at post time so
  two roots cannot promise the same funds. Same epoch twice → `ReplayDetected`.
  Leaf = `sha256(0x00 ‖ index_le ‖ wallet ‖ amount_le)`, node = `sha256(0x01 ‖ min ‖ max)`.
- `distribute_rewards(index, amount, proof)`: anyone (keeper pushes). Pays from the
  FeeVault (shares: to ATA(wallet) created by the caller; SOL: lamport move, refused
  with `BelowMinimum` if the wallet would end below rent exemption). Sets the bit,
  closes the root to its payer when fully paid.
- `close_rewards_root`: keeper, when fully paid or `posted_at + 30 d`; unpaid
  remainder returns to the reserve.
- `lock_creator_shares(amount, duration_s)` / `unlock_creator_shares`: creator only;
  1 d ≤ duration ≤ 4 y; top-ups add and extend (`unlock_at = max`), never shorten;
  unlock returns everything and closes ATA + PDA. Not pausable.
- Close crank, two instructions, both keeper-only and both gated by the same
  precondition: no rebalance, no pending redemption, no PendingBook, idle ≥
  `config.close_idle_s`, and holder shares `H ≤ FeeVault share balance + 10`
  (i.e. nobody outside the fee stash holds anything; the +10 base units cover
  liquidity→amount rounding in the pool position).
  - `close_positions` (chunked, remaining `[position, vault, mint, treasury_ata]`):
    vault dust → treasury ATA (keeper pays creation), vault + Position closed → rent
    to `basket.payer`.
  - `close_basket` (position_count == 0): seeded → `remove_liquidity` (all) with SOL
    to ATA(treasury, wSOL), `claim_position_fee`, cp-amm `close_position` (rent →
    payer), withdrawn shares burned, FeeVault fee shares burned + ATA closed,
    FeeVault free SOL: `creator_owed` to creator (if rent-safe) and the rest to the
    treasury, FeeVault + Basket closed → payer. Unseeded baskets pass `None` for
    the pool/FeeVault accounts. Supply must end ≤ 10 base units (pool vault floor).
- Runtime lesson: program-owned account closes (lamport moves without CPI) must
  happen **after the last CPI** in the instruction, or the CPI boundary check
  reports `UnbalancedInstruction`.

## 3b. Change order as built (Phase 7b) — supersedes §3 where they differ

Everything cp-amm in §3 (position NFT, `OnlyB`, 40/30/30, holder rewards,
`holder_reserve_*`, creator-signature replay path) is gone. What stands:

- **Pool.** One DLMM pool per basket, `share_mint` = X, wSOL = Y, created in
  `create_basket` (`initialize_customizable_permissionless_lb_pair2`, funder = basket
  PDA, `collect_fee_mode = OnlyY`, preset by `PROFILE_*` from `Config.pools`). Pool rent
  (0.0346 SOL) is spent from the deployer's **1 SOL deposit** right there
  (`deposit_lamports` / `deposit_spent_lamports`, `DepositSpent` events).
- **Two positions**, both PDAs owned by the basket (`initialize_position_pda`, seeds
  `["position", lb_pair, basket, lower, width]`):
  - `tight` = `[active − w, active + w]` (index preset w = 31 → 63 bins, one position);
    opened in `seed` from the first buy, bin arrays (2 × 0.0714 SOL) paid by the buyer
    and reimbursed from the deposit. Re-centred by the keeper (`recenter_tight`).
  - `backstop` = whole bin arrays around the launch bin (≤ 4), one position created
    70 wide and grown one bin array per `fund_backstop` (DLMM caps realloc at ~10 KB
    per tx). Placed by the keeper (`place_backstop`: creates the position, reserves
    the arrays, rent from the deposit), funded in **ascending array order**
    (`fund_backstop(array_index)`, moves the idle 20 % slice of the sleeve into the
    array's bins, keeper pays the position growth rent — refundable), then `LIVE`.
    `withdraw_backstop(array_index)` is allowed when the active bin is outside the
    backstop range (reset) or when the gate is closed and the basket has been idle
    `close_idle_s` (wind-down); `close_backstop` once empty; `place_backstop` again
    re-places around the current active bin. States: `UNPLACED → FUNDING → LIVE →
    WITHDRAWING → CLOSED (→ FUNDING …)`.
- **Sleeve accounting.** `holder_shares H = supply − shares in tight − shares in
  backstop − idle shares + pending_redeem_shares` (bin amounts from the position's
  liquidity shares over the bin arrays passed in). Mint: 80 % of the sleeve into
  `tight`, 20 % into the backstop **only if the backstop is LIVE, contains the active
  bin, and the active bin is inside tight** — then as a 35-bin top-up around the
  active bin inside tight's own bin arrays (CU/lock budget); otherwise the 20 % stays
  idle on the basket ATAs and `fund_backstop`/`recenter_tight` pick it up. Redeem:
  `remove_liquidity_by_range2(bps = ⌊net·10⁴/H⌋)` on tight only, shortfall from idle
  SOL, components `vault·net/H`; never gated; **`h = max(h, net)`** so the last
  holder's full redeem cannot fail on bin-share rounding (§6.5).
- **Keeper loop.** `recenter_tight`: allowed when the active bin is out of tight, or
  past 70 % of the half-width **and** ≥ 300 s since `last_recenter_ts` (set at seed);
  removes all of tight, closes the position, opens `[active − w, active + w]`, adds
  the withdrawn amounts (+ idle only while the backstop is LIVE), and while the mint
  gate is open mints treasury shares at the active-bin price to rebuild the ask side
  up to the target X. Position rent round-trips through the keeper. `reset_backstop`
  = `withdraw_backstop` × arrays + `close_backstop` + `place_backstop` + `fund_backstop`
  × arrays (keeper-side confirm count).
- **Fees.** `claim_pool_fees(position, min_bin, max_bin)` (anyone, either position,
  backstop one array at a time): wSOL → native SOL on the FeeVault, routed creator /
  BUYBACK / team / PRIZE (creator = `basket.fees.creator_split_bps` from the attested
  tier, the other three share the rest 50/25/5), `creator_owed_lamports` held back
  when the wallet is not rent-safe, `unrouted_lamports` for SOL the keeper parks
  (recenter claims). Per-basket epoch counters `fee_epoch / fee_epoch_lamports /
  fee_prev_epoch_lamports` for prizes. `sweep_fees` (anyone): creator 20 % of the fee
  shares transferred as shares; 80 % burned and redeemed in kind (tight + idle SOL
  leg routed 3 ways at once, components into FeeVault ATAs via a `Redemption` PDA
  and `sweep_fees_components` chunks). `settle_fees(amount_in, min_out, data)`
  (keeper): venue swap of one FeeVault component ATA → wSOL → routed 3 ways.
- **Treasury vaults.** `init_treasury_vaults` (admin, once): `BuybackVault ["buyback"]`,
  `PrizeVault ["prize"]`. `set_bskt_mint` once. `stage_buyback(lamports)` moves PDA SOL
  onto ATA(vault, wSOL); `execute_buyback` syncs, venue-swaps to $BSKT, burns the
  whole ATA. `post_prize_payout(epoch, total)` with `[wallet, lock, basket]` triplets.
- **create_basket.** Deployer signs and pays; optional keeper ed25519 `TierAttestation`
  `(creator, tier, expiry_slot)`; ≤ 6 positions in the launch tx (DLMM pool init eats
  the trace budget), rest via `add_positions` (8 per tx). `payer_wsol_ata` must exist
  (the frontend creates it; the program only tops it up by 1 lamport). Gate default
  `WindowUntil(now + 48 h)`, bounds 24–72 h, `Open` only behind `allow_open_gate`.
- **Close.** `close_positions` → `close_basket` (tight withdrawn, fees claimed,
  position closed, shares burned, SOL dust to the treasury, FeeVault creator line paid,
  Basket closed: payer gets rent + `deposit − spent`) → `close_fee_vault` once the
  keeper has settled the FeeVault's component ATAs. Backstop must be `UNPLACED` or
  `CLOSED` first (wind-down path above). The fee shares count as outstanding: sweep,
  then the creator redeems their cut (the harness loops redeem/sweep until nothing
  is left — three rounds in practice).

## 4. Measurements and hard limits (keep in README) — DLMM build, LiteSVM

- **64 account-lock limit (incl. program ids).** `mint` N ≤ 8 while the backstop is
  LIVE (N = 9 → 65 locks, `TooManyAccountLocks`, tested); N = 9 is fine before the
  backstop is placed. `seed` N ≤ 9. `redeem` N = 8 with a live backstop = 58 accounts.
  No program-side book cap: bigger books use `redeem_begin/components` and the keeper
  places the backstop after the first buys (§6.5 j).
- **Instruction trace limit 64.** `create_basket` ≤ 6 positions
  (`CREATE_BASKET_POSITIONS`), `add_positions` 8 (`POSITIONS_PER_TX`).
- Tests request 1.4M CU; everything below fits one transaction.
- create_basket (2–6 positions) 230–405k CU; add_positions(8) 250k.
- seed ≈ 950–980k (pool position + 2 bin arrays ≈ 200–255k each).
- mint 520–565k; with a live backstop and the 35-bin top-up ≈ 1.04M (N = 8); active
  bin outside tight (no top-up) ≈ 977k.
- redeem 330–390k; with a live backstop 595–650k (reads 6 bin arrays);
  redeem_begin 320k, redeem_components(5) ≈ 70k.
- place_backstop ≈ 540k (position + up to 2 new bin arrays); fund_backstop per array
  178k / 214k / 632k / 593k (share-side arrays cost more); withdraw_backstop per array
  356–487k; close_backstop ≈ 86k; recenter_tight 752–850k.
- claim_pool_fees tight (63 bins) ≈ 165–174k; backstop per array 150–205k.
- sweep_fees 355–360k; sweep_fees_components(8) 252k; settle_fees ≈ 104k.
- execute_buyback ≈ 86k; post_prize_payout(2) 28k; crystallize 40k; apply_book 45k.
- close_positions(2) 35k; close_basket (seeded) 136k; close_basket (unseeded) 21k.
- **Rent.** Pool 34,646,880 lamports (never closable); bin array 71,437,440 each
  (never closable); position 57,406,080 + growth (refundable). Deposit spent at
  launch with the index preset: pool + 2 tight arrays = **177.5M lamports**; with a
  4-array backstop sharing tight's two arrays = **320.4M**. Measured refunds at close:
  **0.905 SOL** (no backstop) / **0.758 SOL** (backstop), rents included.
- Pool swap fee on the index preset ≈ base 0.25 % + dynamic ≈ 0.4 % per side; Meteora
  keeps 20 % before our split.
- JIT guard: a position cannot remove from the active bin in the second it added
  there (`LiquidityLocked` 6055); the harness steps the clock 1 s after seed / mint /
  fund_backstop. A redeem landing in the same slot as a mint retries next slot.

## 5. Open design questions (historical — answered 2026-09-30)

Answered in the appendix of `docs/change-order-liquidity-and-fees.md`: (w) yes,
(x) yes, (y) yes (redeem is never gated, hard rule), (ac)/(ad)/(ae) moot (holder
rewards deleted), (af) yes, (ag) close runs a final creator sweep first and refunds
the deployer deposit + rents to `basket.payer`, `open_position` stays. (a), (d), (n),
(t) are re-opened by the fee-routing change and appear again in §6.4. The rest
stand as built. Original list kept for reference:

- (a) Mint/redeem fees are withheld **as shares into the FeeVault share ATA**, not
  skimmed per component. Simpler, one token; is that the intent of §4?
- (b) `fixed_kind` is chosen by the creator inside the signed payload.
- (c) `seed` takes `initial_shares` from the client (the $1000 open is a client
  computation) — should the program pin it?
- (d) Creator fee tier (20/30/40) needs a platform attestation; not implemented,
  splits currently use Config defaults.
- (e) Multi-tx redeem via a `Redemption` PDA with `pending_redeem_shares` kept in the
  denominator until every component is paid.
- (f) Share decimals = 6. (g) Metaplex metadata immutable. (h) Whitelist enforced on
  all Position creation. (i) §9 defaults live as Config params.
- (j) 64-lock limit ⇒ single-tx seed N ≤ 9 / mint N ≤ 10. Larger books need a staged
  two-tx mint (or ALTs once the lock-limit feature activates). Asset cap policy in
  TODO.md (Fixed = 20) collides with this.
- (k) Sleeve share side follows range geometry (≈2.2× nominal Y for Bounded).
- (l) Mint with pool price pinned at the range ceiling skips the sleeve; at the floor
  adds shares only; min-mint check skipped when the SOL leg is zero.
- (m) A freeze landing between tx build and execution reverts with `ComponentFrozen`
  (client retries with the claim PDA).
- (n) Meteora takes 20% of pool swap fees before our 40/30/30.
- (o) Creation-fee accounting: implied total + 0.1 SOL fee ≥ 1 SOL min.
- (p) Seed rent ≈ 0.032 SOL paid by the first buyer.
- (q) create/add_positions limited to 8 positions per tx.
- (r) Redeem fee also applies to `redeem_begin` (two-step path) — same bps.
- (s) `claim_frozen` requires the vault to be thawed; if a token is frozen forever the
  claim is stranded. Sunset path is post-launch per TODO.md.
- (t) Pool swap fees are held as **native SOL on the FeeVault PDA** (not a wSOL ATA);
  `claim_pool_fees` and `sweep_fees` are permissionless cranks.
- (u) Creator SOL line is held back (`creator_owed_lamports`) when the creator wallet
  would end below rent exemption, paid on a later sweep.
- (v) `crystallize` is **keeper co-signed** (NAV attestation); the program never
  prices (D4). HWM is set to the **post-fee** NAV; mgmt fee accrues at `crystallize`
  and `apply_book` only (not on every mint/redeem).
- (w) Swap venue: generic CPI hook with a **Config allow-list** (Jupiter v6 default,
  ≤ 4 programs) rather than a hard-wired Jupiter interface; keeper builds the inner
  instruction; program guards vault deltas.
- (x) Turnover and sell caps are **weight-based** (Σ|Δw|/2; per-position
  `(w_old−w_new)/w_old + 5% tolerance`), not value-based — the program has no prices.
- (y) `mint` blocked while a rebalance is active; `redeem` allowed (pays whatever the
  vaults hold, including half-traded positions).
- (z) New-mint Positions are created at `submit_book` / `open_position` (before any
  trade) so redeem always pays out every basket-held token; `close_position` requires
  an empty vault; leftover dust would block finalize until swept.
- (aa) Finalize checks structure (hash, weights, count) not value; there is no
  on-chain check that vault values actually match the new weights.
- (ab) A rebalance whose window expired without `finalize_rebalance` keeps `mint`
  blocked until the keeper finalizes or resubmits; only the keeper can unstick it.
- (ac) **Reward currency.** D15 says "the basket's flagship reward mint". Built:
  rewards are paid in **shares or SOL** (whatever the FeeVault holds), chosen per
  root. Paying in a component would need a swap in the crank (D17 forbids the
  keeper sweeping pool SOL into components). Confirm shares/SOL is the intent.
- (ad) Rewards funding is committed at `post_rewards_root` (debited from the
  reserve), and a stale root can be closed by the keeper after **30 days**, returning
  the unpaid remainder. Pick the TTL.
- (ae) SOL reward leaves for wallets that would end below rent exemption are
  refused (`BelowMinimum`); the keeper's batch should skip them and they come back
  to the reserve at close. Alternative: pay in shares only.
- (af) Creator lock bounds **1 day – 4 years**; top-ups extend to the later date.
- (ag) `close_basket` semantics: "supply == 0" is unreachable literally (the
  FeeVault's fee shares and the pool's treasury shares are supply), so the rule
  built is "no holder shares outside the FeeVault (±10 base units) and idle 14 d".
  On close the sleeve SOL, vault dust and any unswept fee SOL go to the **treasury**;
  fee shares are burned; rents (Basket, Positions, vaults, FeeVault, cp-amm
  position) go to `basket.payer`. Unswept creator/holder fee lines are forfeited to
  the treasury — acceptable, or should `close_basket` run a final `sweep_fees`?
- (ah) `close_basket` requires a fully swept-out `PendingBook` and no in-flight
  `Redemption`; a holder who ran `redeem_begin` and never finished blocks the close
  (their shares are burned already). Do we want a keeper path to force-complete a
  stale Redemption?
- (ai) Baskets created but never seeded are closable by the keeper after 14 d idle
  (rent back to the creating payer). Shorter window for those?

---

## 6. Change order (2026-09-30): DLMM facts, migration plan, open questions

Source: `docs/change-order-liquidity-and-fees.md`. Nothing in the program has been
changed for it yet; this section is the checklist. Read §6.1 first — two of the
change order's assumptions about DLMM do not hold and change the deployer economics.

### 6.1 DLMM facts verified against the program IDL (v0.12.0) and Meteora docs

Program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (same id on devnet); closed
source, so we CPI through `declare_program!(lb_clmm)` from `idls/lb_clmm.json` and
test against the mainnet dump `tests/fixtures/lb_clmm.so`, exactly as for cp-amm.

- **Bin geometry.** Price of bin `i` = `(1 + bin_step/10000)^i`; `bin_step ≤ 400`
  bps; 70 bins per `BinArray`; one `PositionV2` spans up to **1400 bins** (70 stored
  inline, the rest as extension data; grow ≤ 91 bins per resize instruction).
- **Rent (SDK constants).** LbPair 0.0072 + two reserves 0.0041 + oracle ≈ 0.023 →
  **pool ≈ 0.035 SOL, not closable**. Position 0.0574 SOL + **0.00078 SOL per bin
  beyond 70** (both refundable on `close_position2`). **BinArray 0.0714 SOL each,
  effectively NON-refundable**: `close_bin_array` needs a second `signer` account
  that only Meteora's admin satisfies (the SDK never calls it; Meteora's own docs call
  bin-array rent non-refundable). This contradicts change order §1 ("rent is
  refundable when bin arrays are closed") and §2.4 ("close emptied bin arrays").
- **Consequence for the keeper float.** Bin arrays are per *price range*, not per
  position. Because `tight` always lives inside `backstop`, re-centering `tight`
  never needs a new bin array while price stays inside the backstop. New arrays are
  only paid when the backstop itself is (re)placed. So the "float" is ≈ 0 per
  re-center; the one-off cost is the launch set of arrays.
- **Positions for a PDA owner.** `initialize_position_pda(lower_bin_id, width)`:
  seeds `["position", lb_pair, base, lower_bin_id, width]`, `base` and `owner` both
  sign — the basket PDA can be both via CPI signer seeds, so no client keypair and
  no position NFT. `close_position2(rent_receiver)` refunds to any account we name.
- **Pool creation.** `initialize_customizable_permissionless_lb_pair2(CustomizableParams
  { active_id, bin_step, base_factor, base_fee_power_factor, activation_type,
  activation_point: None, has_alpha_vault: false, creator_pool_on_off_control: false,
  concrete_function_type, collect_fee_mode })`; PDA `[ILM_BASE, sorted mints]`,
  `funder` signs (basket PDA, paid from the deposit). No preset account needed. Base
  fee = `base_factor × bin_step × 10 × 10^power / 1e9` (min 0.01 %, max 10 %).
  Dynamic fee parameters are derived by the program from the bin step. Customizable
  pools give Meteora **20 % of swap fees** (`ILM_PROTOCOL_SHARE = 2000`) before our
  split — same as cp-amm did.
- **Liquidity CPIs (v2 flavours, all take bin arrays as remaining accounts):**
  `add_liquidity_by_strategy2 { amount_x, amount_y, active_id, max_active_bin_slippage,
  strategy: { min_bin_id, max_bin_id, SpotBalanced } }` — X (shares) is spread over
  bins ≥ active, Y (SOL) over bins ≤ active, no ratio leftovers (unlike cp-amm);
  `remove_liquidity_by_range2(from_bin, to_bin, bps)`; `claim_fee2(min_bin, max_bin)`;
  `rebalance_liquidity` (remove + resize + add + shrink in one CPI, `rent_payer`
  signs for extension rent); `initialize_bin_array(index)` (CU-heavy, sets 70
  prices). Bin array index = `floor(bin_id / 70)`.
- **Active-bin price for treasury shares:** `LbPair.active_id` is a plain `i32` in the
  zero-copy account — read on-chain, no oracle.

### 6.1b Measured in LiteSVM against the mainnet binary (`tests/dlmm_spike.rs`, green)

Bin step 100, fee 1 %, tight `[−12, +12]` (12 SOL + 12 000 shares), backstop
`[−140, +162]` (3 SOL + 3 000 shares), positions owned by a wallet (the PDA path is
identical via CPI signer seeds).

| Call | CU | Notes |
| --- | --- | --- |
| `initialize_customizable_permissionless_lb_pair2` | 45–58k | rent **0.0346 SOL** (LbPair + 2 reserves + oracle), never closable |
| `initialize_bin_array` | **200–255k each** | rent **0.0714 SOL each**, never closable; 5 arrays for the backstop |
| `initialize_position_pda` (70 bins) | 12–15k | rent 0.0574 SOL |
| `increase_position_length2` (+91 bins) | 9k | needed 3× for 303 bins (10 KB realloc cap per ix); 303-bin rent **0.239 SOL** |
| `add_liquidity_by_strategy2`, 25-bin tight | 129k | one call |
| same, 70-bin chunk | 136k (SOL side) / **530–540k (share side)** | **> ~70 bins per call → program OOM**; must chunk per bin array |
| `swap2` 1 SOL inside tight | 40k | 984 shares out (1 % fee + 1 tick) |
| `swap2` 14 SOL through 80 bins | 470k | left tight, filled from backstop, pool kept quoting |
| `swap2` 25 SOL > all asks | fails `6036 BitmapExtensionAccountIsNotProvided` | this is what "stops quoting" looks like |
| `remove_liquidity_by_range2` 25-bin tight (all) | 121k | |
| same, 303-bin backstop (all) | **1.20M** | ≈ 260–330k per 70-bin chunk |
| `claim_fee2` 25 / 303 bins | 32k / 154k | |
| `close_position2` | 8k | rent refunded to `rent_receiver` |

Behavioural findings:
- **JIT guard.** A position cannot remove liquidity from the *active* bin in the
  same slot it added liquidity there (`6055 LiquidityLocked`). A stranger's swap
  does not trigger it (tested), so there is no griefing vector; but a `redeem`
  landing in the same slot as a `mint` (same vault position) must retry next slot,
  and `recenter` must remove before it adds (it does).
- Imbalanced Spot deposits leave a little X undeposited (≈ 5 % here); the program
  must burn or keep the leftover treasury shares, as `burn_basket_share_dust` does.
- The backstop is thin by construction: with 20 % of the sleeve over 302 bins, the
  1.3 SOL that spilled past `tight` moved price from 1.13× to 2.2×. It guarantees a
  quote, not depth; depth in the lag window is the keeper's speed.

Consequences for the plan (folded into §6.3 / §6.4):
- **Redeem cannot withdraw pro-rata from the backstop atomically** (1.2M CU alone).
- **Launch is several transactions**: create + pool (1), positions + 5 bin arrays
  (1–2), backstop deposit (3, share side is 540k per chunk), tight seed (1).
- Per-basket launch rent at bin step 100 / `[¼, 5×]`: 0.0346 + 5 × 0.0714 = **0.39 SOL
  non-refundable**, 0.0574 + 0.239 = **0.30 SOL refundable** (positions).

### 6.2 Cost model per basket (drives the deposit decision — §6.4 Q2/Q3)

Backstop `[NAV/4, 5×NAV]` = `ln 4 / ln(1+s)` bins below + `ln 5 / ln(1+s)` above.

| bin step | tight ±12 % | backstop bins | bin arrays | launch rent (SOL) | of which non-refundable |
| --- | --- | --- | --- | --- | --- |
| 100 (1 %) | 25 bins | 140 + 162 = 302 | 5–6 → 0.36–0.43 | ≈ 0.73 | ≈ 0.44 (pool + arrays) |
| 200 (2 %) | 13 bins | 70 + 81 = 151 | 3–4 → 0.21–0.29 | ≈ 0.46 | ≈ 0.29 |
| 100, index `[½, 3×]` | 25 bins | 70 + 110 = 180 | 3–4 | ≈ 0.45 | ≈ 0.29 |

Position rent (refundable) for 302 bins ≈ 0.24 SOL; the 1 SOL deposit covers any row.
Bin step 100 gives a tick of 1 % (a 1 SOL buy on a 12 SOL SOL-side tight moves ≈ 8
bins ≈ 8 %); bin step 200 halves the array cost but doubles the tick.

### 6.3 Migration plan (order of work; each step = tests first, then handler, commit)

**Status: all 11 steps done** (commits `2c0152b` → `858434b`). The list below is the
plan as written; where the build diverged, §3b is what exists and §6.5 says why.

1. **Spike — done** (`src/dlmm.rs`, `tests/common/dlmm.rs`, `tests/dlmm_spike.rs`,
   §6.1b). Re-centering will be remove (121k) → `close_position2` (8k) →
   `initialize_position_pda` (14k) → add (130k) ≈ 275k CU in one instruction; no
   need for `rebalance_liquidity`, whose per-bin add params we cannot compute
   on-chain without the withdrawn amounts.
2. **Delete holder rewards**: `RewardsRoot`, `post_rewards_root`, `distribute_rewards`,
   `close_rewards_root`, `tests/rewards.rs`, Merkle harness, `RewardsRoot*` events,
   `REWARDS_ROOT_TTL_S`, `MAX_REWARD_LEAVES`, `holder_reserve_*` on `FeeVault`.
3. **`create_basket` rewrite**: deployer signs; drop `ed25519.rs`, instructions sysvar,
   nonce/replay fields, `creator_sig` payload; `basket.creator = basket.payer =
   deployer`; take `config.deposit_lamports` (1 SOL) onto the Basket account
   (`deposit_lamports`, `deposit_spent_lamports` fields); DLMM pool init CPI with
   `funder = basket`; `Basket` gains `lb_pair`, `position_tight`, `position_backstop`,
   `bin_preset: u8`, loses `range_kind`/`pool_position`/reward fields; `Config` gains
   bin presets (`bin_step`, `base_fee_bps`, `tight_half_width_bins`,
   `backstop_down_bins`, `backstop_up_bins`, `recenter_trigger_bins`) per basket
   type, `deposit_lamports`, mint-window bounds, fee split, buyback/team/prize
   destinations, prize params, `allow_open_gate`.
4. **`seed`**: init `tight` (and, per Q15, `backstop` + its bin arrays either here
   from the deposit or by the keeper right after), mint treasury shares at the
   active-bin price, deposit sleeve into `tight`, 1 SOL minimum first buy unchanged.
   **`mint`**: `add_liquidity_by_strategy2` into `tight` only (129k CU).
   **`redeem`**: per Q14 — pro-rata of *all* pool SOL, paid out of `tight` (+ idle
   sleeve), `remove_liquidity_by_range2(bps)` on `tight` only; returned treasury
   shares burned; `holder_shares()` reads both positions (Σ bin amount × position
   share of bin `liquidity_supply`, from the bin arrays passed in).
5. **Fees**: split 20/50/25/5 in `Config`; `sweep_fees` → creator (FeeVault line),
   `BUYBACK` PDA, `team_wallet`, `PRIZE` PDA; `claim_pool_fees` claims both positions;
   mint/redeem fee currency per Q1. FeeVault shrinks to the creator line.
6. **Mint gate** default `WindowUntil(now + config.default_window_s)` then `Closed`;
   `Open` only if `config.allow_open_gate`; window bounded by Config.
7. **`recenter_tight`** (keeper): reads `active_id`, refuses unless the active bin is
   within `recenter_trigger_bins` of an edge or outside; withdraws all of `tight`,
   re-places `[active − w, active + w]` from the basket's own ATAs (program computes
   amounts, keeper passes no amounts), optional treasury-share top-up per Q7; emits
   `TightRecentered { old_range, new_range, x, y }`. **`recenter_backstop`** per Q5.
8. **`close_basket`**: creator sweep first → `remove_liquidity_by_range2` (all) ×2 →
   `claim_fee2` ×2 → `close_position2` ×2 (rent → payer) → burn shares → FeeVault →
   refund `deposit_lamports − deposit_spent_lamports` (+ recovered rents) to payer.
   Bin arrays and the LbPair stay (cannot be closed).
9. **Prize pool**: `PRIZE` PDA accrual; `pay_prizes(epoch, winners[])` keeper-signed
   with `CreatorLock` liveness check per winner, `Config.prize_{epoch_s, top_n,
   split_bps}`, min-epoch guardrail per Q9. **Buyback**: accrual PDA now, swap+burn
   instruction per Q8.
10. **Tests**: swap fixtures, rewrite seed/mint/redeem/fees/close tests around two
    positions, add `recenter_tight` tests incl. out-of-range and keeper-late
    (backstop still quotes: a swap through the pool succeeds after price leaves
    `tight`), CU/account budgets re-measured (§4).
11. **Docs**: strike `program-build-confirmation.md` §3 range table, refresh
    frontend brief §6 / launch wizard copy, README status.

### 6.4a Decisions (Nick, 2026-09-30, second round) — these are the spec now

- **Fee currency (Q1).** Mint/redeem fees stay withheld as **shares** in the FeeVault.
  `sweep_fees` pays the creator's 20 % in shares and **redeems the other 80 % in
  kind** into FeeVault-owned component ATAs; a permissionless `settle_fees` swaps
  those components to SOL through the allow-listed venue and routes the SOL
  buyback / team / prizes (50 / 25 / 5 of the 80 %). Pools are created with
  **`collect_fee_mode = OnlyY`** so every pool swap fee arrives as SOL and routes
  straight to the four lines. Nothing in the fee path prices anything.
- **Deposit (Q2/Q16).** Deployer eats the non-refundable rent out of the 1 SOL:
  `deposit_lamports` / `deposit_spent_lamports`, refund at close = deposit − spent
  (+ position rents). Launch screen shows the exact number. No graduation refund
  (Q11): refund only at `close_basket`.
- **Presets (Q3/Q4/Q5, Config defaults per type, creator cannot change, tune on
  devnet):**

  | type | bin step | base fee | tight | backstop | reset confirm |
  | --- | --- | --- | --- | --- | --- |
  | index / majors | 25 | 0.25 % | ±8 % (±31 bins) | ±40 %, ≤ 4 arrays | 3 checks |
  | Mirror / Strategy | 50 | 0.5 % | ±12 % (±23 bins) | ±50 %, ≤ 4 arrays | 3 checks |
  | meme / sector / Fixed | 100 | 1 % | ±15 % (±14 bins) | ±70 %, ≤ 4 arrays | 1 check |

  Backstop is **one position, symmetric around the launch active bin**, placed as
  whole bin arrays (cap 4 → ≤ 0.29 SOL non-refundable; narrow the width, never
  raise the cap), holding ~20 % of the sleeve. The keeper's re-center loop never
  moves it; withdrawn only on close; re-placed only by `reset_backstop` (keeper,
  logged) when the active bin has been **outside its bin range** for N consecutive
  keeper checks (N per type above; the count is keeper-side, the program checks
  "outside now"), re-placed symmetrically at the type's width.
- **Tight re-center (Q4).** `recenter_tight` when the active bin has moved past 70 %
  of the half-width from centre, or is out of range; **5-minute minimum interval**
  between re-centres of the same basket. Built as: the interval applies to the 70 %
  trigger; an out-of-range active bin may always be re-centred (speed is priority 5).
- **Backstop funding (Q6/Q15).** Deployer's launch tx places `tight` from the first
  buy; the **keeper places the backstop within the first minute** from the sleeve
  slice of that buy, rent from the deposit. **Every later mint** adds 20 % of its
  sleeve to the existing backstop position; if price has left the backstop the add
  goes to `tight` instead (no new bin arrays on the mint path).
- **Treasury-share top-up at re-center (Q7).** Allowed **while the mint gate is
  open** (mint treasury shares at the active-bin price to rebuild asks). Gate closed
  → no: asks come only from holders selling; that is the premium.
- **Redeem (Q14).** Withdraws pro-rata from **`tight` only**, one transaction. The
  backstop is a protocol-level guarantee, counted in NAV, settled at close.
- **Buyback (Q8).** BUYBACK PDA accrues SOL. `Config.bskt_mint: Option<Pubkey>`
  starts `None`; `execute_buyback` refuses while unset; admin sets it **once** under
  the treasury timelock, then immutable.
- **Prizes (Q9).** `post_prize_payout(epoch, recipients[])` keeper-signed; on-chain
  checks: epoch elapsed, total ≤ PRIZE balance, count ≤ Config max, each recipient
  has a live `CreatorLock`, each recipient's basket claimed ≥
  `Config.min_epoch_pool_fees` SOL of pool fees during the epoch (needs per-basket
  epoch counters on `claim_pool_fees`). Ranking / follower PnL off-chain.
- **Mint window (Q12).** 48 h default, 24–72 h bounds. **Team wallet (Q13).**
  `Config.team_wallet`, timelocked admin change, no per-basket override; devnet
  placeholder key, Squads vault before mainnet.
- **Creator tier (Q10).** Keeper-signed **ed25519 attestation over
  `(creator, tier, expiry_slot)`** passed as an argument to `create_basket`, keeper
  key from Config; missing/expired → base tier. Keep `ed25519.rs` for this; delete
  only the creator-signature / replay path. Tier → creator bps table lives in Config
  (defaults all 2000 until Nick sets the ladder — **still open: values and which
  line funds the bump**).

### 6.5 Decisions taken during the build that the change order / §6.4a do not cover

Rule of the build is "ask, don't decide"; these came up mid-step where stopping would
have left the program half-migrated, so each was taken the conservative way, tested,
and is listed here for Nick to confirm or overturn. Program-side each is a small change.

a. **`settle_fees` is keeper-only.** Q1 said "permissionless". It re-issues an
   arbitrary inner instruction with the FeeVault PDA as signer against an allow-listed
   venue; permissionless would let anyone pick the route and the slippage floor for
   our fee SOL. Guards (`UnexpectedAccountInSwap`, `min_amount_out`, venue allow-list)
   are the same as `execute_swap`. Flip = one `require!`.
b. **Pool fees claimed inside `close_basket` go to the treasury un-routed** (they are
   dust by then: the keeper claims right before close).
c. **Any remaining holder blocks `close_basket`** (`H ≤ 10` base units), including
   the FeeVault's own fee shares — so the close path is sweep → creator redeems → close.
d. **D13 launch fee scheduler dropped** (DLMM customizable pools have no scheduler).
   Creation fee 0.1 SOL kept, to the treasury at seed.
e. **Managed baskets use the Mirror/Strategy pool preset** (not named in the table).
f. **Per-mint backstop top-up is 35 bins around the active bin, inside tight's bin
   arrays, only while the backstop is LIVE and the active bin is inside tight.** The
   change order's "20 % of every mint into the existing backstop position" over the
   full 4-array range costs 1.25–1.4M CU and 2 extra account locks → does not fit a
   mint transaction. The 20 % slice still reaches the backstop (same position, same
   bins it already covers), just concentrated. When the active bin is outside tight
   the slice stays idle and `fund_backstop` / `recenter_tight` sweep it in.
g. **Mint single-tx cap N ≤ 8 once the backstop is live** (N = 9 needs 65 locks).
   N = 9 works before the backstop is placed; the frontend brief should say "≤ 8
   components for one-click mint, 9+ use the two-step path" or we cap books at 8.
h. **Backstop funding goes in ascending bin-array order, one array per transaction,**
   growing the position each time (DLMM `InvalidRealloc` above ~10 KB per tx). Keeper
   pays the growth rent (0.0008 SOL/bin, refunded at close).
i. **`recenter_tight` leaves idle SOL/shares alone unless the backstop is LIVE** —
   while UNPLACED / FUNDING / CLOSED the idle balance is the backstop's slice and
   must not be swept into tight.
j. **No program-side cap on book size**; the budgets in §4 are enforced by the client
   (two-step redeem, backstop placed after first buys for 9+ books).
k. **Recenter interval counts from `seed`** (`last_recenter_ts` = seed time): the first
   70 %-triggered re-centre is ≥ 5 min after launch; out-of-range always allowed.
l. **Creator tier ladder defaults to 2000 bps for every tier** until Nick sets values
   (Q10 still open: the ladder and which line funds the bump — today the other three
   lines shrink pro rata).
m. **Redeem clamps `h = max(h, net)`.** Bin-share → amount conversion undercounts a
   position by up to ~31 base units, so `H` can land below the last holder's balance
   and the final redeem would fail by a few units. With the clamp the last holder
   gets everything that is left. Same clamp in `sweep_fees` / `sweep_fees_components`.
n. **Close crank checks the backstop state before loading the pool view**, so a live
   backstop fails with `BackstopState` instead of `PositionMismatch`.
o. **Prize epoch 0 starts at `initialize_config`** (`epoch_anchor_ts = now`, as the
   struct doc already said; the handler had left it at 0).
p. **`close_fee_vault` does not check the FeeVault's component ATAs** are empty —
   they are regular ATAs the keeper settles with `settle_fees`; closing early strands
   nothing on-chain but leaves tokens in ATAs nobody can sign for anymore. Keeper
   ops: settle before close (could be enforced by passing the ATAs).
q. **Transient failures to expect on mainnet:** a `redeem` in the same slot as a
   `mint`, or `withdraw_backstop` in the same slot as `fund_backstop`, fails with
   DLMM `LiquidityLocked` (JIT guard) — retry next slot. Keeper eats bin-array rent
   when the deposit is exhausted (`spend_deposit` pays what it can — untested path).
r. **`payer_wsol_ata` must exist before `create_basket`** (the launch tx is at the
   trace limit); the frontend creates it in the same transaction, before our ix.
s. **Launch economics for the frontend brief §6:** launch tx ≈ deposit 1 SOL + ~0.034
   SOL rents/fees + 0.1 SOL creation fee at seed; `≈ 0.76–0.9 SOL` comes back at
   close (deposit minus pool + bin-array rent, plus account rents), not "≈ 0.6".

### 6.4 Open questions (asked 2026-09-30; answered in 6.4a — kept for the reasoning)

Q1. **Fee currency.** Today mint/redeem fees are withheld as *shares* into the
    FeeVault. Buyback, team and prizes need *SOL*. Options: (a) charge mint and
    redeem fees in **SOL** (mint: `fee_bps × implied total`, the buyer already wraps
    SOL for the sleeve; redeem: `fee_bps × shares × active-bin price`, paid by the
    redeemer) and burn the share side of claimed pool fees; (b) keep share fees and
    have the sweep sell 80 % of them into our own pool (sell pressure on our token,
    SOL comes out of our own sleeve); (c) redeem the fee shares in-kind and let the
    keeper Jupiter-swap components to SOL (D17 tension). My pick: (a).
Q2. **Non-refundable part of the deposit** (§6.1b): pool 0.035 + 5 bin arrays
    0.357 = **0.39 SOL** at bin step 100 (0.25 SOL at bin step 200) cannot be
    recovered. Who eats it: the deployer (≈ 0.6 of the 1 SOL comes back; conflicts
    with priority 4 "non-refundable beyond a few dollars are not") or the team
    (refund the full 1 SOL, treasury pays ≈ 0.4 SOL per basket; conflicts with
    priority 3)? Or a larger deposit (1.5 SOL) with the non-refundable part
    disclosed on the wizard's final screen?
Q3. **Bin presets.** Proposal: memes/Fixed/Strategy `bin_step 100`, base fee 1 %;
    index/majors `bin_step 100`, base fee 0.5 %, narrower backstop. Or bin step 200
    for memes to halve array cost at a 2 % tick? Confirm the numbers.
Q4. **Tight and trigger widths.** Proposal: tight = ±12 bins (±12 % at step 100),
    re-center allowed when the active bin is ≤ 2 bins from an edge or outside.
Q5. **Backstop geometry and "never touched".** Proposal: `[¼×, 5×]` of the price at
    placement (upside wider because mint-closed baskets have no arb capping a
    premium; downside is capped by redeem arbitrage). Once price has moved more than
    half the log-distance to an edge (> 2.24× up or < 0.5× down from centre) the
    keeper may **re-place the backstop too** (costs new bin arrays, rare). Without
    this a basket that legitimately 3×'s has 1.7× of cover left. Accept the
    exception to "never touched"?
Q6. **Backstop funding.** "Fixed slice (e.g. 20 %) of sleeve → backstop": at seed
    only, or on every mint? Proposal: seed splits 80/20; later mints go to `tight`
    only (cheap, one bin array); a keeper `top_up_backstop` moves capital when the
    backstop falls below 20 % of total sleeve SOL.
Q7. **Treasury-share top-up at re-center.** After a pump `tight` is all SOL; after a
    dump all shares. Re-centering a one-sided position leaves no asks (or no bids).
    Proposal: `recenter_tight` mints treasury shares at the active-bin price so the
    new ask side matches the bid side in value (same sleeve model as `mint`; NAV
    excludes shares in positions), and never mints SOL (a dumped basket stays
    asks-only until buyers restore bids). Confirm.
Q8. **Buyback PDA.** Accrue SOL now; `buyback_swap_burn` needs the `$BSKT` mint and a
    venue — ship the accrual PDA in this pass and the swap/burn instruction when the
    mint exists? Who may trigger it (keeper vs permissionless)?
Q9. **Prize guardrails on-chain vs keeper.** Volume is not visible on-chain; proposal:
    `Config.prize_min_epoch_accrual_lamports` (fees accrued into the PRIZE PDA that
    epoch) as the on-chain proxy, ranking/ties done by the keeper, `CreatorLock`
    liveness checked on-chain per winner. Default epoch 14 d, top 3 → 50/30/20.
Q10. **Creator tier ("tiered by leaderboard rank at create").** The attestation used
    to ride the deleted ed25519 payload. Options: keeper-signed `set_creator_tier`
    within N days of create; or a keeper-written `CreatorProfile` PDA read at
    create. Tier table (20 % base → ?) also needed.
Q11. **Graduation refund.** "Returned when the basket graduates past a TVL threshold":
    TVL is not priceable on-chain; proposal: refund when SOL in both positions ≥
    `Config.graduation_sol` (e.g. 50 SOL) via a permissionless crank — or drop
    graduation and refund only at close (simpler, my pick until we have data).
Q12. **Mint window default** inside the 24–72 h bound: 48 h? And `MIN_SKIN_BPS`.
Q13. **Team wallet**: a fixed `Config.team_wallet` (Squads vault) updated only by the
    timelocked admin? Any per-basket override? (Assumed: no override.)
Q14. **Redeem vs backstop (from §6.1b).** Withdrawing pro-rata from the 303-bin
    backstop costs ≈ 1.2M CU, so `redeem` cannot touch it atomically. Proposal:
    the backstop is a *protocol-managed reserve* whose SOL and shares count in
    NAV, and `redeem` pays the holder's pro-rata share of **all** pool SOL out of
    `tight` (+ idle sleeve); a keeper crank `refill_tight` moves capital from
    backstop to tight (chunked, keeper pays CU) whenever tight's SOL falls under a
    Config floor. If a single redemption exceeds what tight holds, it goes through
    the existing two-tx `Redemption` path and completes after a refill (redeem is
    still never *gated*, only staged). Alternative: shrink the backstop so it fits
    one chunk (≤ 70 bins ⇒ `[¼, 5×]` needs bin step ≥ 400, a 4 % tick — bad for
    priority 2). Confirm the reserve model.
Q15. **Who seeds the backstop.** Its deposit is 5 chunks (share side 540k CU each),
    i.e. ≥ 3 extra deployer transactions at launch. Proposal: the deployer's launch
    is create (+ pool) and seed (tight) only; the **keeper places the backstop
    within seconds** from the sleeve slice, paying bin-array rent from the deposit
    (`deposit_spent_lamports`). Between launch and placement only `tight` quotes.
    Or keep it in the deployer's launch bundle (5–6 signed txs)?
Q16. **Deposit consumption vs refund.** With bin arrays non-refundable the deposit
    should be tracked as `deposit_lamports` (paid in) and `deposit_spent_lamports`
    (rent that cannot come back); refund at close = `deposit − spent` + position
    rents. Wizard copy would show "≈ 0.6 SOL refundable of 1 SOL". OK, or is the
    team eating `spent` (Q2)?
