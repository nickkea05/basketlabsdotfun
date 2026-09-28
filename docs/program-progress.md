# basketlabs.fun — program build progress (handoff)

Living log for the `basket` Anchor program build. Spec is
`docs/program-build-confirmation.md` (D1–D20, §1–§9). Rule of the build: tests
written before handlers, LiteSVM, one test file per instruction, and any design
question the spec leaves open gets **asked, not decided** (collected in §5 below).

Last updated: 2026-09-28 (end of session, mid Phase 3).

---

## 1. Where things stand

| Phase | Scope | State |
| --- | --- | --- |
| 0 | Workspace, spike (`declare_program!(cp_amm)`, anchor-spl metadata, U256 math under SBF) | done |
| 1 | `initialize_config`, `update_whitelist`, `create_basket` (ed25519 introspection, chain-hash book, replay binding), `add_positions` | done, tests green |
| 2 | `seed` + `mint` with DAMM v2 CPI, sleeve step rule, treasury-share accounting | done, tests green |
| 3 | `redeem`, `redeem_begin` / `redeem_components`, `claim_frozen` (FrozenClaim) | **written, not yet compiled or run** |
| 4 | FeeVault, `claim_pool_fees`, `sweep_fees`, 40/30/30 split | not started |
| 5 | `crystallize` + HWM, management fee (gates/schedules already enforced in mint) | not started |
| 6 | `submit_book` / `apply_book` / `execute_swap` (Jupiter CPI) / `finalize_rebalance` | not started |
| 7 | `post_rewards_root` / `distribute_rewards`, creator lock/unlock, `close_basket`, admin | not started |
| wrap | `scripts/fetch-fixtures.ps1`, gitignore `*.so` fixtures, README/TODO, present §5 to Nick | not started |

Last known green run (before Phase 3 edits): **38/38** — unit 6, add_positions 4,
create_basket 6, initialize_config 6, mint 6, seed 5, smoke 2, update_whitelist 3.

## 2. Restart checklist (do this first tomorrow)

From `c:\dev\ponsclone\basketfun`:

1. `anchor build` — expect compile errors in the brand-new
   `programs/basket/src/instructions/redeem.rs`. It was written blind against the
   harness; nothing in it has been through the compiler yet. Things most likely to
   need touching:
   - `BasketSigner::new(&basket)` — confirm the constructor name/signature in
     `sleeve.rs` (may be a different helper; `seed.rs`/`mint.rs` show the real one).
   - `ComponentAccounts` field names used: `position`, `position_ai`, `vault`,
     `user_ata`, `mint`, `available`, `vault_frozen`, `user_frozen`. Check against
     `positions.rs`; `position_ai` (the raw AccountInfo for the Position PDA,
     needed to write back `owed`) may not exist yet and needs adding.
   - `math::liquidity_share(liquidity, net, h)` and
     `math::component_for_shares(net, available, h, Rounding::Down)` — confirm names
     in `math.rs`; add if missing (`L·net/H` floor, `available·net/H` floor).
   - `Redemption` fields assumed: `bump, basket, holder, shares, position_count,
     paid_count, paid_mints: Vec<Pubkey>, created_at`. `FrozenClaim` fields assumed:
     `bump, basket, wallet, mint, amount, created_at`. Check `state/`; if
     `paid_mints` is not there, add it (with `#[max_len(...)]`) or switch the
     duplicate check to a bitmap.
   - Seeds constants assumed: `seeds::REDEMPTION`, `seeds::CLAIM`, `seeds::FEES`,
     `seeds::POSITION`, `seeds::BASKET`. Harness uses `["claim", share_mint, wallet,
     mint]` and `["redemption", share_mint, holder]`.
   - Errors assumed: `NotSeeded`, `PoolMismatch`, `ComponentFrozen`, `DuplicateMint`,
     `ComponentMismatch`, `ZeroAmount`, `MathOverflow`.
   - `damm::position_nft_account`, `damm::pool_authority`, `damm::token_vault`,
     `damm::event_authority` — same helpers `seed.rs`/`mint.rs` use; match names.
   - `read_token_amount` is imported from `positions.rs`; may live elsewhere.
   - `Basket::touch(now)` — exists if create/mint use it, otherwise drop the calls.
2. `cargo.exe test -p basket --test redeem -- --test-threads=1 --nocapture`
   (`tests/redeem.rs`, 6 tests, never run). Then the full suite:
   `cargo.exe test -p basket -- --test-threads=1`.
3. Windows notes: use `cargo.exe` explicitly (a stray 0-byte `C:\Windows\system32\cargo`
   file triggers an "open with" popup otherwise). Intermittent `LNK1104 cannot open
   file ...exe` during `anchor build` is a file-lock race (McAfee) — just rerun.

## 3. What changed this session (Phase 2 → 3)

Program (`programs/basket/src/`):

- `instructions/redeem.rs` (new) — `Redeem`, `RedeemBegin`, `RedeemComponents`,
  `ClaimFrozen` account structs + handlers. Design:
  - Gross `shares` from holder: fee (`fees.redeem_fee_bps`) transferred as shares to
    the FeeVault share ATA, **net burned**. No gate, no pause check (D9).
  - `H = supply − shares in our pool position + pending_redeem_shares`, read before
    the burn.
  - Pool leg: `remove_liquidity(L·net/H)` with `token_b_account = holder's wSOL ATA`
    (created idempotently, closed to SOL if we created it) and `token_a_account =
    basket share ATA`, whose contents are then burned (treasury shares).
  - Components: `available·net/H` per leg (`available = vault − Position.owed`),
    `transfer_checked` from vault (basket PDA signer). Frozen vault or frozen holder
    ATA → find the `FrozenClaim` PDA among trailing remaining accounts (missing →
    `ComponentFrozen`), create/top-up it (rent from holder), `Position.owed += amount`.
  - `redeem_begin`: burn + fee + pool leg, opens `Redemption { shares: net,
    position_count, paid_count: 0, paid_mints: [] }`, `pending_redeem_shares += net`.
    `redeem_components(count)`: pays the first `count` component groups from
    remaining accounts, rejects mints already paid (`DuplicateMint`), closes the
    Redemption and decrements `pending_redeem_shares` once `paid_count >=
    position_count`.
  - `claim_frozen`: transfers `claim.amount` from vault to wallet ATA (created if
    missing), `Position.owed −= amount`, closes the claim to the wallet.
- `instructions/mint.rs` — creation-unit rule now prices off `c.available` (vault −
  owed) instead of raw vault balance; imports `COMPONENT_GROUP`; exact remaining
  account length check `n * COMPONENT_GROUP`.
- `instructions/positions.rs` — `ComponentAccounts.available`,
  `ComponentAccounts::trailing(remaining, n)`, and
  `load_components(basket_key, remaining, n)` (parses first `n` groups, requires
  `remaining.len() >= n*4`).
- `instructions/mod.rs`, `lib.rs` — `redeem(shares)`, `redeem_begin(shares)`,
  `redeem_components(count: u16)`, `claim_frozen()` wired.

Tests (`programs/basket/tests/`):

- `redeem.rs` (new, unrun): `redeem_pays_pro_rata_components_and_sleeve`,
  `redeem_is_never_gated_or_paused`, `redeem_validations`,
  `frozen_component_is_skipped_into_a_claim`, `two_step_redeem_for_large_books`
  (N=9, chunks `[..5]`, `[5..]`), `redeem_account_counts_and_cu_by_size`.
- `common/redeem.rs` (new, unrun) — ix builders + `freeze`/`thaw` helpers;
  `RedeemComponents { count: chunk.len() }` now matches the program.
- `common/mod.rs` — freezable mints (`create_mint_with_freeze`,
  `launch_fixed_freezable`), `launch_partial`, `set_paused`; every tx gets
  `SetComputeUnitLimit(1_400_000)` appended last.

## 4. Measurements and hard limits (keep in README)

- Mainnet **64 account-lock limit** (`increase_tx_account_lock_limit` not active):
  single-tx `seed` N ≤ 9 (26 fixed + 4N), `mint` N ≤ 10 (23 fixed + 4N). N=10 seed
  → `TooManyAccountLocks` (tested).
- **Instruction trace limit 64** → ≤ 8 positions per `create_basket` /
  `add_positions` tx (`POSITIONS_PER_TX = 8`); harness chunks automatically.
- Default 200k CU is not enough anywhere with CPI; tests request 1.4M.
- create_basket(6) 281k CU; add_positions(7) 240k (~34k/position).
- seed N=2/5/8/9: 34/46/58/62 accounts, 269k/331k/323k/362k CU; rent+fees ≈ 0.032 SOL
  paid by the first buyer.
- mint N=2/5/8/9: 30/42/54/58 accounts, 121k/134k/149k/164k CU.
- Meteora cp-amm: creator = basket PDA (owns position NFT), payer = buyer (system
  program refuses to debit a data-carrying PDA); base fee layout is
  PodAlignedFeeTimeScheduler (cliff u64@0, mode u8@8, periods u16@14,
  period_frequency u64@16, reduction_factor u64@24); scheduler ≤ 1 day; protocol
  takes 20% of swap fees before our split.
- Sleeve geometry: SOL leg `B = r·D` is primary, pool price = NAV; share side follows
  the range (≈2.207× nominal for Bounded [0.5×, 8×], 3.414× FloorOnly, 1× Full).

## 5. Open design questions for Nick (not decided — flagged)

Places where the spec was silent and I picked a placeholder so work could continue.
Each is easy to change; none are baked into account layouts that other phases depend
on yet, except (a) and (e).

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
