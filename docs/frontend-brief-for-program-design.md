# Frontend brief for the Anchor program design

Read this before designing the on-chain contracts. It describes what the web app
already does, the exact data it produces at launch, and the decisions that are
already fixed. The program has to accept what is described here; the frontend
does not get redesigned to fit the program.

Repo: `basketfun/` (monorepo). Product name: **basketlabs.fun**.

```
apps/web            React 19 + Vite. The site.
packages/core       Pure JS shared by web and (later) keeper/API. Constants,
                    launch payload builder, weight math, strategy contract.
                    This is the single source of truth for the wire schema.
packages/data       Jupiter / GeckoTerminal / RPC clients, token normalisation.
packages/solana     Solana helpers (Anchor client goes here).
programs/basket     Anchor 1.2.0 workspace. Skeleton only: constants, one error
                    enum, empty instruction and state modules, LiteSVM smoke tests.
docs/TODO.md        Running launch checklist.
```

Toolchain already installed and verified on the dev machine: Anchor CLI 1.2.0,
Solana CLI 4.3.0 (platform-tools v1.57), host Rust 1.96.1, LiteSVM 0.16 for
in-process tests. `anchor-spl` with Token-2022 features is a dependency.

---

## 1. The four basket types

`BasketType` is a `u8` enum. The JS and Rust values already agree and are
exported in the IDL as constants.

| Value | Key        | UI name   | Who controls contents after launch                           |
| ----- | ---------- | --------- | ------------------------------------------------------------ |
| 0     | `Fixed`    | Fixed     | Nobody. Immutable forever.                                   |
| 1     | `Mirror`   | Mirror    | The host wallets' holdings, via the keeper. Not the creator. |
| 2     | `Managed`  | Managed   | The creator, at will (timelock / turnover cap TBD).          |
| 3     | `Strategy` | Automated | Code pinned by hash at launch, run by the keeper. Nobody.    |

Trust ranking shown to users: Fixed (lowest risk) < Mirror ≈ Strategy (medium)
< Managed (highest).

The program must make these guarantees verifiable on-chain:

- Fixed: no instruction exists that can change contents after `create`.
- Mirror and Strategy: only the **keeper** authority can submit a new book, and
  for Strategy the book must be attributable to the pinned code hash.
- Managed: only the **creator** can submit a new book.

---

## 2. Launch flow in the UI (what the user actually does)

Steps differ by type. `packages/core/src/constants.js` `STEPS`,
`apps/web/src/features/launch/useDraft.js` `stepsFor()`.

| Type     | Steps                                                                                         |
| -------- | --------------------------------------------------------------------------------------------- |
| Fixed    | Type → Assets (search/pick) → Allocation (weights) → Details → Review                         |
| Managed  | same as Fixed                                                                                 |
| Mirror   | Type → Wallet (paste 1–20 host addresses) → Allocation (read-only preview) → Details → Review |
| Strategy | Type → Strategy (write 3 functions) → Test (sandbox run) → Details → Review                   |

Every step edits one `draft` object (`useDraft.js`). At Review the draft is
converted to the launch payload by `buildLaunchParams()` and printed verbatim.
Submit is currently a dry run (`submitLaunch()` in `packages/core/src/params.js`)
that validates and returns `{ status: 'DRY_RUN', reason: 'PROGRAM_NOT_DEPLOYED', params }`.
That call is the seam where the Anchor client goes.

The connected wallet (Wallet Standard; Phantom is the primary target) is the
`creator`. It signs the launch transaction and receives creator fees.

---

## 3. The launch payload (`create_basket` args)

Built by `buildLaunchParams(draft, { creator })`. Field names are snake_case
on purpose to mirror the Rust struct one-for-one. Everything the program needs
is here; anything not here does not exist yet.

```ts
{
  basket_type: 0 | 1 | 2 | 3,          // u8, BasketType
  name: string,                        // ≤ 32 chars (NAME_MAX)
  symbol: string,                      // ≤ 10 chars, uppercased (SYMBOL_MAX)
  uri: string,                         // off-chain metadata URI. '' today (upload not built)
  metadata: {                          // off-chain only. NOT sent on-chain; pinned behind `uri`
    description: string,               // ≤ 280
    image: string | null,
    twitter: string | null,
    telegram: string | null,
    website: string | null,
  },
  assets: Array<{                      // sorted by weight_bps desc, weight_bps > 0 only
    mint: string,                      // base58
    weight_bps: number,                // u16
    token_program: string,             // TOKEN or TOKEN_2022 program id (Token-2022 is real: xStocks, some majors)
    decimals: number,                  // u8
  }>,
  host_wallets: string[],              // Mirror only, 1..=20 (MAX_HOSTS). [] for every other type
  host_weighting: 0 | 1 | null,        // Mirror only. HostWeighting: 0 Equal, 1 Value. null otherwise
  strategy: {                          // Strategy only. null otherwise
    hash: string,                      // sha256 hex (64 chars) of the assembled module — what goes on-chain
    source: { select: string, weight: string, hold: string },  // published off-chain so anyone can verify hash(source)
  } | null,
  initial_share_price_usd_micro: 1000000000,  // $1,000 in 6-dp micro-dollars (USDC scale). Every basket.
  creator: string | null,              // connected wallet pubkey
}
```

### Invariants the frontend already enforces (program must re-enforce)

From `validateParams()`:

- `name`, `symbol` non-empty.
- `assets.length >= 1` (UI requires `MIN_ASSETS = 2`).
- Fixed: `assets.length <= 20` (`MAX_ASSETS_FIXED`). Mirror / Managed /
  Strategy: **no product-level cap**. If compute forces a ceiling, measure on
  devnet first and put the number in the program, not the UI.
- `sum(weight_bps) == 10_000` exactly (`BPS_TOTAL`). Normalisation is done by
  `normalizeWeights()`: floor each, give the rounding remainder to the largest
  position. Program should assert the sum, not re-normalise.
- Mirror: `host_wallets.length >= 1`.
- Strategy: `strategy.hash` present.

### Example payloads

Fixed:

```json
{
  "basket_type": 0,
  "name": "Solana Blue",
  "symbol": "BLUE",
  "uri": "",
  "assets": [
    {
      "mint": "So11111111111111111111111111111111111111112",
      "weight_bps": 5000,
      "token_program": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
      "decimals": 9
    },
    {
      "mint": "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN",
      "weight_bps": 2500,
      "token_program": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
      "decimals": 6
    },
    {
      "mint": "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R",
      "weight_bps": 2500,
      "token_program": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
      "decimals": 6
    }
  ],
  "host_wallets": [],
  "host_weighting": null,
  "strategy": null,
  "initial_share_price_usd_micro": 1000000000,
  "creator": "7xKq...9bA2"
}
```

Mirror (two hosts, equal weighting). `assets` is the **snapshot at launch**;
the keeper replaces it on every rebalance:

```json
{
  "basket_type": 1,
  "name": "fomo10",
  "symbol": "FOMO",
  "assets": ["...combined book from the hosts' current holdings, bps sum 10000..."],
  "host_wallets": ["AVzP2GeRmqGphJsMxWoqjpUifPpCret7LqWhD8NWQK49", "9WzD...AWWM"],
  "host_weighting": 0,
  "strategy": null,
  "initial_share_price_usd_micro": 1000000000,
  "creator": "..."
}
```

Strategy:

```json
{
  "basket_type": 3,
  "name": "Top volume small caps",
  "symbol": "TVSC",
  "assets": ["...the book the test run produced, bps sum 10000..."],
  "host_wallets": [],
  "host_weighting": null,
  "strategy": {
    "hash": "3f9a...c21e",
    "source": { "select": "async function select(ctx) {...}", "weight": "async function weight(ctx, mints) {...}", "hold": "function hold(ctx, mint) {...}" }
  },
  "initial_share_price_usd_micro": 1000000000,
  "creator": "..."
}
```

---

## 4. Weights and the "book"

A **book** is `{ mint: weight_bps }` summing to 10 000. Everything that
changes basket contents is expressed as a new book. The program only ever
needs to store the current book (or a hash of it plus the positions) and
accept a new one from the right authority.

- Fixed: book set once at create.
- Managed: creator submits a new book (`submit_book` or `rebalance`).
- Mirror: keeper computes the book from host holdings and submits it.
  Combination rule is `combineHosts()` in `params.js`:
  - `Equal` (0): each host's own percentage book counts 1/N.
  - `Value` (1): every host's USD holdings are pooled, then normalised.
  - Overlapping holdings across hosts are summed.
- Strategy: keeper runs the pinned code, the output is a book, keeper submits
  it. See §5.

Rebalancing (selling/buying underlying to hit the new book) is a keeper /
program concern. Swap venue (Jupiter CPI vs direct AMM) is undecided.

---

## 5. Strategy (type 3): how the code becomes something the program can pin

All of this lives in `packages/core/src/strategy.js` and is already tested.

**Contract.** Exactly three functions, each in its own editor pane:

```ts
async function select(ctx): Promise<string[]> // which mints
async function weight(ctx, mints): Promise<Record<string, number>> // relative sizes, any scale
function hold(ctx, mint): boolean // keep a dropped position one more cycle?
```

**Assembly.** `assembleStrategy(sections)` concatenates the three sections in
order with a fixed footer:

```
<select source>

<weight source>

<hold source>

export default { select, weight, hold }
```

**Hash.** `hashStrategy(sections)` = SHA-256 of the UTF-8 bytes of the
assembled module, as 64 hex chars. **This is `strategy.hash` in the payload and
is what the program stores.** The source is published off-chain (currently
alongside the payload; later IPFS/R2 behind `uri`) so anyone can recompute the
hash. The program never sees or executes JS; it stores 32 bytes.

**Static lint** (before any run): rejects `import`, `export`, `require`,
`eval`, `Function(`, `globalThis`, `self`, `window`, `process`, `fetch`,
`XMLHttpRequest`, `WebSocket`, `importScripts`, `postMessage`, and requires
each section to declare a function with its own name.

**Test run** (the "Test" step). `apps/web/src/features/launch/strategy/runner.js`
spawns a Web Worker with network globals removed. The worker only talks to the
host through a `ctx` object; the host answers data calls:

- `ctx.rpc(method, params)` – read-only Solana JSON-RPC via `/api/rpc`.
- `ctx.data.tokens(mints)`, `ctx.data.prices(mints)`, `ctx.data.top(list, opts)` – Jupiter.
- `ctx.programs.{TOKEN, TOKEN_2022}`, `ctx.memo`, `ctx.basket.{current, heldFor()}`, `ctx.log()`, `ctx.now()`, `ctx.params` (reserved).

Budgets: 40 data calls, 20 s wall, 500 mints per data call. The full authoring
spec for humans/AI is `apps/web/public/strategy-spec.md`.

A successful run yields:

```ts
{ ok: true, selected: string[], book: Array<{ mint, weight_bps, token }>, hash: string, source: string, ms, calls }
```

The draft then sets `strategyHash = hash`, `assets = book tokens`,
`weights = book`. **Editing any section invalidates the hash and the book**;
the user must re-run before Review. So the payload's `assets` for a Strategy
basket is always the output of the exact code whose hash is being pinned.

**Important caveat already logged in TODO:** the browser worker is a stand-in.
Before Strategy baskets go live, the test run and every scheduled run must
execute server-side in an isolate (Cloudflare Worker / QuickJS) with CPU
limits. The program design should assume a single keeper authority submits
books for Mirror and Strategy baskets, and that every keeper run is logged
publicly (inputs + output) so it can be replayed against the pinned hash.

---

## 6. Decisions already made (do not reopen without reason)

- **$1,000 opening share price** for every basket
  (`initial_share_price_usd_micro = 1_000_000_000`). Lifetime return must read
  at a glance. The math with 6- vs 9-decimal share mints is still to be
  confirmed.
- **Elastic supply, mint/redeem at NAV.** Shares are minted when someone
  deposits and burned on redeem, priced off vault NAV. This conflicts with
  LaunchLab / DBC (they revoke mint authority), so the plan is a custom
  program with the share mint authority held by the basket PDA. Bonding-curve
  "graduation" is a UI concept, not a program requirement.
- **Program-owned vault per basket.** One ATA per underlying mint, owned by
  the basket PDA. Must handle Token-2022 (xStocks have a disabled transfer hook
  and a permanent delegate; still freely transferable).
- **Asset caps.** Fixed = 20. Mirror / Managed / Strategy = uncapped at the
  product level. Deployers manage tradeability via tracking filters (below),
  not a base-layer limit.
- **Mirror tracking filters** (not built, will be fields on the payload the
  program enforces at rebalance): min host position USD, min asset market cap,
  min hold time, min weight floor (dust), max single-asset weight, allow/deny
  mints.
- **Lazy creation.** Nothing goes on-chain at "Launch". The creator signs the
  payload (ed25519 message), we store it and list the basket as unfunded. The
  **first buy** transaction creates the accounts and pays rent (~0.01–0.03 SOL
  bundled into a buy that is already larger); the program verifies the
  creator's signature over the payload so the buyer cannot alter it. The
  creator therefore never needs SOL. Unfunded baskets expire off the list
  after N days.
- **Zero-supply close crank.** Keeper closes baskets with `supply == 0` and no
  activity for 14 days, refunding rent to whoever paid. Baskets with holders
  that go dead need a separate sunset path (post-launch).
- **No first-buyer edge.** Minting at NAV means snipers have no reason to
  fund launches; do not design around bot liquidity.
- **Managed baskets** need visible rules buyers can read before buying:
  timelock on book changes and a per-epoch turnover cap. Values TBD.
- **Fees.** Creation fee, management fee, protocol cut: model undecided.
  Creator is the fee recipient address in the payload.
- **Portfolio filtering.** Our share mints need a cheap on-chain marker
  (PDA derivation from the mint, or a metadata field) so the portfolio page
  can filter a wallet's token accounts to "ours" without an index sweep.

---

## 7. What the program skeleton already contains

`programs/basket/src/`:

- `constants.rs`: `BPS_TOTAL: u16 = 10_000`, `BASKET_TYPE_{FIXED,MIRROR,MANAGED,STRATEGY}: u8 = 0..3`, all `#[constant]` so they appear in the IDL.
- `error.rs`: one `#[error_code]` enum `BasketError { WeightsMustSumToTotal, InvalidBasketType }`. Anchor 1.x allows a single error enum per program; every error goes here.
- `instructions.rs`: empty; comment lists the planned set: `create_basket`, `mint_shares`, `redeem_shares`, `submit_book`, `rebalance`, `close_basket`.
- `state.rs`: empty; planned `Basket` (config, type, authorities, share mint, book hash / strategy hash) and `Position` (one per underlying mint).
- `tests/smoke.rs`: program loads into LiteSVM; constants match the JS values.
- Program id is a localnet placeholder; mainnet id assigned at first deploy.

Testing rule for this repo: **tests are written before the handler they
cover**, one file per instruction, LiteSVM in-process. A validator run before
devnet for CPI paths.

---

## 8. What the frontend expects back

- From `create` (or the first buy under lazy creation): the basket address and
  share mint so the Result step can link to the token page.
- From an indexer (not yet built) for every basket: `type`, `ageH`,
  `marketCap`, `volume24h`, `change24h`, `nav`, `price`, `raised`, holders,
  and the current book. The Explore, Search, Filters and Leaderboard code
  already consumes exactly this shape from mock data.
- For the portfolio: a way to identify our share mints among a wallet's token
  accounts (see marker above), plus indexed buys/sells per wallet for PnL.
- Errors surfaced as the `BasketError` variants so the UI can map them to
  messages (weights sum, unknown type, unauthorised, timelock active, etc.).

---

## 9. Open questions the contract design must answer

1. Account layout for the book: fixed-size positions array with a program
   ceiling, or a separate `Position` account per mint (realloc-free, uncapped)?
2. Share mint decimals and how $1,000 opening price maps to the first mint.
3. Keeper authority model: single hot key, multisig, or per-basket keeper PDA
   with a rotatable signer?
4. Managed timelock and turnover cap values and how they are stored.
5. Rebalance mechanics: does the program swap (Jupiter CPI) or does the keeper
   swap into the vault and the program only verifies the resulting book is
   within tolerance?
6. Fee accrual points and where fees are held.
7. Lazy-create signature scheme: ed25519 program precompile vs. having the
   creator co-sign the first buy.
8. NAV oracle: on-chain pricing source for mint/redeem (Pyth? pool TWAPs?),
   and what happens for unpriceable positions.
