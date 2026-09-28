// GeckoTerminal client: price history for the token info panel. Calls go
// through the app's proxy route (see config.js).

import { baseUrl } from './config.js'

// Free tier is ~30 req/min. One quiet retry on 429 before giving up.
async function get(path, retry = true) {
  const res = await fetch(`${baseUrl('geckoterminal')}${path}`, { headers: { accept: 'application/json' } })
  if (res.status === 429 && retry) {
    await new Promise((r) => setTimeout(r, 1800))
    return get(path, false)
  }
  if (res.status === 429) throw new Error('Rate limited — try again in a moment')
  if (!res.ok) throw new Error(`GeckoTerminal ${res.status}`)
  return res.json()
}

const poolCache = new Map()
/** Address of the deepest pool for a mint, or null if none is indexed. */
export async function topPool(mint) {
  if (poolCache.has(mint)) return poolCache.get(mint)
  const j = await get(`/networks/solana/tokens/${mint}/pools?page=1`)
  const pools = (j.data ?? []).slice().sort((a, b) => Number(b.attributes.reserve_in_usd ?? 0) - Number(a.attributes.reserve_in_usd ?? 0))
  const p = pools[0]
  const out = p ? { address: p.attributes.address, name: p.attributes.name, dex: p.relationships?.dex?.data?.id ?? null } : null
  poolCache.set(mint, out)
  return out
}

/** Chart ranges -> GeckoTerminal OHLCV params. */
export const RANGES = {
  '24H': { timeframe: 'minute', aggregate: 5, limit: 288 },
  '7D': { timeframe: 'hour', aggregate: 1, limit: 168 },
  '30D': { timeframe: 'hour', aggregate: 4, limit: 180 },
}

const seriesCache = new Map()
/** Close prices oldest -> newest as [{ t, close, volume }]. */
export async function priceSeries(pool, range) {
  const key = `${pool}:${range}`
  if (seriesCache.has(key)) return seriesCache.get(key)
  const { timeframe, aggregate, limit } = RANGES[range]
  const j = await get(`/networks/solana/pools/${pool}/ohlcv/${timeframe}?aggregate=${aggregate}&limit=${limit}&currency=usd`)
  const list = j.data?.attributes?.ohlcv_list ?? []
  const out = list.map(([t, , , , c, v]) => ({ t, close: c, volume: v })).sort((a, b) => a.t - b.t)
  seriesCache.set(key, out)
  return out
}
