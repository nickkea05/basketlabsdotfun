# basketlabs.fun — program build confirmation

This document closes the design loop. It takes the pressure-test
(`liquidity-brief-response.md`), the frontend brief
(`frontend-brief-for-program-design.md`), and the earlier
`ANCHOR_PROGRAM_DESIGN.md`, and records the decisions. Where this file and any
earlier doc disagree, **this file wins**. Build against it. Nick will add
further instructions separately; treat those as amendments to this file.

Repo: `basketfun/` monorepo. Program: `programs/basket` (Anchor 1.2.0,
LiteSVM tests first, one test file per instruction, written before the
handler).

---

## 1. Decisions (final)

| #   | Topic                   | Decision                                                                                                                                                                                                                                                                                       |
| --- | ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Share mint              | **Classic SPL** + Metaplex metadata. Not Token-2022. No transfer fee now or later; pool swap fees are the churn fee.                                                                                                                                                                            |
| D2  | Mint authority / marker | `mint_authority = PDA(["share_auth", share_mint])`. Portfolio filter = derive PDA from each mint and compare. No metadata marker.                                                                                                                                                              |
| D3  | Pool venue              | **Meteora DAMM v2** (`cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`), `initialize_customizable_pool`, position NFT owned by the basket PDA, `collect_fee_mode = OnlyB` (SOL). CPI via the `cp-amm` crate. Raydium CPMM rejected (0.15 SOL non-refundable creation fee). DLMM not used.          |
| D4  | Mint pricing            | **In kind, oracle-free.** Client swaps SOL → components at book weights; program mints `shares = min_i(deposit_i / vault_i) × outstanding` (creation-unit rule). First mint sets price from the deposit itself. The program never prices anything.                                             |
| D5  | Sleeve                  | Every mint splits: `(1 − r)` of value into components (already delivered in kind), `r` into the pool as SOL plus treasury shares at the pool's current ratio. **Front-loaded:** `r = 25%` until the pool's SOL side ≥ 20 SOL, then the type default (§3). Rule is on-chain and displayed.       |
| D6  | Mint gate               | `MintGate { Open, WindowUntil{close_ts}, Closed }` + optional `ReopenSchedule { anchor_ts, period_s, open_s }`. Time-based only. **Immutable after creation for every type.** No tightening, no loosening. Redeem has no gate.                                                                   |
| D7  | Minimums                | Creation buy ≥ **1 SOL** (covers rent, opens the pool). Later site mints ≥ **0.1 SOL**. Pool trades: no minimum (not ours to set).                                                                                                                                                                |
| D8  | First buy               | A **batch** signed once (`signAllTransactions`): k Jupiter swap txs → `create_basket` (with ed25519 precompile over the creator-signed payload) → `seed` (deposit components, mint, open pool, deposit sleeve). Later buys: swaps → `mint`. Measure in LiteSVM whether `create_basket` + `seed` merge for N ≤ 8. |
| D9  | Redeem                  | In kind, pro-rata components **plus** pro-rata withdrawal of the vault's DAMM v2 position (SOL paid out, withdrawn treasury shares burned). Above ~12 assets, two transactions. Never pausable, no gate field.                                                                                   |
| D10 | Frozen components       | If a component ATA is frozen (xStocks freeze authority), `redeem` **skips it and records a claim** `(wallet, mint, amount)`; `claim_frozen` pays it out when unfrozen. Redeem never reverts because of a third party.                                                                            |
| D11 | Fees                    | Platform-set, identical across baskets. Creator's only dial: Managed performance fee within a band. See §4.                                                                                                                                                                                    |
| D12 | Performance fee         | Managed only. **Fund-level high-water mark**, crystallized monthly as newly minted shares to the creator, only when NAV > HWM. Nothing on buys, nothing while underwater. Band 0–20%, default 10%, set in the signed payload, immutable.                                                        |
| D13 | Fee scheduler           | DAMM v2 fee scheduler (launch sniper tax, decaying) enabled **only** on `WindowUntil` / `Closed` baskets. Off on `Open` (dead weight while ceiling arb is live).                                                                                                                                 |
| D14 | Treasury                | Protocol share → a **Squads multisig with a 48–72 h time lock** on outgoing proposals. Address and delay published. Buyback program deployed later, pointed at by the same treasury; not immutable by design (dev control retained, transparency via timelock).                                  |
| D15 | Rewards to holders      | Off-chain hold-time weighting (`balance × min(days_held, 30)`), on-chain Merkle root per epoch, pushed distribution in the basket's flagship reward mint. Creator and creator-controlled wallets excluded.                                                                                       |
| D16 | Graduation              | Two on-chain-computable states. **Listed** = pool passes Jupiter's routing gate (<30% round-trip on $500) from reserves. **Graduated** = mint gate has closed, or for `Open` baskets pool SOL side ≥ 50 SOL.                                                                                     |
| D17 | Keeper                  | Small rotatable signer set in `Config`. Duties: Mirror/Strategy `submit_book` + rebalance, rewards roots, fee sweeps, zero-supply closes. **No pool duties**; no sweeping of pool SOL into components.                                                                                          |
| D18 | Admin                   | Squads multisig. Can: rotate keeper, edit whitelist, pause mint/book changes, set fee bounds and defaults. Cannot: move vault or treasury funds, change any live basket's book, gate, or fees. Upgrade authority behind timelock.                                                                |
| D19 | Creator signature       | ed25519 precompile; message = borsh(`CreateBasketArgs`) + domain separator binding program id, cluster, creator nonce, basket seed. Prevents replay.                                                                                                                                             |
| D20 | Book storage            | `Position` PDA per mint; Fixed capped at 20 by validation, others uncapped; `add_positions` for large books; basket inert until `complete`.                                                                                                                                                      |

---

## 2. Accounts

| Account         | Seeds                                                          | Notes                                                                                                                                                                                                                                                                                                                                                       |
| --------------- | -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Config`        | `["config"]`                                                   | admin, keeper set, treasury (Squads vault), fee bounds/defaults, whitelist authority, pause flag                                                                                                                                                                                                                                                            |
| `Basket`        | `["basket", share_mint]`                                       | type, creator, name/symbol/uri, book_hash, position_count, complete, strategy_hash, hosts_hash, host_weighting, fees, managed_rules (timelock, turnover cap, epoch), perf_fee_bps, hwm_nav, last_crystallized, gate, schedule, sleeve params (r_step_bps, r_default_bps, step_threshold_lamports), pool, position_nft, created_at, last_activity_at, flags |
| `ShareAuth`     | `["share_auth", share_mint]`                                   | mint authority PDA (also the portfolio marker)                                                                                                                                                                                                                                                                                                              |
| `Position`      | `["position", share_mint, mint]`                               | weight_bps, token_program, decimals, vault ATA                                                                                                                                                                                                                                                                                                              |
| Vault ATAs      | ATA(`Basket`, mint)                                            | components, Token or Token-2022                                                                                                                                                                                                                                                                                                                             |
| Pool position   | DAMM v2 position NFT in a Token-2022 ATA owned by `Basket`     | the sleeve                                                                                                                                                                                                                                                                                                                                                  |
| `FeeVault`      | `["fees", share_mint]` + ATAs                                  | creator + holder shares, in kind, until swept                                                                                                                                                                                                                                                                                                               |
| `PendingBook`   | `["pending", share_mint]`                                      | Managed timelocked book                                                                                                                                                                                                                                                                                                                                     |
| `FrozenClaim`   | `["claim", share_mint, wallet, mint]`                          | D10                                                                                                                                                                                                                                                                                                                                                         |
| `RewardsRoot`   | `["rewards", share_mint, epoch]`                               | D15                                                                                                                                                                                                                                                                                                                                                         |
| `CreatorLock`   | `["lock", share_mint]`                                         | creator's locked self-position (leaderboard skin)                                                                                                                                                                                                                                                                                                           |
| `Whitelist`     | `["whitelist"]` (sharded if needed)                            | allowed mints + liquidity tier                                                                                                                                                                                                                                                                                                                              |

**NAV (off-chain and in tests):** `NAV = (components_value + pool_SOL) / (supply − treasury_shares_in_pool)`. The program never computes `components_value`; it only enforces pro-rata rules.

---

## 3. Sleeve and gate defaults by type

| Type / mode         | Default `r` after step | Band   | Pool price range | Gate default        |
| ------------------- | ---------------------- | ------ | ---------------- | ------------------- |
| Fixed, majors/index | 7%                     | 5–10%  | `[0.5×, 8×]`     | Open                |
| Mirror              | 10%                    | 5–15%  | `[0.5×, 8×]`     | Open                |
| Strategy            | 12%                    | 8–20%  | `[0.5×, ∞)`      | Window → Closed     |
| Managed             | 12%                    | 8–20%  | `[0.5×, ∞)`      | Window → Closed     |
| Fixed, meme/sector  | 20%                    | 15–25% | full range       | Closed after window |

Step phase for all: `r = 25%` until pool SOL ≥ 20 SOL. Pool swap fee: 1% meme/sector, 0.3% index/majors (fee scheduler on top for Window/Closed per D13). Creator picks within band in the signed payload; program enforces band.

---

## 4. Fees and splits

| Fee         | Level                       | Form                                                              | Split    |
| ----------- | --------------------------- | ----------------------------------------------------------------- | -------- |
| Mint        | 1.00%                       | in kind at `mint`/`seed`                                          | 40/30/30 |
| Redeem      | 0.25%                       | in kind at `redeem`                                               | 40/30/30 |
| Pool swap   | per §3                      | SOL, claimed from the position by crank or at redeem              | 40/30/30 |
| Management  | 1%/yr, Managed only         | share inflation to creator, accrued at `apply_book`/`crystallize` | creator  |
| Performance | 0–20%, Managed only         | share inflation to creator on HWM crystallization                 | creator  |
| Creation    | 0.1 SOL inside the 1 SOL first buy | SOL                                                        | protocol |

Split key: **holders 40% / creator 30% / protocol 30%**. Creator share is tiered by leaderboard rank at creation (20 / 30 / 40 of the creator line), written into `Basket.fees`, not re-evaluated. Protocol never the largest line. Numbers are defaults in `Config`, changeable for _new_ baskets only.

Fee line goes into the token's Metaplex description so venues render it ("Basket · mint 1% · perf 10%").

---

## 5. Instructions

| Instruction                                | Signer                                        | Notes                                                                                                                                                                                                                                                      |
| ------------------------------------------ | --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `initialize_config`                        | admin                                         | once                                                                                                                                                                                                                                                       |
| `create_basket`                            | payer (buyer)                                 | verifies creator's ed25519 sig (D19); creates Basket, share mint (SPL), metadata, ShareAuth, Positions, vault ATAs; stores args; rent from payer                                                                                                           |
| `add_positions`                            | payer                                         | for books beyond one tx; sets `complete` when count matches                                                                                                                                                                                                |
| `seed`                                     | buyer                                         | first mint: transfer components (min-ratio rule), mint X to buyer and Y = X·r/(1−r) treasury shares, CPI `initialize_customizable_pool` with (Y, r·D SOL), position NFT → Basket; enforces 1 SOL minimum; sets pool fee config + scheduler per type       |
| `mint`                                     | buyer                                         | gate check; min 0.1 SOL equivalent; transfer components; mint X + treasury shares at pool ratio; CPI `add_liquidity`                                                                                                                                       |
| `redeem`                                   | holder                                        | burn; pro-rata components (skip frozen → `FrozenClaim`); CPI `remove_liquidity` pro-rata; pay SOL; burn withdrawn treasury shares; fee in kind                                                                                                             |
| `claim_frozen`                             | holder                                        | D10                                                                                                                                                                                                                                                        |
| `submit_book`                              | creator (Managed) / keeper (Mirror, Strategy) | Managed → `PendingBook`; others → target book + rebalance window                                                                                                                                                                                           |
| `apply_book`                               | anyone                                        | Managed: after timelock, within turnover cap; accrues mgmt fee                                                                                                                                                                                             |
| `execute_swap`                             | keeper                                        | Jupiter CPI with Basket as authority; in/out mint checks, per-position sell cap, slippage vs keeper attestation, output must land in vault                                                                                                                 |
| `finalize_rebalance`                       | keeper                                        | tolerance check, create/close Positions, set `book_hash`                                                                                                                                                                                                   |
| `crystallize`                              | anyone                                        | Managed: monthly; if NAV attestation > `hwm_nav`, mint perf-fee shares to creator, update HWM (NAV attested by keeper for this one purpose; off-chain-verifiable)                                                                                          |
| `claim_pool_fees`                          | anyone                                        | CPI `claim_position_fee`; SOL to `FeeVault`/treasury per split                                                                                                                                                                                             |
| `sweep_fees`                               | anyone                                        | creator share out; holder share to rewards distributor; protocol share to treasury                                                                                                                                                                         |
| `post_rewards_root` / `distribute_rewards` | keeper / anyone                               | D15                                                                                                                                                                                                                                                        |
| `lock_creator_shares` / `unlock_creator_shares` | creator                                  | leaderboard skin                                                                                                                                                                                                                                           |
| `close_basket`                             | keeper                                        | supply == 0, idle 14 d, refund rent to original payer                                                                                                                                                                                                      |
| `pause` / `unpause`                        | admin                                         | mint + book changes only                                                                                                                                                                                                                                   |
| `update_whitelist`, `rotate_keeper`        | admin                                         |                                                                                                                                                                                                                                                            |

Events on every state change (create, seed, mint, redeem, book, rebalance, crystallize, fees, rewards, lock, close) with slot and wallet. The indexer (follower PnL, leaderboard, portfolio) is built on these plus SPL transfers.

Errors, single enum: `WeightsMustSumToTotal`, `InvalidBasketType`, `MintNotWhitelisted`, `DuplicateMint`, `TooManyAssets`, `BadTokenProgram`, `BadSignature`, `ReplayDetected`, `NameTooLong`, `SymbolTooLong`, `FeeOutOfBounds`, `SleeveOutOfBand`, `MintGateClosed`, `BelowMinimum`, `BasketIncomplete`, `ImmutableBasket`, `Unauthorized`, `TimelockActive`, `TurnoverExceeded`, `RebalanceWindowClosed`, `SlippageExceeded`, `OffBook`, `ComponentFrozen`, `NothingToCrystallize`, `Paused`.

---

## 6. Batch and UX contract (frontend ↔ program)

- Site **Buy** = one click. App quotes, builds `k` Jupiter swaps + `mint` (or `create_basket` + `seed` on first buy), submits via `signAllTransactions`. Wallet shows one sheet. Loading state = jenga stack building per confirmed tx.
- Site **Sell** = `redeem` + Jupiter swaps of the received components to SOL in one batch.
- Venue buys/sells (Axiom, Fomo, DexScreener → Jupiter) hit the DAMM v2 pool only. Holders redeem later on the site with the same wallet. No linking, no allowlist: the share is the right.
- Failure mid-batch leaves components in the buyer's wallet, never in a program account. UI offers retry or sell-to-SOL.
- Result step needs basket address, share mint, pool address.

---

## 7. Security and process

- Admin and treasury on Squads; outgoing treasury proposals time-locked 48–72 h; addresses and delays published.
- Program upgrade authority = admin multisig behind timelock; verified build published.
- No instruction can move vault funds except `redeem` (to the redeeming holder) and `execute_swap` (into the vault). No admin path to `FeeVault`, pool position, or vault.
- Redeem is unpausable by construction (no pause check in handler).
- Tests before handlers, LiteSVM, one file per instruction; validator run for DAMM v2 and Jupiter CPI paths before devnet; measure CU and account counts for `seed`, `mint`, `redeem` at N = 2, 8, 12, 20.

---

## 8. Build order

1. `Config`, `create_basket` (+ ed25519 introspection, replay binding), `add_positions`, `Position`, whitelist. Tests.
2. `seed` and `mint` with DAMM v2 CPI (pool init, add liquidity) and the sleeve step rule. Tests incl. pool ratio and treasury-share accounting.
3. `redeem` incl. `remove_liquidity`, frozen-component skip, two-tx path. Tests.
4. Fees: in-kind skims, splits, `claim_pool_fees`, `sweep_fees`, `FeeVault`. Tests.
5. Gates and schedules, `crystallize` + HWM, management fee accrual. Tests.
6. Books: `submit_book`/`apply_book`/`execute_swap`/`finalize_rebalance` with Jupiter CPI. Validator tests.
7. Rewards roots + push distribution, creator lock, close crank, pause/admin.
8. Devnet: CU measurements, batch UX end to end, indexer event coverage.

---

## 9. Items Nick will confirm or amend

- Exact `Config` defaults if different from §3/§4.
- Whether the step threshold is 20 SOL of pool SOL (default here) or a TVL-based number.
- Managed timelock / turnover cap values (defaults 24 h / 30 % per 7 d).
- Reward epoch (default 6 h) and flagship reward mint rule.
- Whether $BSKT ever takes a governance role (out of scope now; keep `Config` authorities upgradeable to a governance PDA later).
