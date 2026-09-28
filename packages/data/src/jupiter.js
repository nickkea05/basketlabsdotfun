// Jupiter token + holdings API client. Calls go through the app's proxy route
// (see config.js); upstream is lite-api.jup.ag.

import { baseUrl } from './config.js'

async function get(path) {
  const res = await fetch(`${baseUrl('jupiter')}${path}`, { headers: { accept: 'application/json' } })
  if (!res.ok) throw new Error(`Jupiter ${res.status} on ${path}`)
  return res.json()
}

/** Normalise a Jupiter token record into the shape the app uses everywhere. */
export function normalizeToken(t) {
  return {
    mint: t.id,
    name: t.name ?? '',
    symbol: t.symbol ?? '',
    icon: t.icon ?? null,
    decimals: t.decimals ?? 0,
    tokenProgram: t.tokenProgram,
    price: t.usdPrice ?? null,
    mcap: t.mcap ?? null,
    liquidity: t.liquidity ?? null,
    holders: t.holderCount ?? null,
    tags: t.tags ?? [],
    verified: !!t.isVerified || (t.tags ?? []).includes('verified'),
    // Detail fields for the info panel.
    fdv: t.fdv ?? null,
    circSupply: t.circSupply ?? null,
    totalSupply: t.totalSupply ?? null,
    organicScore: t.organicScore ?? null,
    createdAt: t.firstPool?.createdAt ?? null,
    audit: t.audit ?? null, // { mintAuthorityDisabled, freezeAuthorityDisabled, topHoldersPercentage, ... }
    website: t.website ?? null,
    twitter: t.twitter ?? null,
    stats: {
      m5: t.stats5m ?? null,
      h1: t.stats1h ?? null,
      h6: t.stats6h ?? null,
      h24: t.stats24h ?? null,
    },
  }
}

/** Free-text or mint search. Jupiter matches on symbol, name and mint. */
export async function searchTokens(query, limit = 60) {
  const q = query.trim()
  if (!q) return []
  const data = await get(`/tokens/v2/search?query=${encodeURIComponent(q)}&limit=${limit}`)
  return data.map(normalizeToken)
}

/** Batch lookup by mint (up to 100 per call). */
export async function tokensByMints(mints) {
  const out = []
  for (let i = 0; i < mints.length; i += 100) {
    const chunk = mints.slice(i, i + 100)
    const data = await get(`/tokens/v2/search?query=${chunk.join(',')}`)
    out.push(...data.map(normalizeToken))
  }
  return out
}

/** Most-traded tokens over 24h. Used as the default browse list. */
export async function topTokens(limit = 100) {
  const data = await get(`/tokens/v2/toporganicscore/24h?limit=${limit}`)
  return data.map(normalizeToken)
}

export const TOP_LISTS = Object.freeze({ traded: 'toptraded', trending: 'toptrending', organic: 'toporganicscore' })
export const TOP_INTERVALS = Object.freeze(['5m', '1h', '6h', '24h'])
export const TOP_LIMIT = 100

/**
 * Jupiter's ranked lists: 'traded' (by volume), 'trending', 'organic' (by
 * organic score) over an interval, or 'recent' (newest pools, no interval).
 * At most 100 rows per call; that is Jupiter's ceiling.
 */
export async function topList(list, { interval = '24h', limit = TOP_LIMIT } = {}) {
  const n = Math.min(Math.max(1, limit | 0), TOP_LIMIT)
  if (list === 'recent') return (await get(`/tokens/v2/recent?limit=${n}`)).map(normalizeToken)
  const path = TOP_LISTS[list]
  if (!path) throw new Error(`unknown list "${list}"; use ${Object.keys(TOP_LISTS).join(', ')} or recent`)
  if (!TOP_INTERVALS.includes(interval)) throw new Error(`unknown interval "${interval}"; use ${TOP_INTERVALS.join(', ')}`)
  return (await get(`/tokens/v2/${path}/${interval}?limit=${n}`)).map(normalizeToken)
}

let xstocksCache = null
/** Every xStock Jupiter knows about. Cached for the session. */
export async function xstocks() {
  if (xstocksCache) return xstocksCache
  const data = await get('/tokens/v2/search?query=xStock&limit=100')
  xstocksCache = data.map(normalizeToken).filter((t) => t.tags.includes('xstocks'))
  return xstocksCache
}

export const SOL_MINT = 'So11111111111111111111111111111111111111112'

/**
 * A wallet's spot holdings valued in USD, as [{ token, amount, valueUsd }],
 * sorted by value. Dust and non-net-worth accounts are dropped.
 */
export async function walletHoldings(address, { minValueUsd = 1, maxMints = 300 } = {}) {
  const h = await get(`/ultra/v1/holdings/${address}`)
  const amounts = new Map()
  if (h.uiAmount > 0) amounts.set(SOL_MINT, h.uiAmount)
  for (const [mint, accounts] of Object.entries(h.tokens ?? {})) {
    let sum = 0
    for (const a of accounts) if (!a.excludeFromNetWorth && a.uiAmount > 0) sum += a.uiAmount
    if (sum > 0) amounts.set(mint, sum)
  }
  const mints = [...amounts.keys()]
  const truncated = mints.length > maxMints
  const tokens = await tokensByMints(mints.slice(0, maxMints))
  const rows = tokens
    .map((token) => ({ token, amount: amounts.get(token.mint), valueUsd: (token.price ?? 0) * amounts.get(token.mint) }))
    .filter((r) => r.valueUsd >= minValueUsd)
    .sort((a, b) => b.valueUsd - a.valueUsd)
  const total = rows.reduce((s, r) => s + r.valueUsd, 0)
  return { rows, totalUsd: total, truncated, tokenAccounts: mints.length }
}
