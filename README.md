# basketlabs.fun

A Solana launchpad for basket tokens. A basket is a share mint backed by a
program-owned vault of components plus a small SOL/share liquidity sleeve on
Meteora DLMM (two vault-owned positions: a keeper-recentered `tight` and a wide
`backstop`; migrating from DAMM v2 per `docs/change-order-liquidity-and-fees.md`).
Shares mint and redeem at NAV, pro rata against the vault.

Three basket types:

- **Fixed** — the book is set at launch and can never change. No admin path exists.
- **Mirror** — the book tracks one or more host wallets; a keeper submits rebalance
  books that the program validates against the launch rules.
- **Managed** — the creator rebalances under a timelock and turnover cap that buyers
  can read before they buy.

Status: pre-launch. The web app runs against mock data. The Anchor program has
every instruction of the original spec implemented with LiteSVM tests (85 green,
including a DLMM spike against the mainnet binary). The 2026-09-30 change order
(DLMM liquidity, deployer deposit, fee split, no holder rewards) is planned in
`docs/program-progress.md` §6 and not yet applied; the program has not been
deployed to devnet, reviewed, or audited. Nothing here should touch real money.

## Layout

```
apps/web            React web app (Vite)
packages/core       basket schema, weight math, launch payload, strategy contract
packages/data       market data clients (Jupiter, GeckoTerminal) and normalisers
packages/solana     wallet standard glue, Anchor client (IDL published from here)
programs/basket     the Anchor program + LiteSVM tests
idls/               third-party IDLs consumed via declare_program! (Meteora DLMM, cp-amm)
docs/               design docs, build spec, progress log, launch TODO
scripts/            fetch-fixtures.ps1 (mainnet program dumps for tests)
```

## Docs

- `docs/program-build-confirmation.md` — the program spec (decisions D1–D20).
- `docs/change-order-liquidity-and-fees.md` — amends the spec: DLMM two-position
  liquidity, deployer-signed create + 1 SOL deposit, 20/50/25/5 fee split, prize
  pool, holder rewards removed. Wins on conflict.
- `docs/program-progress.md` — build state, restart checklist, measured limits,
  open design questions.
- `docs/TODO.md` — everything that has to be true before launch.
- `programs/README.md` — toolchain matrix, Windows notes, program id handling.

## Getting started

Web (Node ≥ 24):

```
npm install
npm run dev          # apps/web
npm test             # packages/*/test
npm run lint
```

Program (Anchor 1.2.0, Solana CLI 4.3.0, Rust 1.96.1 — see `programs/README.md`):

```
.\scripts\fetch-fixtures.ps1     # once: dumps DLMM, cp-amm + token metadata .so files
anchor build
cargo test -p basket -- --test-threads=1
```

Tests run in-process on LiteSVM; no validator is needed. The program keypair in
`target/deploy/` is generated per machine and is never committed.

## Conventions

- Tests are written from the spec before the implementation. One test file per
  instruction under `programs/basket/tests/`.
- Anything that works in the demo but would fail with real traffic or real money
  gets a line in `docs/TODO.md` the moment it is noticed.
- Open design questions are asked, not decided; they are collected in
  `docs/program-progress.md` §5.

## License

Not yet chosen (tracked in `docs/TODO.md`). All rights reserved until then.
