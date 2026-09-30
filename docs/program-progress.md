# basketlabs.fun — program build progress (handoff)

Living log for the `basket` Anchor program build. Spec is
`docs/program-build-confirmation.md` (D1–D20, §1–§9) **as amended by
`docs/change-order-liquidity-and-fees.md`** (2026-09-30; wins on conflict). Rule of
the build: tests written before handlers, LiteSVM, one test file per instruction, and
any design question the spec leaves open gets **asked, not decided** (§6 below).

Last updated: 2026-09-30 (change order received; DLMM facts verified, migration plan
and open questions in §6; **no program code changed yet**).

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
| 7b | **Change order** (DAMM v2 → DLMM two positions, deployer-signed create + deposit, fee split 20/50/25/5, rewards removed, Window→Closed default, prize pool) | plan in §6; blocked on §6.4 answers |
| 8 | Devnet: deploy, CU measurements on a real validator, batch UX end to end, indexer event coverage | not started (after 7b) |
| wrap | README/TODO refresh, present §5 to Nick | done; §5 answered 2026-09-30 |

Last green run: **85/85** — unit 8, add_positions 4, apply_book 3, claim_pool_fees 3,
close_basket 3, create_basket 6, creator_lock 2, crystallize 6, dlmm_spike 1,
execute_swap 4, finalize_rebalance 4, initialize_config 6, mint 6, redeem 6,
rewards 3, seed 5, smoke 2, submit_book 5, sweep_fees 5, update_whitelist 3.

## 2. Restart checklist

From `c:\dev\ponsclone\basketfun`:

1. Move stale test exes out of the way (McAfee holds them → `LNK1104`):
   `Get-ChildItem target\debug\deps\*.exe | Move-Item -Destination $env:TEMP\stale -Force`
2. `anchor build` — if the IDL step fails with a link error, repeat step 1 and rerun.
3. `cargo.exe test -p basket --no-fail-fast -- --test-threads=1` (always `cargo.exe`:
   a stray 0-byte `C:\Windows\system32\cargo` triggers an "open with" popup).
4. Fixtures: `.\scripts\fetch-fixtures.ps1` now also dumps `lb_clmm.so` (DLMM).
   The DLMM IDL is `idls/lb_clmm.json` (dlmm-sdk `idls/dlmm.json`, program v0.12.0;
   the on-chain IDL account does not exist, `anchor idl fetch` fails).
5. Next up: get Nick's answers on **§6.4**, then execute §6.3 in order, then Phase 8
   (devnet). Devnet needs a program keypair outside the repo, `anchor keys sync`,
   Meteora DLMM on devnet (same program id as mainnet) + Jupiter (no devnet
   deployment — rebalance tests there use the DLMM venue path as LiteSVM does), and
   the TS client in `packages/solana` generated from the IDL.

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

## 4. Measurements and hard limits (keep in README)

- Mainnet **64 account-lock limit**: single-tx `seed` N ≤ 9 (26 fixed + 4N), `mint`
  N ≤ 10 (23 fixed + 4N). N=10 seed → `TooManyAccountLocks` (tested).
- **Instruction trace limit 64** → ≤ 8 positions per `create_basket` /
  `add_positions` tx (`POSITIONS_PER_TX = 8`); harness chunks automatically.
- Default 200k CU is not enough anywhere with CPI; tests request 1.4M.
- create_basket(6) 281k CU; add_positions(7) 240k (~34k/position).
- seed N=2/5/8/9: 34/46/58/62 accounts, 269k/331k/323k/362k CU; rent+fees ≈ 0.032 SOL
  paid by the first buyer.
- mint N=2/5/8/9: 30/42/54/58 accounts, 121k/134k/149k/164k CU.
- redeem N=2/5/8/9: 30/42/54/58 accounts, 118k/151k/160k/169k CU; redeem_begin 124k;
  redeem_components(5) 52k.
- claim_pool_fees ≈ 80–85k; sweep_fees small; crystallize 47k.
- submit_book (1 new position) 51k; apply_book 44k; execute_swap (cp-amm inner) 38k;
  finalize_rebalance(3) 19k.
- post_rewards_root 20k; distribute_rewards (shares, ATA created) 37k;
  lock_creator_shares 43k; unlock 20k; close_positions(2) 73k; close_basket (seeded,
  3 cp-amm CPIs) 144k; close_basket (unseeded) 23k.
- Meteora cp-amm: creator = basket PDA (owns position NFT), payer = buyer; base fee
  layout is PodAlignedFeeTimeScheduler; scheduler ≤ 1 day; protocol takes 20% of swap
  fees before our split.
- Sleeve geometry: SOL leg `B = r·D` is primary, pool price = NAV; share side follows
  the range (≈2.207× nominal for Bounded [0.5×, 8×], 3.414× FloorOnly, 1× Full).

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

### 6.4 Open questions for Nick (not covered by the change order — asked, not decided)

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
