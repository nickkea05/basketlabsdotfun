import { BasketType, BPS_TOTAL, HostWeighting, INITIAL_SHARE_PRICE_USD, maxAssetsFor } from './constants.js'

/**
 * Scale a set of weights so they sum to exactly BPS_TOTAL while keeping the
 * relative sizing. Rounding remainder goes to the largest position. Equal
 * weights if everything is zero.
 */
export function normalizeWeights(weights) {
  const mints = Object.keys(weights)
  if (mints.length === 0) return {}
  const total = mints.reduce((s, m) => s + (weights[m] || 0), 0)
  let out
  if (total <= 0) {
    const each = Math.floor(BPS_TOTAL / mints.length)
    out = Object.fromEntries(mints.map((m) => [m, each]))
  } else {
    out = Object.fromEntries(mints.map((m) => [m, Math.floor(((weights[m] || 0) * BPS_TOTAL) / total)]))
  }
  const sum = Object.values(out).reduce((s, v) => s + v, 0)
  const largest = mints.reduce((a, b) => (out[b] > out[a] ? b : a))
  out[largest] += BPS_TOTAL - sum
  return out
}

export const sumBps = (weights) => Object.values(weights).reduce((s, v) => s + (v || 0), 0)

/** Convert a host wallet snapshot (valueUsd per asset) into bps weights. */
export function weightsFromHoldings(rows) {
  const raw = Object.fromEntries(rows.map((r) => [r.token.mint, r.valueUsd]))
  return normalizeWeights(raw)
}

/**
 * Combine several host wallets into one target book.
 *   Equal: each wallet's own percentage book counts 1/N.
 *   Value: every wallet's dollars are pooled, then normalised.
 * Every pooled position is kept, ordered by combined weight. Returns
 * { assets: [token...], weights: { mint: bps } }.
 */
export function combineHosts(hosts, weighting = HostWeighting.Equal) {
  const score = new Map() // mint -> raw weight
  const tokens = new Map() // mint -> token
  for (const h of hosts) {
    const rows = h.snapshot?.rows ?? []
    const total = rows.reduce((s, r) => s + r.valueUsd, 0)
    for (const r of rows) {
      const share = weighting === HostWeighting.Equal ? (total > 0 ? r.valueUsd / total / hosts.length : 0) : r.valueUsd
      score.set(r.token.mint, (score.get(r.token.mint) ?? 0) + share)
      if (!tokens.has(r.token.mint)) tokens.set(r.token.mint, r.token)
    }
  }
  const ranked = [...score.entries()].sort((a, b) => b[1] - a[1])
  return {
    assets: ranked.map(([mint]) => tokens.get(mint)),
    weights: normalizeWeights(Object.fromEntries(ranked)),
  }
}

/** Current mirror weights for a draft. */
export const mirrorWeights = (draft) => combineHosts(draft.hosts, draft.hostWeighting).weights

/**
 * Build the exact argument object for the Anchor `create_basket` instruction
 * from the UI draft. Field names are snake_case to mirror the Rust struct.
 * This object is what gets serialised and sent; the review screen prints it
 * verbatim.
 */
export function buildLaunchParams(draft, { creator = null } = {}) {
  const mirror = draft.basketType === BasketType.Mirror
  const weights = mirror ? mirrorWeights(draft) : draft.weights

  const assets = draft.assets
    .map((a) => ({
      mint: a.mint,
      weight_bps: weights[a.mint] ?? 0,
      token_program: a.tokenProgram,
      decimals: a.decimals,
    }))
    .filter((a) => a.weight_bps > 0)
    .sort((a, b) => b.weight_bps - a.weight_bps)

  return {
    basket_type: draft.basketType, // BasketType enum, u8
    name: draft.meta.name.trim(),
    symbol: draft.meta.symbol.trim().toUpperCase(),
    // Off-chain metadata (description, image, socials) is pinned and referenced
    // by URI, like every other Solana token. Empty until the upload step exists.
    uri: '',
    metadata: {
      description: draft.meta.description.trim(),
      image: draft.meta.image ? '<data-url omitted>' : null,
      twitter: draft.meta.twitter.trim() || null,
      telegram: draft.meta.telegram.trim() || null,
      website: draft.meta.website.trim() || null,
    },
    assets,
    // Mirror only. Empty otherwise so the program can assert on it. One entry
    // copies a trader; several make an index (each host is an equal vote
    // unless host_weighting says to pool by value).
    host_wallets: mirror ? draft.hosts.map((h) => h.address) : [],
    host_weighting: mirror ? draft.hostWeighting : null, // HostWeighting enum, u8
    // Strategy only. The hash is what the program stores; the source is
    // published alongside so anyone can verify hash(source) and replay runs.
    strategy:
      draft.basketType === BasketType.Strategy
        ? {
            hash: draft.strategyHash ?? null,
            source: { select: draft.strategy.select, weight: draft.strategy.weight, hold: draft.strategy.hold },
          }
        : null,
    // USD price each share opens at, in micro-dollars (6 dp) to match USDC.
    initial_share_price_usd_micro: INITIAL_SHARE_PRICE_USD * 1_000_000,
    // The connected wallet. Signs the transaction and receives creator fees.
    creator,
  }
}

/** Sanity checks the program will also enforce. Returns a list of problems. */
export function validateParams(p) {
  const errs = []
  if (!p.name) errs.push('name is empty')
  if (!p.symbol) errs.push('symbol is empty')
  if (p.assets.length === 0) errs.push('no assets')
  const cap = maxAssetsFor(p.basket_type)
  if (p.assets.length > cap) errs.push(`${p.assets.length} assets, fixed baskets allow at most ${cap}`)
  const total = p.assets.reduce((s, a) => s + a.weight_bps, 0)
  if (total !== BPS_TOTAL) errs.push(`weights sum to ${total}, expected ${BPS_TOTAL}`)
  if (p.basket_type === BasketType.Mirror && p.host_wallets.length === 0) errs.push('mirror basket has no host_wallets')
  if (p.basket_type === BasketType.Strategy && !p.strategy?.hash) errs.push('strategy basket has no strategy hash')
  return errs
}

/**
 * Submit to the program. There is no program yet, so this performs a dry run:
 * it validates, waits as a real transaction would, and reports back with the
 * payload it would have sent. Swap the body for the Anchor client call when
 * the program is deployed.
 */
export async function submitLaunch(params) {
  const errors = validateParams(params)
  await new Promise((r) => setTimeout(r, 900))
  if (errors.length) return { ok: false, status: 'REJECTED', errors, params }
  return {
    ok: false,
    status: 'DRY_RUN',
    reason: 'PROGRAM_NOT_DEPLOYED',
    message: 'Payload built and validated. No on-chain program to send it to yet.',
    params,
  }
}
