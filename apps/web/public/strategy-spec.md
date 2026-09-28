# basketlabs.fun strategy authoring specification

Audience: an AI coding assistant (or an engineer) producing a strategy for an
**Automated basket** on basketlabs.fun. Read the whole document before writing code.
The output format at the end is mandatory.

## 1. What a strategy is

A basket is a Solana token whose vault holds a set of underlying SPL tokens in
fixed proportions ("the book"). An Automated basket's book is recomputed on a
schedule by running three user-supplied JavaScript functions inside a sandbox.
The sandbox has **no network, no imports, no filesystem, no globals**. The only
way to reach the outside world is the `ctx` object passed to each function.

The three functions run in order, once per rebalance cycle:

| Order | Signature                                                                      | Purpose                                                      |
| ----- | ------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| 1     | `async function select(ctx): Promise<string[]>`                                | Which mints belong in the basket right now.                  |
| 2     | `async function weight(ctx, mints: string[]): Promise<Record<string, number>>` | Relative size of each selected mint.                         |
| 3     | `function hold(ctx, mint: string): boolean`                                    | Whether to keep a currently-held mint that `select` dropped. |

The runtime then: deduplicates `select`'s output, drops mints that cannot be
priced, calls `weight` with the survivors, discards non-positive weights,
normalises the rest to 10,000 basis points, and rebalances the vault to match.
For each mint in the previous book that is absent from the new one, `hold` is
consulted; `true` keeps it at its previous weight for one more cycle.

The source of all three functions is hashed (SHA-256 of the assembled module)
and the hash is stored on-chain at launch. **The code can never be changed.**
Every scheduled run publishes its inputs and outputs so it can be replayed.

## 2. Language and environment

- ECMAScript 2023, module semantics, strict mode. `async`/`await`, optional
  chaining, spread, `Map`/`Set`, `BigInt` are available.
- Each function must be a **standalone declaration** with exactly the name in
  the table above. Helper functions may be declared in the same section.
- The following are rejected at lint time: `import`, `export`, `require`,
  `eval`, `new Function`, `globalThis`, `self`, `window`, `process`, `fetch`,
  `XMLHttpRequest`, `WebSocket`, `importScripts`, `postMessage`. Do not use them
  and do not try to alias them.
- No `console`. Use `ctx.log(...)`.
- Determinism is not required but is encouraged: prefer inputs from `ctx` over
  `Math.random()` or wall-clock branching.

## 3. Budgets (enforced; exceeding any fails the run)

| Budget                                                 | Limit                                                            |
| ------------------------------------------------------ | ---------------------------------------------------------------- |
| Data calls per run (`ctx.rpc` + `ctx.data.*` combined) | 40                                                               |
| Wall time per run                                      | 20 seconds                                                       |
| Mints per `ctx.data.*` call                            | 500                                                              |
| Output mints                                           | unbounded, but every mint costs a swap at rebalance; prefer < 60 |

A failed run leaves the previous book in place. It never produces a partial
rebalance.

## 4. The `ctx` object

```ts
interface Ctx {
  /** Solana JSON-RPC, mainnet, read-only. Returns `result` of the response. */
  rpc(method: string, params?: unknown[]): Promise<unknown>

  data: {
    /** Token metadata for known mints. Unknown mints are `null`. */
    tokens(mints: string[]): Promise<Array<Token | null>>
    /** USD price per mint. Unknown or unpriced mints are `null`. */
    prices(mints: string[]): Promise<Record<string, number | null>>
    /**
     * Ranked discovery lists. This is how a strategy finds tokens it does not
     * already know. `limit` is capped at 100 (the upstream ceiling); filter
     * client-side from there.
     *   'traded'   – by volume over `interval`
     *   'trending' – upstream trending score over `interval`
     *   'organic'  – by organic score (bot-filtered activity) over `interval`
     *   'recent'   – newest pools; `interval` is ignored
     */
    top(list: 'traded' | 'trending' | 'organic' | 'recent', opts?: { interval?: '5m' | '1h' | '6h' | '24h'; limit?: number }): Promise<Token[]>
  }

  /** Well-known program ids. */
  programs: { TOKEN: string; TOKEN_2022: string }

  /** Launch-time parameters. Empty today; reserved for parameterised strategies. */
  params: Readonly<Record<string, unknown>>

  /** Scratch object shared by select/weight/hold within one run. Empty at start. */
  memo: Record<string, unknown>

  basket: {
    /** Current book as { mint: weight_bps }. Empty on a test run. */
    current: Readonly<Record<string, number>>
    /** Seconds since `mint` entered the basket; 0 if not held. */
    heldFor(mint: string): number
  }

  /** Emits to the run log. Arguments are JSON-serialised. */
  log(...args: unknown[]): void
  /** Milliseconds since epoch at call time. */
  now(): number
}

interface Token {
  mint: string
  symbol: string
  name: string
  decimals: number
  icon: string | null
  price: number | null // USD
  mcap: number | null // USD, circulating — see caveat below
  fdv: number | null // USD, fully diluted
  liquidity: number | null // USD across pools
  holders: number | null
  organicScore: number | null // 0–100, higher = less bot-driven
  verified: boolean
  tokenProgram: string // TOKEN or TOKEN_2022
  tags: string[] // e.g. 'verified', 'xstocks', 'lst'
  createdAt: string | null // ISO timestamp of the first pool
  volume: { m5: number | null; h1: number | null; h6: number | null; h24: number | null } // USD, buys + sells
  change: { m5: number | null; h1: number | null; h6: number | null; h24: number | null } // price change, percent
  traders: { m5: number | null; h1: number | null; h6: number | null; h24: number | null } // distinct traders
  supply: { circulating: number | null; total: number | null }
  audit: { mintAuthorityDisabled?: boolean; freezeAuthorityDisabled?: boolean; topHoldersPercentage?: number } | null
  links: { website: string | null; twitter: string | null }
}
```

**Market-cap caveat.** `mcap`, `fdv` and `supply` describe the token _on
Solana_. For bridged or wrapped assets (wrapped HYPE, PEPE, AVAX, the xStocks)
this is the Solana-side supply, which can be a tiny fraction of the asset's
real market cap. A "small cap" filter will therefore admit wrapped majors
unless you exclude them — for example by dropping `tags.includes('xstocks')`,
symbols that start with `w` followed by a capital, or by requiring
`organicScore` and `holders` thresholds that wrapped assets rarely meet on
Solana.

### 4.1 `ctx.rpc` methods known to be useful

Any read-only Solana RPC method is permitted. Commonly used:

- `getTokenAccountsByOwner(owner, { programId }, { encoding: 'jsonParsed' })`
  – every SPL token account of a wallet, decoded. Call once per token program
  (`ctx.programs.TOKEN` and `ctx.programs.TOKEN_2022`) if Token-2022 assets matter.
- `getTokenLargestAccounts(mint)` – top 20 holders of a mint.
- `getTokenSupply(mint)`
- `getAccountInfo(pubkey, { encoding: 'jsonParsed' })`
- `getMultipleAccounts(pubkeys, { encoding: 'jsonParsed' })` – up to 100.
- `getSignaturesForAddress(address, { limit })` and `getTransaction(sig, { maxSupportedTransactionVersion: 0 })`
  – activity history. Expensive; mind the 40-call budget.
- `getProgramAccounts` is **not** available on the test endpoint.

Values in `jsonParsed` token accounts live at
`account.data.parsed.info.{mint, owner, tokenAmount.{amount, decimals, uiAmount}}`.

## 5. Output contracts

- `select` **must** return an array of base58 mint addresses (strings). Order
  is irrelevant. Returning an empty array fails the run.
- `weight` **must** return a plain object `{ [mint]: number }`. Any positive
  scale is acceptable (dollar values, percentages, scores). Non-positive or
  missing entries remove the mint. Non-numeric values fail the run.
- `hold` **must** return a boolean synchronously. Return `false` to always
  follow `select` exactly.

## 6. Patterns

**Mirror a wallet by value**

```js
async function select(ctx) {
  const owner = '<WALLET>'
  const { value } = await ctx.rpc('getTokenAccountsByOwner', [owner, { programId: ctx.programs.TOKEN }, { encoding: 'jsonParsed' }])
  ctx.memo.amounts = {}
  for (const a of value) {
    const info = a.account.data.parsed.info
    if (info.tokenAmount.uiAmount > 0) ctx.memo.amounts[info.mint] = info.tokenAmount.uiAmount
  }
  return Object.keys(ctx.memo.amounts)
}

async function weight(ctx, mints) {
  const prices = await ctx.data.prices(mints)
  const out = {}
  for (const m of mints) {
    const usd = (ctx.memo.amounts[m] ?? 0) * (prices[m] ?? 0)
    if (usd >= 1) out[m] = usd
  }
  return out
}

function hold(ctx, mint) {
  return ctx.basket.heldFor(mint) < 6 * 3600
}
```

**Top-N by volume under a market-cap ceiling** (discovery via `ctx.data.top`)

```js
async function select(ctx) {
  const rows = await ctx.data.top('traded', { interval: '24h', limit: 100 })
  const picks = rows
    .filter((t) => t.mcap != null && t.mcap > 0 && t.mcap < 100_000_000)
    .filter((t) => !t.tags.includes('xstocks'))
    .filter((t) => (t.liquidity ?? 0) >= 250_000)
    .sort((a, b) => (b.volume.h24 ?? 0) - (a.volume.h24 ?? 0))
    .slice(0, 10)
  ctx.memo.volume = Object.fromEntries(picks.map((t) => [t.mint, t.volume.h24 ?? 0]))
  return picks.map((t) => t.mint)
}
async function weight(ctx, mints) {
  return Object.fromEntries(mints.map((m) => [m, ctx.memo.volume[m] ?? 0]))
}
function hold(ctx, mint) {
  return ctx.basket.heldFor(mint) < 24 * 3600
}
```

**Equal-weight a fixed list, filtered by market cap**

```js
async function select(ctx) {
  const candidates = ['<MINT_A>', '<MINT_B>', '<MINT_C>']
  const tokens = await ctx.data.tokens(candidates)
  return tokens.filter((t) => t && (t.mcap ?? 0) >= 50_000_000).map((t) => t.mint)
}
async function weight(ctx, mints) {
  return Object.fromEntries(mints.map((m) => [m, 1]))
}
function hold() {
  return false
}
```

**Union of several wallets, each counted equally**

```js
async function select(ctx) {
  const wallets = ['<W1>', '<W2>', '<W3>']
  ctx.memo.share = {}
  for (const w of wallets) {
    const { value } = await ctx.rpc('getTokenAccountsByOwner', [w, { programId: ctx.programs.TOKEN }, { encoding: 'jsonParsed' }])
    const rows = value.map((a) => a.account.data.parsed.info).filter((i) => i.tokenAmount.uiAmount > 0)
    const prices = await ctx.data.prices(rows.map((r) => r.mint))
    const usd = rows.map((r) => [r.mint, r.tokenAmount.uiAmount * (prices[r.mint] ?? 0)])
    const total = usd.reduce((s, [, v]) => s + v, 0) || 1
    for (const [m, v] of usd) ctx.memo.share[m] = (ctx.memo.share[m] ?? 0) + v / total / wallets.length
  }
  return Object.keys(ctx.memo.share)
}
async function weight(ctx, mints) {
  return Object.fromEntries(mints.map((m) => [m, ctx.memo.share[m]]))
}
function hold(ctx, mint) {
  return ctx.basket.heldFor(mint) < 24 * 3600
}
```

## 7. Common mistakes

- Calling `ctx.data.prices` once per mint inside a loop. Batch: one call with
  all mints.
- Expecting `ctx.data.top` to filter for you. It returns the upstream top 100;
  every threshold (market cap, liquidity, age, tags) is applied by your code.
- Treating `mcap` as the asset's global market cap for wrapped tokens (see
  the caveat in section 4).
- Forgetting Token-2022 assets (xStocks, some majors). Query both programs if
  the idea depends on them.
- Returning token objects from `select` instead of mint strings.
- Returning weights as strings (`"12.5"`). Return numbers.
- Using `console.log`. Use `ctx.log`.
- Assuming `ctx.basket.current` is populated during a test run. It is empty.

## 8. Required output format

Respond with **exactly three fenced code blocks**, in this order, each
containing one complete function declaration (helpers allowed inside the same
block), with no surrounding prose inside the blocks:

```js
// select.js
async function select(ctx) { ... }
```

```js
// weight.js
async function weight(ctx, mints) { ... }
```

```js
// hold.js
function hold(ctx, mint) { ... }
```

Before the blocks, state in two sentences what the strategy does and what
data it depends on. After the blocks, list any placeholders the user must fill
in (wallet addresses, mints, thresholds). Do not include anything else.
