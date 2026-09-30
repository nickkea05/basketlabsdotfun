# basketlabs.fun — program build progress (handoff)

Living log for the `basket` Anchor program build. Spec is
`docs/program-build-confirmation.md` (D1–D20, §1–§9). Rule of the build: tests
written before handlers, LiteSVM, one test file per instruction, and any design
question the spec leaves open gets **asked, not decided** (collected in §5 below).

Last updated: 2026-09-28 (Phase 7 done and pushed, `1922db4`; all §5 instructions built).

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
| 8 | Devnet: deploy, CU measurements on a real validator, batch UX end to end, indexer event coverage | not started |
| wrap | README/TODO refresh, present §5 to Nick | done except the conversation |

Last green run: **82/82** — unit 6, add_positions 4, apply_book 3, claim_pool_fees 3,
close_basket 3, create_basket 6, creator_lock 2, crystallize 6, execute_swap 4,
finalize_rebalance 4, initialize_config 6, mint 6, redeem 6, rewards 3, seed 5,
smoke 2, submit_book 5, sweep_fees 5, update_whitelist 3.

## 2. Restart checklist

From `c:\dev\ponsclone\basketfun`:

1. Move stale test exes out of the way (McAfee holds them → `LNK1104`):
   `Get-ChildItem target\debug\deps\*.exe | Move-Item -Destination $env:TEMP\stale -Force`
2. `anchor build` — if the IDL step fails with a link error, repeat step 1 and rerun.
3. `cargo.exe test -p basket --no-fail-fast -- --test-threads=1` (always `cargo.exe`:
   a stray 0-byte `C:\Windows\system32\cargo` triggers an "open with" popup).
4. Next up: get Nick's answers on §5, fold them in (most are one-line changes),
   then Phase 8 (devnet). Devnet needs a program keypair outside the repo,
   `anchor keys sync`, a real Meteora cp-amm + Jupiter on devnet (Jupiter has no
   devnet deployment — rebalance tests there will use the cp-amm venue path as the
   LiteSVM tests do), and the TS client in `packages/solana` generated from the IDL.

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

## 5. Open design questions for Nick (not decided — flagged)

Places where the spec was silent and I picked a placeholder so work could continue.
Each is easy to change; (a), (e), (t)–(v) touch account layouts.

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
