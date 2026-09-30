<!-- Saved verbatim from Nick's message of 2026-09-30. Supersedes docs/program-build-confirmation.md where they conflict. -->

# Change order: liquidity, fees, and what they touch

Amends `PROGRAM_BUILD_CONFIRMATION.md`. Where this file conflicts with it, this file wins. It is deliberately short: it says what changes, why, and what else must be re-examined because of it. Read §0 before touching code, because the priorities there are what future design calls should be made against.

---

## 0. Priorities (in order, use these to break ties)

1. **Tradeable on Axiom / Fomo / DexScreener from launch**, with as little volume through our own site as possible. The new-pairs feeds are our acquisition channel; X is secondary.
2. **Good trading experience**: low slippage on 1–2 SOL buys, fills that feel like a graduated pump.fun token.
3. **Our running cost per basket is near zero.** Thousands of baskets will exist; anything non-refundable or per-basket-recurring must be covered by the fee share, never by treasury capital. Rule of thumb the keeper must satisfy: `team_fee_share × fees on basket > cost of managing its liquidity`.
4. **Deployer cost is low and refundable.** A 1 SOL refundable deposit is fine and is a feature (filters low-effort launches). Non-refundable deployer costs beyond a few dollars are not.
5. **The pool can never fail.** One basket whose pool stops quoting is a reputation event for the whole platform. Any design where price can leave the pool's range is disqualified.

Fees are not a priority. Traders already accept 3–5% total cost on Stonk-style tokens; a slightly higher or variable fee is invisible next to slippage on a volatile coin. Do not trade priorities 1, 2 or 5 for fee optimisation.

---

## 1. Why the liquidity design changed

What was in the confirmation doc: a Meteora DAMM v2 pool per basket with a fixed price range (`[0.5×, 8×]` etc.) chosen for capital efficiency.

Why it is wrong, so it does not get re-proposed:

- **Fixed ranges are death for a token whose value tracks assets.** The range is in price, not in distance from NAV. An index basket whose components 4× leaves the top of the range; a meme basket in a 60% drawdown leaves the bottom. In both cases DAMM v2 makes the pool one-sided and it stops quoting on Axiom (buys fail above range, sells fail below). DAMM v2 cannot widen an existing pool. Redeem still works, but the token looks broken on every venue. That violates priority 5 for exactly the assets we are built around.
- **A vanilla / full-range pool never fails but is far too thin.** Constant-product slippage ≈ `buy ÷ SOL side`. To match a graduated pump token (~85 SOL of SOL-side depth) at a 10% sleeve you need ~850 SOL of TVL, ~$120k, before the basket trades decently on Axiom. At the 25% front-loaded sleeve it is still ~340 SOL. That violates priority 1: baskets would sit invisible for weeks.
- **Concentrated liquidity that follows the price is the only design that satisfies both.** ~12 SOL of SOL-side depth concentrated around NAV trades like ~85 SOL full-range, and the pool itself is unbounded so it can never strand. The cost is a keeper bot that re-centers the position as NAV moves. We already run a keeper for Mirror/Strategy rebalances and rewards; this is one more loop in it, not a new dependency.

Venue decision: **Meteora DLMM** (bin-based CLMM). Raydium CLMM was the runner-up (same architecture, fixed fee tiers) and lost on engineering only: its Anchor-1.x CPI crate is a friction branch and its tick-array rent is never refunded. DLMM's CPI crate is current on Anchor 1.x, its rent is refundable when bin arrays are closed, and the existing DAMM v2 integration work carries over (same vendor, same position-NFT ownership pattern for the vault PDA). Dynamic fees are acceptable per priority note above.

---

## 2. What changes (DECIDED)

### 2.1 Pool
- **Meteora DLMM pool per basket**, quote = SOL, created **in the deployer's launch transaction**. Pool exists from block one so DexScreener/Axiom/Fomo index it immediately.
- **No fixed range anywhere.** Delete the range table in `PROGRAM_BUILD_CONFIRMATION.md §3`. Bin step and base-fee preset chosen per basket type (tighter for index/majors, wider for memes); document the presets as `Config` defaults.
- **Two vault-owned positions per basket:**
  - `tight`: ±10–15% around NAV, re-centered by the keeper. This is where fills happen.
  - `backstop`: wide (±60% or wider), placed once and left alone. Keeps the pool quoting through violent moves even if the keeper is late. Priority 5 lives here.
- The vault (basket PDA) owns both position NFTs. Sleeve deposits go to `tight`; a fixed slice (e.g. 20% of sleeve) goes to `backstop`.

### 2.2 Deployer economics
- **Deployer pays a 1 SOL refundable deposit at launch.** It covers DLMM pool + position rent (~0.25 SOL) and the keeper's bin-array float for that basket. Returned when the basket is closed (zero supply, idle) or when the basket graduates past a TVL threshold and the protocol assumes the float. Never charged to a buyer.
- **Lazy creation is removed.** Since the deployer now signs and pays at launch, `create_basket` is a normal instruction the deployer signs. Delete the ed25519-precompile / creator-signature verification path and the replay-binding logic. This is a simplification: less program surface, one fewer audit item.
- Deployer's own first buy (still **1 SOL minimum**) is the first sleeve deposit and seeds `tight`, so the pair appears on the feeds with real liquidity rather than getting dust-filtered.

### 2.3 Mint gate default
- **Default = `WindowUntil` then `Closed`.** Minting open for a creator-chosen window (24–72 h, bounded by `Config`), then closed for good. Redeem never gated. `Open` (perpetual minting) stays in the enum behind a flag for index/majors baskets; ship it disabled at launch. Reopen schedules stay as designed.
- Consequence: after the window, the only NAV growth is fees, sleeve SOL from pool buys, and the underlying. This is intended; it is what lets the token trade at a premium.

### 2.4 Keeper (spec additions)
- Loop per active basket: read pool price and NAV → if price outside `tight` by more than the threshold, withdraw `tight`, re-place around current price, close emptied bin arrays (reclaim rent). Do nothing for baskets with no trades since last check.
- Two keeper instances, alerting on lag, keeper wallet funded from the team fee share. Every action logged with `run_id` as before.
- Keeper never touches `backstop` except on basket close.

---

## 3. Ripple effects: re-examine each of these before assuming existing code still holds

| Area | Why it changes | What to check |
| --- | --- | --- |
| `create_basket` | Deployer signs and pays; no lazy create | Remove sysvar introspection, signature message, nonce. Add deposit escrow account and DLMM pool init CPI. Rent accounting for refund. |
| `seed` / `mint` | Sleeve goes into DLMM bins, not one DAMM position | Treasury-share minting at pool ratio must use the active bin price. Two-position deposit split. Bin selection helper. Compute budget re-measured (DLMM adds accounts). |
| `redeem` | Pro-rata withdrawal from **two** positions across bins | `remove_liquidity` CPI per position; returned SOL paid out, returned treasury shares burned. Two-tx path threshold likely lower than 12 assets now. |
| NAV formula | Pool SOL now lives in bins across two positions | `NAV = (components + SOL in both positions) / (supply − treasury shares in both positions)`. Indexer and tests must read both. |
| Fee claiming | DLMM fees accrue per position and include dynamic fees | `claim_pool_fees` claims both positions; route through the new split (§4). |
| Mint gate | Default flipped to Window→Closed | Frontend copy, `Config` defaults, tests that assumed Open. Ceiling-arb logic still correct when window is open. |
| `close_basket` | Must close both positions and bin arrays, refund deployer deposit | Ordering: withdraw liquidity → close positions → close bin arrays → refund. |
| Whitelist / bin presets | Bin step and fee preset per basket type | New `Config` fields; validation at create. |
| Events / indexer | Position IDs, bin ranges, re-center events, deposit/refund events | Add to event schema; leaderboard "listed" state now = pool SOL side ≥ threshold **in tight position**. |
| Tests | DAMM v2 LiteSVM fixtures | Replace with DLMM program fixtures; add out-of-range and keeper-late scenarios that prove `backstop` keeps the pool quoting. |
| Frontend brief §6 | "Lazy creation", "creator never needs SOL" | Now: creator pays 1 SOL refundable deposit + 1 SOL first buy. Update the Result step and the launch wizard's final screen. |

Anything else that read `pool` as a single DAMM v2 position should be assumed wrong until re-read.

---

## 4. Fee routing (DECIDED, numbers are defaults in `Config`)

Every fee event is split on-chain into four destinations. Same split for every basket; creators do not set it.

```
creator            20%   → creator's share, tiered by leaderboard rank at create
buyback            50%   → BUYBACK PDA: can only ever swap into $BSKT and burn; public from day one
team               25%   → TEAM wallet (Squads), fully at our discretion (ops, keeper, salaries, whatever)
deployer prizes     5%   → PRIZE PDA, paid out to top deployers on a schedule (§5)
```

Applies to: mint fee, redeem fee, and claimed pool swap fees (both DLMM positions, including dynamic fee).
Does not apply to: the deployer deposit (refundable, not a fee), the Managed management fee and performance fee (100% to the creator, as before).

The team share exists to be visible: it tells holders the team is paid from operations, not from selling the token. Do not hide it or route it through the buyback PDA.

**Holder rewards are removed.** The hold-time-weighted reward stream (`RewardsRoot`, `post_rewards_root`, `distribute_rewards`, hold-time indexing, reward-currency choice, root TTL) is deleted. It came from an earlier design where the token could not trade at a premium; the premium is now the holder's upside, and the reward stream was a small dribble with a large surface (per-wallet tracking through pool trades, Merkle roots, push distributions, ATA rent) and the most securities-shaped pattern in the design. Its share goes to the buyback. `FeeVault` shrinks to the creator line only.

---

## 5. Deployer prize pool (NEW, DECIDED)

- The 5% `PRIZE PDA` accrues continuously. Every epoch (default 14 days) the keeper posts a payout for the top deployers on the leaderboard for that epoch, ranked by **follower PnL** (same metric, same exclusions as the leaderboard).
- Default distribution: top 3 → 50 / 30 / 20 of the epoch's pool. Configurable to top 5 or 10 in `Config`.
- Paid in SOL by push to the deployer's wallet, logged as an event, shown on the leaderboard page with the countdown to the next payout and the current pool size.
- Purpose: make the leaderboard's incentive legible on day one. The flywheel (good baskets → rank → distribution) is real but only obvious in hindsight; a visible cheque every two weeks is not. Amounts will be small early; that is fine.
- Guardrails: minimum epoch volume before any payout; a deployer must have a live `CreatorLock` to be eligible; ties broken by TVL.

---

## 6. Things that did NOT change

Vault/mint/redeem core, in-kind pro-rata rule, no oracle on mint/redeem, classic SPL share mint, `ShareAuth` marker, book/rebalance model for Mirror/Managed/Strategy, HWM performance fee, frozen-component claims, admin/keeper authority model, timelocked treasury. Do not touch those to accommodate the pool change unless the ripple table above says so.

---

## Addendum (Nick, same message): keeper robustness

this is a major design change as we are completely changing which protocol we utilize to provide liqudity, aside from our protocol architecture, its incredibly important that the kepper bot behaves incredibly quickly, can resort levels often and can keep up even if a coins volatility skyrockets (either do to underlying nav or trading activity) in crypto a 20% gain in minutes is very possible, and thats the low end of whats possible, so the bot needs to be incredibly intentional in that our lp almost never fails (an absolute extreme case where say one of our tokens sends 5x in 30 seconds, fine thats unprecedented, but anything that can be considered somewhat possible in trenches should be the edge case plus padding for saftey) on axiom or fomo

be very careful with this

---

# Appendix: answers to program-progress.md §5, and sequencing (same message)

Read `CHANGE_ORDER_LIQUIDITY_AND_FEES.md` first. It changes the pool (DAMM v2 → Meteora DLMM, two positions, no fixed range), removes lazy creation (deployer pays a refundable 1 SOL deposit and signs `create_basket`), flips the mint-gate default to Window→Closed, sets the fee split, adds the deployer prize pool, and **removes holder rewards entirely**. Several of the questions below are answered or made moot by it.

Sequencing: apply the change order before Phase 8 (devnet). Treat its ripple table as a checklist; anything that touched the cp-amm position is presumed stale until re-read. Fold the answers below in at the same time.

---

## The two structural choices you flagged

**1. `open_position` (new instruction).** Keep it. Permissionless creation of a new mint's `Position`, with `finalize_rebalance` refusing until every target position exists, is the right shape. Requiring all positions at `submit_book` would cost accounts on large Mirror books for no safety gain.

**2. `close_basket` semantics.** The rule you landed on is right and carries over: close allowed when no holder shares exist outside the FeeVault (±10 base units for pool rounding) and the basket has been idle 14 days. But the mechanics are overtaken by the change order:

- Close now withdraws **both DLMM positions**, closes them, closes the bin arrays (reclaiming rent), and refunds the **deployer's 1 SOL deposit** plus all other rents to `basket.payer` (which is now the deployer).
- **Do not forfeit unswept creator fee lines to the treasury.** That takes money from a creator who did nothing wrong. `close_basket` must run the equivalent of `sweep_fees` first, so the creator's accrued share is paid out; only true dust (below the transfer minimum) is forfeited. With holder rewards removed, the FeeVault only holds the creator line, so this is a single payout.
- Fee shares held by the FeeVault are burned after sweep; sleeve SOL and vault dust go to the treasury as you built.

## The rest of §5, one line each

- **(w) Swap venue as a `Config` allow-list, Jupiter default:** yes.
- **(x) Weight-based turnover / per-position sell caps with 5% tolerance:** yes.
- **(y) Mint blocked during an open rebalance window, redeem never blocked:** yes. Redeem is never gated by anything; this is a hard rule.
- **(ac) Reward currency:** moot. Holder rewards are removed. Delete `RewardsRoot`, `post_rewards_root`, `distribute_rewards`, and the hold-time indexing.
- **(ad) 30-day rewards-root TTL:** moot, same reason.
- **(af) Creator lock range 1 day to 4 years:** yes. Leaderboard eligibility still requires ≥ 30 days and ≥ the `MIN_SKIN_BPS` self-position.
- **(ag–ai) Close semantics:** see above.

## Things to re-check because of the change order, beyond its ripple table

- Any test or fixture that minted "treasury shares to the sleeve" against a single cp-amm position.
- The `Basket` account layout: remove reward fields, add deposit escrow, two position NFT refs, bin preset id.
- `sweep_fees`: now three destinations from the FeeVault (creator, plus routing of the protocol/prize/buyback lines) and no rewards distributor.
- Frontend brief §6 and the launch wizard's final screen: deployer pays a refundable 1 SOL deposit and a 1 SOL minimum first buy; "creator never needs SOL" is no longer true.

Ask before deciding anything the change order and this file do not cover.
