# Launch list

Everything that has to be true before basketlabs.fun takes real money. Items leave this
list only when they are 100% done and verified, not when they are "mostly working".

Working rules:

- Tests are written from the spec before the implementation, then the implementation
  is written without looking back at them. Tests cover code that touches or transforms
  real data (chain reads, API clients, weight math, payload building), not UI polish.
- Anything that works in the demo but would fail under real traffic, real wallets or
  real money gets flagged here the moment it is noticed, even if there is no fix yet.

---

## On-chain (Anchor)

- [ ] **Program: all spec instructions implemented and green on LiteSVM (phases
      1–7)**; next is devnet (phase 8) and the sign-off on the open design questions.
      Phase state, restart checklist, CU / account-limit measurements and the open
      questions are in `docs/program-progress.md`. Spec is
      `docs/program-build-confirmation.md`.
- [x] Workspace ready: Anchor 1.2.0, Solana CLI 4.3.0, platform-tools v1.57, host
      Rust 1.96.1, LiteSVM 0.16. `anchor build` and `anchor test` pass on the empty
      `basket` program. See `programs/README.md` for the version matrix and Windows
      notes.
- [ ] **Dev machine: McAfee holds every executed test binary open indefinitely**, so
      the relink after any program source change fails with `LNK1104` until the stale
      exe is moved aside. Fix is an on-access exclusion for `basketfun\target` (or
      building from WSL). Not a repo problem, but it will make `anchor test` look
      flaky until it is done.
- [ ] Program keypair: `target/deploy/basket-keypair.json` is gitignored and only
      exists on this machine. The real deploy key needs a home (hardware wallet or
      multisig upgrade authority) before anything touches devnet.
- [ ] Factory program: one deployment, `create_basket` instruction taking the exact
      schema `buildLaunchParams()` produces (`CreateBasketArgs`).
- [ ] Program-owned vault per basket. Token-2022 support (xStocks, some majors).
- [ ] Share mint with authority held by the basket PDA. Mint/redeem against NAV.
- [ ] Fixed baskets: no admin path exists after creation. Verifiable on-chain.
- [ ] Mirror baskets: rebalance instruction driven by keeper, weights from host
      snapshot, bounded slippage. Multi-host weighting (`host_weighting`) honoured.
- [ ] Managed baskets: creator-only rebalance, timelock and per-epoch turnover cap so
      buyers can see the rules before buying.
- [ ] Swap venue for rebalances (Jupiter CPI vs. direct AMM). Decide and document.
- [ ] $1000 initial share price: confirm the math with 6/9 decimal share mints.
- [ ] Fee model: creation fee, management fee, protocol cut. Where it accrues.
- [ ] Audit before mainnet. Budget and firm not chosen.
- [ ] Deploy to devnet; web app pointed at it end to end.

## Backend / API (Cloudflare Worker, `apps/api`)

- [ ] **Market-data proxy with cache + request coalescing.** Browser must never call
      GeckoTerminal or Jupiter directly. Server caches candles ~60s per token/range,
      dedupes concurrent identical requests. Without this the whole site shares one
      30 req/min GeckoTerminal budget and dies with ~5 concurrent users.
      (Hit this already in dev: rate limited while browsing charts.)
- [ ] API keys (Jupiter, CoinGecko/GeckoTerminal paid tier if needed) live server-side
      only. Never shipped to the browser.
- [ ] `/api/rpc`: keyed RPC provider (Helius/Triton) behind the same path. Dev uses
      the public mainnet endpoint, which 403s on browser Origin headers (proxy strips
      them) and rate-limits hard.
- [ ] Indexer: read baskets, share prices, NAV, holders from chain. Replaces
      `apps/web/src/mocks/baskets.js`. Explore page currently shows fake data.
      Trending sorts and the Live filter form (`filterBaskets` in core: status, type,
      include/exclude keywords, min/max on cap, volume, change, age, asset count)
      already run client-side against that shape (`type`, `ageH`, `marketCap`,
      `volume24h`, `change24h`, `raised`); the indexer needs to emit the same fields,
      and the same filter schema should become its query parameters so the list
      pages server-side once there are thousands of baskets.
- [ ] **Creator leaderboard data.** Per creator: baskets launched, per-basket max gain
      and current return vs the $1,000 open, holder PnL (indexed buys/sells per wallet
      vs NAV), share of baskets above open, turnover cost for Managed baskets. Then
      the score weights. UI is built with empty states; no calculation exists yet.
- [ ] Wallet holdings for very large wallets: Jupiter's endpoint 500s on wallets with
      hundreds of positions (seen with the Raydium authority). Need RPC fallback that
      reads token accounts directly and prices them.
- [ ] Host wallet snapshots for mirror baskets: who fetches them, how often, and what
      happens when a host goes to zero or holds unpriceable tokens.
- [ ] Metadata + logo upload (IPFS or R2). Currently only stored in the draft.

## Web (`apps/web`)

- [x] Wallet connect via Wallet Standard (Phantom first, any Solana wallet works).
      Connect modal, nav pill with menu, portfolio strip, silent reconnect, creator
      stamped on the launch payload.
- [ ] Lazy creation: nothing goes on-chain at launch. Creator signs the launch params
      (ed25519 message), we store them and list the basket as unfunded. The first buy
      transaction creates the accounts and pays rent (~0.01–0.03 SOL, bundled into a buy
      that is already much larger), with the program verifying the creator signature
      over the params so the buyer cannot alter the book. Dead launches then cost
      nothing and the creator never needs SOL. Unfunded baskets expire off the list
      after N days.
- [ ] Zero-supply close crank: keeper closes baskets with supply 0 and no activity for
      14 days, refunding rent to whoever paid it. Baskets with holders that go dead
      need a separate sunset path (freeze, liquidate vault to SOL, claim account);
      post-launch.
- [ ] Sniper note: NAV-priced mints have no first-buyer edge, so unlike pump.fun no
      bots will fund launches for us. Only relevant if we route through a curve.
- [ ] Portfolio holdings: wallet token accounts ∩ our share mints (from indexer). PnL
      from indexed buys/sells per wallet vs current NAV. Needs the program first.
- [ ] Launch submit actually sends the transaction; success/error screens driven by
      chain confirmation. Result step currently shows the payload only.
- [ ] Error boundaries per card/panel so one thrown error does not blank the page.
- [ ] Buy/sell on the token page wired to mint/redeem.
- [ ] Portfolio page: real holdings for the connected wallet.
- [ ] **Mirror tracking filters.** The base layer mirrors everything the hosts hold;
      these are the deployer's tools for making the basket profitable and tradeable.
      Not started. Each is a field on the launch payload the program enforces at
      rebalance: - min host position size (e.g. only positions the wallet holds >$500 of)
      - min market cap of the asset (avoid a host aping an $8k launch) - min hold time (asset must have been in the wallet > N hours before we follow) - min weight floor in the combined book (drop dust under X%) - max single-asset weight (cap concentration) - allow / deny list of mints
- [ ] Asset cap policy is decided: Fixed = 20, Mirror and Managed = uncapped at the
      product level. Still need to measure rebalance and redemption compute on devnet
      to know whether the program needs a hard ceiling (and what it is).
- [ ] Chain selector in asset filters is Solana-only for launch; hide or label it.
- [ ] Search (quick bar, Advanced modal, ⌘K) runs client-side over the mock list via
      `searchBaskets` in core. Point it at the indexer. A pasted mint that is not in
      the index should fall back to an on-chain lookup of the share-mint marker before
      saying "not a basket". Creator search (by wallet) once creators are indexed.
- [ ] Trending shows the top 10 by the chosen sort across every basket. Decide whether
      on-curve baskets should be eligible or whether Trending is graduated-only.
- [ ] Leaderboard rows: the table header exists, the row component does not. Build it
      when the first real data lands so it is shaped by real numbers, not guesses.
- [ ] Leaderboard 7d / 30d / All toggle is state only; nothing reads it yet.
- [ ] Draft persistence across reloads (localStorage) for "Keep draft".
- [ ] Mobile pass on the launch modal.

## Testing

- [ ] Fixture-based tests for `@basketfun/data` (recorded Jupiter/GeckoTerminal
      responses -> normalised shapes). No live network in the default test run.
- [ ] Separate live smoke suite, run manually, that hits the real APIs and asserts
      shapes have not drifted.
- [ ] `buildLaunchParams` / `validateParams` tests for every basket type, including
      rejection cases (weights not 10000, too few assets, missing hosts).
- [ ] Anchor tests for every instruction: LiteSVM (in-process, `anchor test`) for
      logic, plus a validator run before devnet for CPI paths. Two smoke tests exist
      (program loads into the SVM; constants match the JS schema).
- [x] Weight math (`normalizeWeights`, `combineHosts`) — 6 tests. Written after the
      code; treat as regression coverage, not spec coverage.

## Infra / deploy

- [ ] Cloudflare Pages for `apps/web`, Worker for `apps/api`. `wrangler.toml` in repo.
- [ ] Environment config: devnet vs mainnet program ids, RPC endpoints, API bases.
- [ ] CI: lint, test, build on every push. Block merge on failure.
- [ ] Custom domain, HTTPS, security headers (CSP in particular; we inline SVG and
      load token logos from arbitrary hosts).

## Trust / open source

- [ ] Pick a license.
- [ ] Root README: what the protocol is, how vaults are held, what the creator can and
      cannot do per basket type. This is the trust document.
- [ ] Publish program ids and verifiable build hashes once deployed.
- [ ] Risk disclosure in-app for managed baskets (creator can change contents).

## Strategy runtime (post-MVP, design decided)

Every basket's rebalance is "authorized party submits a book, program validates it."
Mirror is strategy #1 and uses this path from day one; user code later reuses it.

- [ ] Program stores `strategy_id` / `strategy_hash` per basket; keeper is the book
      authority for Mirror and Strategy types.
- [x] Strategy contract is three functions: `select(ctx) -> mints`,
      `weight(ctx, mints) -> {mint: share}`, `hold(ctx, mint) -> bool`. Defined in
      `packages/core/src/strategy.js` (lint, assemble, hash, book). Mirror-over-RPC is
      the worked example beside the editors.
- [x] Launch flow draft: `BasketType.Strategy` ("Automated"), Code step with example + three editors, Test step that runs the code in a Web Worker with network
      removed and `ctx` round-tripped to the host. `public/strategy-spec.md` is the
      AI-facing authoring spec with a "Copy spec for AI" button.
- [ ] **The browser worker is a stand-in.** Before Strategy baskets go live the test
      run and every scheduled run must execute in the server isolate below; the
      browser cannot enforce CPU limits and a user's own machine is not a trust
      boundary for the published hash.
- [ ] `ctx.params`: parameterised strategies (form fields the author declares, filled
      at launch) so templates can be forked without editing code.
- [ ] Data catalog (`ctx.data.*`): wallet holdings, token stats, price history,
      pump.fun bonding progress, fomo trending / most held / leaderboards (check fomo
      has a public API first). Every adapter runs server-side with our keys and cache.
- [ ] Runtime: one V8 isolate (Cloudflare Worker) or QuickJS-WASM per run. No
      `fetch`, no imports, no globals. Hard limits: CPU ms, wall time, data calls per
      run, bytes in, bytes out. Fail closed: a failed run keeps the previous book.
- [ ] Static rejection before execution (`import`, `eval`, `new Function`, `globalThis`).
- [ ] Testing step in the launch flow: dry run in the production sandbox, shows the
      resulting book, data calls made, CPU used vs budget.
- [ ] Dry runs require a connected wallet, are rate-limited per wallet and queued.
      Add a per-run fee if abused.
- [ ] Every scheduled run logs inputs and output publicly so anyone can replay the
      published code against the published inputs.
- [ ] "Auto-managed" type first: selector / weighting / hold-policy dropdowns backed by
      built-in strategies. Open the paste box only after the sandbox exists.
- [ ] SDK package, CLI (`npx basketfun run strategy.js`), `llms.txt` and MCP so an
      agent writes correct strategies without guessing.

## Product decisions still open

- [ ] Elastic supply vs. closed-end: LaunchLab/DBC revoke mint authority, which
      conflicts with mint/redeem at NAV. Custom program or accept closed-end?
- [ ] Multi-chain baskets (EVM vault + Solana share token). Post-launch.
- [ ] Which majors without a liquid Solana version to exclude at launch (HYPE etc.).
