// Search over the basket index: match, filter, rank, page. Pure functions so
// they can be tested without the UI and moved server-side when the indexer
// lands without changing the modal.

import { BASKET_TYPES } from './constants.js'

export const SEARCH_PAGE_SIZE = 24

export const SEARCH_SORTS = [
  { id: 'relevance', label: 'Relevance' },
  { id: 'mc', label: 'Market cap' },
  { id: 'vol', label: 'Volume' },
  { id: 'new', label: 'Newest' },
  { id: 'old', label: 'Oldest' },
]

export const SEARCH_MATCH = [
  { id: 'all', label: 'Everything' },
  { id: 'name', label: 'Name' },
  { id: 'ticker', label: 'Ticker' },
  { id: 'address', label: 'Address' },
  { id: 'assets', label: 'Contents' },
]

export const SEARCH_AGES = [
  { id: 'all', label: 'All', test: () => true },
  { id: '24h', label: '24h', test: (t) => t.ageH < 24 },
  { id: '7d', label: '7d', test: (t) => t.ageH < 168 },
  { id: '30d', label: '30d', test: (t) => t.ageH < 720 },
]

export const SEARCH_TYPES = [
  { id: 'all', label: 'All', test: () => true },
  ...BASKET_TYPES.map((b) => ({ id: b.key, label: b.name, test: (t) => t.type === b.id })),
]

export const SEARCH_STATUS = [
  { id: 'all', label: 'All', test: () => true },
  { id: 'curve', label: 'On curve', test: (t) => t.raised != null },
  { id: 'graduated', label: 'Graduated', test: (t) => t.raised == null },
]

export const SEARCH_DEFAULTS = Object.freeze({ sort: 'relevance', match: 'all', age: 'all', type: 'all', status: 'all' })

const BASE58 = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/
export const looksLikeAddress = (q) => BASE58.test(q.trim())

const norm = (s) => (s ?? '').toString().toLowerCase().trim()

// Relevance of one basket to a query, restricted to the chosen field. 0 = no
// match. Exact ticker > name prefix > ticker prefix > address > name contains
// > ticker contains > holds the asset.
export function scoreBasket(t, q, match = 'all') {
  if (!q) return 1
  const name = norm(t.name)
  const ticker = norm(t.ticker)
  const ca = norm(t.ca)
  let s = 0
  const want = (f) => match === 'all' || match === f

  if (want('ticker')) {
    if (ticker === q || ticker === q.replace(/^\$/, '')) s = Math.max(s, 100)
    else if (ticker.startsWith(q.replace(/^\$/, ''))) s = Math.max(s, 70)
    else if (ticker.includes(q)) s = Math.max(s, 40)
  }
  if (want('name')) {
    if (name === q) s = Math.max(s, 95)
    else if (name.startsWith(q)) s = Math.max(s, 80)
    else if (name.split(/\s+/).some((w) => w.startsWith(q))) s = Math.max(s, 65)
    else if (name.includes(q)) s = Math.max(s, 50)
  }
  if (want('address')) {
    // Mocks carry a truncated address; the real index compares full mints.
    if (ca === q) s = Math.max(s, 100)
    else if (q.length >= 4 && (ca.includes(q) || (ca.includes('…') && matchesEllipsis(ca, q)))) s = Math.max(s, 60)
  }
  if (want('assets')) {
    const syms = t.basket.map((b) => norm(b.symbol))
    if (syms.includes(q)) s = Math.max(s, 45)
    else if (syms.some((x) => x.startsWith(q))) s = Math.max(s, 30)
  }
  return s
}

// "7xKq…9bA2" matches a full address that starts with 7xKq and ends with 9bA2.
function matchesEllipsis(short, full) {
  const [head, tail] = short.split('…')
  return full.startsWith(head) && full.endsWith(tail)
}

const CMP = {
  mc: (a, b) => b.marketCap - a.marketCap,
  vol: (a, b) => b.volume24h - a.volume24h,
  new: (a, b) => a.ageH - b.ageH,
  old: (a, b) => b.ageH - a.ageH,
}

export function searchBaskets(items, query, f = SEARCH_DEFAULTS) {
  const q = norm(query)
  const age = SEARCH_AGES.find((x) => x.id === f.age).test
  const type = SEARCH_TYPES.find((x) => x.id === f.type).test
  const status = SEARCH_STATUS.find((x) => x.id === f.status).test

  const hits = []
  for (const t of items) {
    if (!age(t) || !type(t) || !status(t)) continue
    const s = scoreBasket(t, q, f.match)
    if (s > 0) hits.push({ t, s })
  }

  if (f.sort === 'relevance') {
    // Ties (and the empty query, where everything scores 1) fall back to size.
    hits.sort((a, b) => b.s - a.s || CMP.mc(a.t, b.t))
  } else {
    const cmp = CMP[f.sort]
    hits.sort((a, b) => cmp(a.t, b.t))
  }
  return hits.map((h) => h.t)
}

export function pageResults(results, n) {
  const pages = Math.max(1, Math.ceil(results.length / SEARCH_PAGE_SIZE))
  const p = Math.min(Math.max(1, n), pages)
  const from = (p - 1) * SEARCH_PAGE_SIZE
  return {
    items: results.slice(from, from + SEARCH_PAGE_SIZE),
    page: p,
    pages,
    from: results.length ? from + 1 : 0,
    to: Math.min(from + SEARCH_PAGE_SIZE, results.length),
    total: results.length,
  }
}
