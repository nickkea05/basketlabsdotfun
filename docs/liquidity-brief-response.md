# Response to the liquidity & token-model brief

Pressure-test of the 90/10 self-funded pool plan against the programs as they
exist today. Sources inspected are listed at the end; anything I could not
verify from a primary source is marked as such. No code.

Companion docs: `frontend-brief-for-program-design.md` (what the web app sends
at launch), `TODO.md`. `ANCHOR_PROGRAM_DESIGN.md` is referenced by the brief
but is not in this repo; I have not read it.

---

## 0. Headline

The 90/10 plan holds on R1–R9 with three changes, one of which is big:

1. **Mint in kind, not against SOL.** The brief's NAV formula needs
   `components_value`, i.e. an oracle, the moment a buyer hands the program SOL.
   If instead the buyer's client swaps into the components at book weights
   (Jupiter, client-side) and the program mints `shares = min_i(deposit_i /
vault_i) × outstanding` (the ETF creation-unit rule), **the program never
   prices anything**. The sleeve pairs at the pool's own ratio, and the very
   first buy sets the pool price from the deposit itself (`Y = X · r/(1−r)`
   treasury shares against `r·D` SOL). Redeem is already in kind. Zero oracles,
   zero keeper for the peg, and the arb loops in §3 of the brief still work
   because both legs (pool and NAV) are permissionless.
2. **Venue: Meteora DAMM v2, not Raydium CPMM.** Creation is ~0.022 SOL of
   rent vs. ~0.19 SOL on CPMM, of which 0.15 SOL is a non-refundable protocol
   fee. A $10 sleeve cannot pay $30. DAMM v2 also gives quote-only fee
   collection (fees land in SOL, claimable by our PDA), a permissionless
   `initialize_customizable_pool` with fee scheduler / rate limiter / fixed
   price range, and a Rust CPI crate that other Anchor programs (MetaDAO's
   launchpad) already use in production.
3. **First buy is a batch, not one transaction.** Component swaps cannot be
   CPI'd N times inside one transaction (Jupiter CPI cannot use lookup tables;
   1232-byte / 1.4M CU limits). The buyer signs a batch: `k` Jupiter swap
   transactions, then `create_basket`, then `seed` (transfers + pool + mint).
   Phantom shows one approval sheet for the batch (`signAllTransactions`).

Everything below is the detail and the citations.

---

## 1. Venue for the vault-owned pool

| Criterion                                                                    | Raydium CPMM `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C`                                                                                                                                                                                                                   | Meteora DAMM v2 `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`                                                                                                                                                                                                                                      | Meteora DLMM `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`                                         |
| ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Creation cost                                                                | **~0.19 SOL**: `create_pool_fee = 150_000_000` (0.15 SOL, paid to `DNXgeM9EiiaAbaWvwjHj9fQQLAX5ZsfHyvmYUNRAdNC8`, non-refundable) + rent for 6 PDAs.                                                                                                                          | **~0.022 SOL** for pool + one position (Meteora docs). Rent only; recoverable on close.                                                                                                                                                                                                            | ~0.25 SOL for a launch pool (bin arrays are rent-heavy).                                           |
| Min liquidity                                                                | None enforced.                                                                                                                                                                                                                                                                | None enforced.                                                                                                                                                                                                                                                                                     | None, but empty bins are useless.                                                                  |
| Token-2022 share mint                                                        | Allow-list at `Initialize`: `TransferFeeConfig`, `MetadataPointer`, `TokenMetadata`, `InterestBearingConfig`, `ScaledUiAmount`. Anything else → `NotSupportMint` unless an admin-created `SupportMintAssociated` PDA exists (2026-09 change removed the hardcoded whitelist). | Permissionless: `TransferFeeConfig`, `MetadataPointer`, `TokenMetadata`. Others need a token badge (Google form + Discord). `TransferHook` only if program id and authority are `None`.                                                                                                            | Token-2022 via `PermissionlessV2` pools (`initialize_lb_pair2`).                                   |
| Our planned mint (MetadataPointer + TokenMetadata + 0-bps TransferFeeConfig) | Passes.                                                                                                                                                                                                                                                                       | Passes.                                                                                                                                                                                                                                                                                            | Passes.                                                                                            |
| LP ownership by a PDA                                                        | Fungible LP token in a PDA-owned ATA. Fine.                                                                                                                                                                                                                                   | Position is a Token-2022 NFT; owner can be any pubkey. MetaDAO passes a Squads vault as `creator`. Fine.                                                                                                                                                                                           | Position account with owner pubkey. Fine.                                                          |
| Add/remove liquidity by CPI                                                  | `Deposit` / `Withdraw`. `raydium-cp-swap` has a `cpi` feature, but the Anchor-1.x build is on branch `chore/upgrade-anchor` (Anchor 1.0.2); `master` pins 0.32.1. Version friction with our Anchor 1.2.0.                                                                     | `add_liquidity` / `remove_liquidity` / `claim_position_fee` via the `cp-amm` CPI crate (`damm_v2_cpi::…` in MetaDAO's programs). Meteora documents "Rust integration with DAMM v2 through CPI and the quote library".                                                                              | `add_liquidity*` / `remove_liquidity*` / `rebalance_liquidity`. Heavier account sets (bin arrays). |
| Fee configuration                                                            | Fixed tiers via admin-created `AmmConfig` (index 0 = 0.25% trade fee; 12% of that to protocol, 4% to fund, LP keeps 0.21%). Fees compound into reserves.                                                                                                                      | `initialize_customizable_pool`: any `cliff_fee_numerator`, 5 base-fee modes (time linear/exp, market-cap linear/exp, rate limiter), optional dynamic fee, `collect_fee_mode` BothToken / OnlyB / Compounding. Fees do **not** auto-compound (except Compounding mode); position owner claims them. | Base fee + bin step fixed at creation; dynamic fee.                                                |
| Price range                                                                  | Full range only.                                                                                                                                                                                                                                                              | Fixed `sqrt_min_price` / `sqrt_max_price` at creation (concentrated) or full range.                                                                                                                                                                                                                | Discrete bins; position must be moved to follow price.                                             |
| Keeper needed for the pool to work                                           | No.                                                                                                                                                                                                                                                                           | No.                                                                                                                                                                                                                                                                                                | **Yes** once price leaves the seeded bins.                                                         |
| Jupiter instant routing                                                      | Yes ("Raydium CPMM" on the list).                                                                                                                                                                                                                                             | Yes ("Meteora DAMM V2" on the list).                                                                                                                                                                                                                                                               | Yes ("Meteora DLMM").                                                                              |
| DexScreener / Axiom / Photon / Fomo                                          | Indexed. Threshold behaviour not documented by those products; not verifiable from primary sources.                                                                                                                                                                           | Indexed. DBC graduates into DAMM v2 and most 2025–26 launchpads use it, so trench tooling treats it as the default post-graduation venue. Same caveat on thresholds.                                                                                                                               | Indexed.                                                                                           |

**Recommendation: DAMM v2, `initialize_customizable_pool`, position NFT held
by the basket PDA, `collect_fee_mode = OnlyB` (SOL).**

Why, in order:

- The 0.15 SOL CPMM creation fee is fatal for lazy creation. Under the brief's
  own numbers a $100 first buy has a $10 sleeve; the pool cannot cost $30 of
  it. DAMM v2's ~0.022 SOL is rent, so it comes back when a zero-supply basket
  is closed.
- SOL-only fee collection means pool fees arrive as SOL in the vault's
  position, which is exactly the asset the sleeve wants more of (N3, and the
  "sweep excess pool SOL into components" keeper duty becomes optional).
- The customizable pool exposes the anti-sniper toolkit (§3 below) without
  Meteora's involvement. CPMM's fee schedule is whatever tiers Raydium's admin
  has created.
- A fixed concentrated range is available with no keeper. Useful for the
  sleeve math (§4).
- CPI crate on Anchor 1.x exists and is used in production by a third party.

Costs of this choice: DAMM v2 fees must be claimed (a permissionless crank or
folded into `redeem`); the position is a Token-2022 NFT our program has to hold
in a PDA-owned Token-2022 account; and Meteora's upgrade authority becomes a
dependency for the sleeve (see risk 9).

DLMM stays where the brief put it: an optional shelf on top, never the base.

---

## 2. Can the first buy be one transaction?

No. Two hard walls:

- **Jupiter CPI cannot use address lookup tables**, so every account of every
  route sits in the 1232-byte transaction body. One route is typically 25–60
  accounts (Jupiter caps `maxAccounts` at 64 and warns that below ~50 routing
  quality collapses). Two component swaps already do not fit next to anything
  else; a 20-asset basket is out of the question. Jupiter's own docs describe
  Flash Fill as the workaround for exactly this, and Flash Fill is a
  client-driven transaction, not a CPI.
- **Compute.** 1.4M CU per transaction. A Jupiter route is 100–400k CU;
  creating 20 vault ATAs is ~20 × 25k; pool init ~150–200k; ed25519 verify,
  mint, metadata on top.

There is also a payload-size wall the brief did not list: the ed25519
precompile verifies the message bytes that are _in the transaction_. Twenty
`(mint, weight_bps)` pairs is 680 bytes before name, symbol, gate and nonce.
Signing `sha256(borsh(CreateBasketArgs))` instead of the raw args does not
help, because the args still have to be in the instruction for the program to
re-hash them. Conclusion: the creation transaction has to be its own
transaction, with the args stored into the basket account so later
transactions do not carry them.

**Proposed shape (client-side swaps, three program instructions):**

```
[batch signed once via signAllTransactions]

tx 1..k   Jupiter /build swaps, SOL → each component, output to the buyer's own ATAs.
          (k ≈ ceil(N / 2) with maxAccounts trimmed; measured, not assumed.)
          The buyer keeps r·D as SOL (or wSOL).

tx k+1    create_basket
            • ed25519 precompile ix: creator pubkey, sig, message = borsh(CreateBasketArgs)
            • program ix: introspects the sysvar, checks message == args, checks
              args.program_id / cluster / nonce, creates Basket PDA, share mint,
              metadata, N vault ATAs. Payer = buyer. Stores args on the Basket.

tx k+2    seed  (first buy only; later buys call `mint`)
            • transfers components from buyer ATAs → vault ATAs in book proportion
              (min-ratio rule; excess stays in the buyer's wallet)
            • mints X shares to buyer, Y = X·r/(1−r) treasury shares to the sleeve
            • CPI initialize_customizable_pool with (Y shares, r·D SOL), position NFT → basket PDA
```

For N ≤ ~8, `create_basket` and `seed` can probably be merged; measure in
LiteSVM before deciding. We already have LiteSVM wired up.

Atomicity: the batch is not atomic on its own. If tx k+2 fails after the swaps
landed, the buyer holds the components in their own wallet, which is the right
failure mode (nothing is stuck in a program account). A Jito bundle makes the
whole thing all-or-nothing if we want that later.

UX: the user sees one Phantom sheet listing 3–12 transactions. This is what
Jupiter's own multi-tx flows look like. From an external venue (Axiom, Fomo)
the buyer never sees any of it; they buy the share in the pool.

Later buys (`mint`) are the same minus creation: swaps, then one program tx
that transfers components, mints `X` to the buyer, mints sleeve shares at the
pool's current ratio and `add_liquidity`.

---

## 3. Anything closer to a launchpad token?

What each idea in the brief actually buys, checked against the programs:

| Idea                                              | Available?                                                                                                                                                                                                 | Satisfies                                                                                                      | Does not satisfy                                                                                                                                                                                                              |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| DAMM v2 fee scheduler / rate limiter on our pool  | Yes, permissionless via `initialize_customizable_pool` (`fee_scheduler_mode`, `cliff_fee_numerator`, `reduction_factor`, `period_frequency`; rate limiter is a separate base-fee mode; fee capped at 99%). | Launch-day sniper tax that goes to the vault (holders). Chart volume, "launch feel".                           | Pointless while the mint gate is Open: a sniper mints at NAV and sells into the pool. Only bites in Closed mode. Mutually exclusive: scheduler _or_ rate limiter, chosen at creation.                                         |
| DLMM single-sided shelf                           | Yes.                                                                                                                                                                                                       | Zero-slippage inside a bin; can post a shelf exactly at NAV.                                                   | Needs a keeper to re-centre and inventory to restock. Already ruled out as base.                                                                                                                                              |
| Raydium CPMM with program-owned LP                | Yes.                                                                                                                                                                                                       | Compounding fees, Burn & Earn compatibility.                                                                   | 0.15 SOL fee per pool; fixed fee tiers; Anchor version friction.                                                                                                                                                              |
| AMM with a virtual / oracle-referenced side       | **No** permissionless one. DBC has virtual reserves but fixes supply and migrates (R6). Orca, Raydium, Meteora spot pools are all real-reserve.                                                            | —                                                                                                              | Building one is the oracle-synthetic design the brief already rejected.                                                                                                                                                       |
| Pool program sweeps its SOL into components       | **No** AMM does this. Compounding mode reinvests fees into _the pool_, not into other assets.                                                                                                              | —                                                                                                              | Stays a keeper duty on our side, and sweeping pool SOL shrinks the very liquidity the sleeve exists to create. Recommend not doing it at all; let the sleeve be the sleeve.                                                   |
| Jupiter DEX integration so mint/redeem is a route | Technically documented (`jupiter-amm-interface`, `Amm` trait, `jupiter-amm-test-kit` parity tests), but gated: security audit, traction, team/backers, Jupiter forks your SDK.                             | Nothing at launch.                                                                                             | Our mint is `components → share` (N inputs) and redeem is `share → N outputs`; the `Amm` trait is a two-mint swap. Only a `SOL → share` mint would fit, and that needs pricing again. Not viable, and not day one regardless. |
| **Pool-first, arb-minted (ETF AP model)**         | Yes with what we already have.                                                                                                                                                                             | Every retail buy is chart volume; premium possible; backing and redeem intact because arbs mint/redeem at NAV. | Needs the pool to be deep enough for retail on day one, which means a large _first_ sleeve. See §4 front-loading. Not a different design, a parameter choice.                                                                 |

**Net:** nothing on Solana today gives a pool-from-block-one launchpad token
with elastic NAV-backed supply _except_ the structure the brief already has.
The concrete upgrades are (a) DAMM v2's fee scheduler for Closed-mode
baskets, (b) a fixed price range for capital efficiency, (c) a front-loaded
sleeve so the pool is trench-sized before TVL is. None of them changes the
program's trust model.

---

## 4. Sleeve ratio

Constant-product math, buy of `b` SOL against SOL reserve `R`: effective price
/ spot = `1 + b/R`, so "impact" ≈ `b/R`. Round-trip loss ≈ `2b/R`.

For **$500 buy, < 5% impact**: `R ≥ $10,000`on the SOL side. Pool SOL side is`r × cumulative deposits`, so:

| Basket TVL | `r` needed for $10k SOL side (full range) | With fixed range `[0.5×, 8×]` NAV (~2× efficiency) | With `[0.5×, 4×]` (~2.4×) |
| ---------- | ----------------------------------------- | -------------------------------------------------- | ------------------------- |
| $10k       | 100% (impossible)                         | ~50%                                               | ~42%                      |
| $100k      | 10%                                       | ~5%                                                | ~4%                       |
| $1M        | 1%                                        | ~0.5%                                              | ~0.4%                     |

Jupiter's routability gate (< 30% round-trip on $500) is looser: `R ≥ ~$3.3k`,
i.e. TVL ≥ ~$33k at r = 10% full range, ~$17k with a 2× range.

Efficiency figures are the standard concentrated-liquidity multiplier
`1 / (1 − 1/√k)` for a symmetric range factor `k`, applied loosely to an
asymmetric range; treat as ±30% until run through Meteora's `cp-amm` quote
library with real `sqrt_price` bounds. A range whose **upper** bound is hit
turns the pool one-sided (all SOL, no shares to sell) and the chart stalls, so
Closed-mode baskets, which are the ones that run premiums, want a high or
infinite upper bound. The lower bound is safe at ~0.5× because floor arb holds
price near NAV.

Suggested bands (creator picks within band; program enforces band):

| Type / mode                    | Default `r` | Band   | Range        |
| ------------------------------ | ----------- | ------ | ------------ |
| Fixed, majors index, gate Open | 7%          | 5–10%  | `[0.5×, 8×]` |
| Mirror, gate Open              | 10%         | 5–15%  | `[0.5×, 8×]` |
| Strategy, gate Window → Closed | 12%         | 8–20%  | `[0.5×, ∞)`  |
| Managed, gate Window → Closed  | 12%         | 8–20%  | `[0.5×, ∞)`  |
| Meme / sector, gate Closed     | 20%         | 15–25% | full range   |

**Front-loading (proposal, needs your call):** `r` as a step schedule on the
pool's SOL side rather than a constant: e.g. 30% until the pool holds 20 SOL,
then the type default. Gets a basket past the Jupiter gate at ~$3–5k TVL
instead of ~$30k, at the price of higher tracking error for the earliest
buyers. The rule is on-chain and readable, so it is disclosed.

---

## 5. Closed-end mechanics

Time-only, computed from immutable fields, no oracle, no keeper:

```rust
pub enum MintGate {
    Open,                                   // ceiling arb always on
    WindowUntil { close_ts: i64 },          // subscription window, then closed
    Closed,
}
pub struct ReopenSchedule {                 // Option<> on the basket
    anchor_ts: i64,   // first re-open
    period_s:  u32,   // e.g. 30 days
    open_s:    u32,   // e.g. 48 h
}
fn mint_is_open(now) -> bool {
    match gate { Open => true, Closed => sched(now), WindowUntil{close_ts} => now < close_ts || sched(now) }
}
fn sched(now) = schedule.map_or(false, |s| now >= s.anchor_ts && (now - s.anchor_ts) % s.period_s < s.open_s)
```

- Set in `CreateBasketArgs`, signed by the creator, immutable for Fixed / Mirror
  / Strategy. Whether a Managed creator may _tighten_ it later is a question
  below.
- `redeem` has no gate field at all. There is nothing to pause.
- Frontend computes "mint opens in 3d 4h" from the same three numbers.
- A TVL-target close (from the brief) needs pricing. Not recommended; a
  block-height or timestamp close is the on-chain-honest version.

---

## 6. Portfolio filter marker

Cheapest that is verifiable offline with no indexer:

- Share mint's `mint_authority = find_program_address(["share_auth", mint], basket_program)`.
- Client: `getTokenAccountsByOwner(wallet)` → collect mints →
  `getMultipleAccounts(mints)` → keep those whose `mint_authority` equals the
  PDA derived from the mint itself. One RPC round-trip, no metadata parsing,
  works for classic SPL or Token-2022, and cannot be spoofed (nobody else can
  make a PDA of our program the authority of their mint).
- Fast path: indexer publishes the share-mint set; client intersects.

A metadata field is strictly worse: parsing cost, Token-2022-only, and
copyable by anyone.

---

## 7. Risks not in the brief, ranked

1. **Third-party freeze on a component breaks R1 for that component.** xStocks
   have a freeze authority. If the issuer freezes a vault ATA, an in-kind
   `redeem` that touches that component fails, gating redemption of the whole
   basket. Design choice needed: `redeem` skips frozen components and records
   an IOU per (wallet, mint), or reverts. Recommend skip + claim.
2. **Redeem-arb bleed in drawdowns.** Every sell into the pool pushes price
   below NAV; arbs buy shares in the pool and redeem at NAV; the vault gave NAV
   and got price. It is bounded by pool depth × discount and is exactly the IL
   the brief accepts, but it is _continuous_ in a downtrend and it is paid by
   holders who stay. Worth an explicit number in the risk disclosure.
3. **Redeem of 20 components in one transaction.** 20 vault ATAs + 20 buyer
   ATAs + 20 mints + Token-2022 program + pool `remove_liquidity` accounts.
   Fits with an ALT and ~600k CU for classic SPL; Token-2022 transfers with
   `transfer_checked` are heavier. Likely needs a two-tx redeem above ~12
   assets. Measure.
4. **Rent floor on the first buy.** Basket + mint + metadata + N vault ATAs +
   pool + position ≈ 0.03 + 0.002·N SOL. A $20 first buy cannot pay it. The UI
   must enforce a minimum first buy (question below).
5. **Anyone can open a second pool for our mint.** DAMM v2 and CPMM are
   permissionless; a competing pool fragments liquidity and DexScreener may
   pick the wrong pair. Cannot be prevented; mitigated by ours being seeded
   first and deepest.
6. **Concentrated range upper bound.** Covered in §4: if premium exceeds
   `sqrt_max_price` the pool is all SOL and buys fail on external venues. Use a
   high or infinite upper bound on Closed-mode baskets.
7. **Token-2022 share mint vs. trench tooling.** Every pump / LaunchLab token is
   classic SPL + Metaplex metadata. Some bots, scanners and older wallets treat
   Token-2022 (and especially a `TransferFeeConfig`, even at 0 bps) as a
   warning flag; RugCheck-style tools score it down. If the only reason for
   Token-2022 is a reserved transfer fee, classic SPL is the safer default.
8. **Sleeve minting at pool ratio when pool ≠ NAV.** Treasury shares are minted
   at whatever the pool ratio is. At a premium fewer are minted (harmless); at a
   discount more are minted, but they sit in the pool and only become
   outstanding when someone pays SOL for them into the vault-owned position, so
   NAV of outstanding shares is unaffected. Needs to be stated precisely in the
   design doc so the audit does not flag it as unbacked minting.
9. **Meteora upgrade authority.** The sleeve (SOL + treasury shares) lives
   inside a program we do not control. Pump.fun's AMM has the same property and
   nobody cares, but it belongs in the risk disclosure and the audit scope.
10. **Jupiter grace period is token-age based, 30 days.** After that, a pool
    that fails the $500 round-trip test is delisted from routing until it
    passes again. Dead baskets vanish from aggregators; that is acceptable and
    matches R4's "dead baskets never appear".
11. **Creator-signature replay.** The signed message must bind program id,
    cluster, a creator nonce and the intended basket seed, or a payload signed
    for devnet can be replayed on mainnet (or twice).
12. **Fee scheduler is dead weight while the gate is Open.** Arbs mint at NAV
    and sell into the pool; the sniper tax is paid by nobody. Only enable it on
    Window/Closed baskets.

---

## 8. Alternative that beats 90/10 on R1–R9

None that I can find with today's programs. The only structural variant worth
naming is the **pool-first / arb-minted** posture from §3, and it is the same
program with a larger first sleeve, so it is a parameter, not a design. Every
other route either re-introduces an oracle (SOL-denominated mint, TVL-target
gates, virtual reserves), a keeper for the peg (DLMM shelf), or capital (any
seeded pool).

The one requirement that is not achievable exactly as stated is **R7 "nothing
on-chain until the first buy" combined with a $100 first buy**: rent for a
20-asset basket plus a pool is ~0.07 SOL. The closest achievable version is a
minimum first buy that covers rent (the UI can show "min 0.1 SOL to open this
basket"), or a smaller basket. Rent is recoverable on close, so it is a float,
not a cost.

---

## 9. Definition of "graduation"

Pick **(c)**, with objective, on-chain-computable definitions:

- **Listed** = the pool passes Jupiter's normal-routing gate (< 30% round-trip
  on $500), computed from pool reserves. This is the moment the token is
  tradeable in aggregators and wallets without visiting us.
- **Graduated** = mint gate has closed (basket is closed-end) **or**, for
  Open-gate baskets that never close, pool SOL side ≥ a fixed threshold (e.g.
  50 SOL). Otherwise index baskets could never graduate.

---

## Sources inspected

- Raydium docs (MCP): `/products/cpmm/accounts` (Token-2022 allow-list, 2026-09
  whitelist removal, `create_pool_fee`), `/products/cpmm/fees` (AmmConfig index
  0 rates), `/products/cpmm/instructions` (`Initialize`, `Deposit`, `Withdraw`,
  `InitializeWithPermission`), `/products/cpmm/code-demos` (Rust CPI skeleton,
  Anchor branch note), `/quick-start/deploy-cpmm-pool` (~0.19 SOL), `/reference/program-addresses`
  (program ids, fee receiver, `create_pool_fee = 150000000`), `/reference/token-2022-support`.
- Meteora docs (via Solana MCP): "What is DAMM v2", "Token 2022 Extensions",
  "Pool Fee Configs" (`initialize_pool`, `initialize_pool_with_dynamic_config`,
  `initialize_customizable_pool`), "Rate Limiter", "Becoming a Liquidity
  Provider" (~0.022 SOL vs ~0.25 SOL DLMM), DLMM developer overview
  (`PermissionlessV2`, program id), DAMM v2 developer guide (program id, Rust
  CPI + quote library).
- MetaDAO `programs` repo (via Solana MCP): `v06/v07/v08_launchpad`
  `complete_launch.rs` / `settle_launch.rs`, production CPI into
  `damm_v2_cpi::initialize_customizable_pool` / `initialize_pool_with_dynamic_config`
  with a Squads vault as position owner.
- Jupiter developer docs: Market Listing (instant routing list incl. Raydium
  CPMM and Meteora DAMM V2; 30-day token-age grace; $500 round-trip and
  $1000-vs-$500 impact criteria), Build Swap Transaction / Common Instructions
  (CPI cannot use ALTs; `maxAccounts` 1–64; Flash Fill), DEX Integration
  (`jupiter-amm-interface`, audit + traction prerequisites).
- Not verifiable from primary sources: Axiom / Photon / Fomo listing thresholds.
