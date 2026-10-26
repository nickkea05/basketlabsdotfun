// Basket market state and the rankings behind the homepage sections. Pure, so
// the indexer can compute the same lists server-side later.
//
// A basket's mint window opens on its first buy and stays open for
// MINT_WINDOW_H hours. While open, anyone can mint at NAV, so arbitrage keeps
// the pool price pinned to NAV. Once it closes, supply is fixed and the pool
// trades at whatever premium or discount demand sets; redeem stays open
// forever, which floors the discount.

export const MINT_WINDOW_H = 48

/** Hours left in the mint window, or 0 once it has closed. */
export const mintHoursLeft = (t) => Math.max(0, t.mintLeftH ?? 0)

export const isMintOpen = (t) => mintHoursLeft(t) > 0

/** Share of the window still remaining, 0..1. */
export const mintWindowLeft = (t) => Math.min(1, mintHoursLeft(t) / MINT_WINDOW_H)

/** Pool price relative to NAV, in percent. Positive is a premium. */
export const premiumPct = (t) => (t.nav ? ((t.price - t.nav) / t.nav) * 100 : 0)

/** "31h 12m", "38m", "Closed". */
export function fmtHoursLeft(h) {
  if (!(h > 0)) return 'Closed'
  const mins = Math.round(h * 60)
  if (mins < 60) return `${Math.max(1, mins)}m`
  const hh = Math.floor(mins / 60)
  const mm = mins % 60
  return mm ? `${hh}h ${mm}m` : `${hh}h`
}

const byAgeAsc = (a, b) => a.ageH - b.ageH

export const MINT_SORTS = [
  { id: 'new', label: 'Newest', cmp: byAgeAsc },
  { id: 'closing', label: 'Closing soon', cmp: (a, b) => mintHoursLeft(a) - mintHoursLeft(b) },
  { id: 'mc', label: 'Market cap', cmp: (a, b) => b.marketCap - a.marketCap },
]

/** Baskets still minting at NAV. */
export function mintOpenBaskets(items, sort = 'new') {
  const s = MINT_SORTS.find((x) => x.id === sort) ?? MINT_SORTS[0]
  return items.filter(isMintOpen).sort(s.cmp)
}

export const TRENDING_SORTS = [
  { id: 'vol1h', label: '1h volume', cmp: (a, b) => (b.volume1h ?? 0) - (a.volume1h ?? 0) },
  { id: 'txns', label: 'Trades', cmp: (a, b) => (b.txns1h ?? 0) - (a.txns1h ?? 0) },
  { id: 'chg', label: '24h change', cmp: (a, b) => b.change24h - a.change24h },
  { id: 'mc', label: 'Market cap', cmp: (a, b) => b.marketCap - a.marketCap },
]

export const TRENDING_SIZE = 10

/** What's moving right now. Anything with no trades in the last hour sits out. */
export function trendingBaskets(items, sort = 'vol1h', n = TRENDING_SIZE) {
  const s = TRENDING_SORTS.find((x) => x.id === sort) ?? TRENDING_SORTS[0]
  return items
    .filter((t) => (t.txns1h ?? 0) > 0)
    .sort(s.cmp)
    .slice(0, n)
}

// NAV return, not price: a basket ranks for what it holds, not for the premium
// people pay on top. Each window needs at least that much history so a
// two-hour-old basket can't top the 7d board.
export const PERFORMANCE_WINDOWS = [
  { id: '24h', label: '24h', minAgeH: 24, get: (t) => t.navChange24h },
  { id: '7d', label: '7d', minAgeH: 168, get: (t) => t.navChange7d },
  { id: 'all', label: 'All time', minAgeH: 72, get: (t) => t.navChangeAll },
]

/** Below this a basket is too small for its return to mean much. */
export const PERFORMANCE_MIN_MC = 100_000

export const PERFORMANCE_SIZE = 6

export function topPerformers(items, window = '7d', n = PERFORMANCE_SIZE) {
  const w = PERFORMANCE_WINDOWS.find((x) => x.id === window) ?? PERFORMANCE_WINDOWS[0]
  return items
    .filter((t) => t.ageH >= w.minAgeH && t.marketCap >= PERFORMANCE_MIN_MC && w.get(t) != null)
    .sort((a, b) => w.get(b) - w.get(a))
    .slice(0, n)
}
