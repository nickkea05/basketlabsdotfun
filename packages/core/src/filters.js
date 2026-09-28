// Comprehensive basket filters for the Live panel: categorical toggles,
// include / exclude keywords and typed min / max ranges. Pure; the indexer can
// run the same schema server-side later.

import { BASKET_TYPES } from './constants.js'

export const RANGE_FIELDS = [
  { id: 'marketCap', label: 'Market cap', unit: '$', get: (t) => t.marketCap },
  { id: 'volume24h', label: '24h volume', unit: '$', get: (t) => t.volume24h },
  { id: 'change24h', label: '24h change', unit: '%', get: (t) => t.change24h },
  { id: 'ageH', label: 'Age', unit: 'h', get: (t) => t.ageH },
  { id: 'assets', label: 'Assets', unit: '', get: (t) => t.basket.length },
]

export const FILTER_STATUS = [
  { id: 'all', label: 'All', test: () => true },
  { id: 'curve', label: 'On curve', test: (t) => t.raised != null },
  { id: 'graduated', label: 'Graduated', test: (t) => t.raised == null },
]

export const FILTER_TYPES = [
  { id: 'all', label: 'All', test: () => true },
  ...BASKET_TYPES.map((b) => ({ id: b.key, label: b.name, test: (t) => t.type === b.id })),
]

export const EMPTY_FILTERS = Object.freeze({ status: 'all', type: 'all', include: '', exclude: '', ranges: Object.freeze({}) })

const SUFFIX = { k: 1e3, m: 1e6, b: 1e9 }

/** "100k" -> 100000, "$1,250,000" -> 1250000, "-5 %" -> -5. Null if blank or unreadable. */
export function parseAmount(s) {
  if (s == null) return null
  const cleaned = String(s)
    .toLowerCase()
    .replace(/[$,%\s]/g, '')
  if (!cleaned) return null
  const m = /^(-?(?:\d+\.?\d*|\.\d+))([kmb])?$/.exec(cleaned)
  if (!m) return null
  const n = parseFloat(m[1]) * (m[2] ? SUFFIX[m[2]] : 1)
  return Number.isFinite(n) ? n : null
}

export const parseKeywords = (s) =>
  String(s ?? '')
    .split(',')
    .map((k) => k.trim().toLowerCase())
    .filter(Boolean)

const haystack = (t) => [t.name, t.ticker, ...t.basket.map((b) => b.symbol)].map((x) => String(x ?? '').toLowerCase())
const mentions = (t, words) => {
  const hay = haystack(t)
  return words.some((w) => hay.some((h) => h.includes(w)))
}

export function filterBaskets(items, f = EMPTY_FILTERS) {
  const status = (FILTER_STATUS.find((s) => s.id === f.status) ?? FILTER_STATUS[0]).test
  const type = (FILTER_TYPES.find((s) => s.id === f.type) ?? FILTER_TYPES[0]).test
  const inc = parseKeywords(f.include)
  const exc = parseKeywords(f.exclude)
  const bounds = RANGE_FIELDS.map((r) => {
    const v = f.ranges?.[r.id]
    return v ? { get: r.get, min: parseAmount(v.min), max: parseAmount(v.max) } : null
  }).filter((b) => b && (b.min != null || b.max != null))

  return items.filter((t) => {
    if (!status(t) || !type(t)) return false
    if (inc.length && !mentions(t, inc)) return false
    if (exc.length && mentions(t, exc)) return false
    for (const b of bounds) {
      const v = b.get(t)
      if (b.min != null && v < b.min) return false
      if (b.max != null && v > b.max) return false
    }
    return true
  })
}

const compact = (n) => {
  const a = Math.abs(n)
  const s = a >= 1e9 ? `${trim(a / 1e9)}B` : a >= 1e6 ? `${trim(a / 1e6)}M` : a >= 1e3 ? `${trim(a / 1e3)}K` : trim(a)
  return (n < 0 ? '-' : '') + s
}
const trim = (x) => String(+x.toFixed(2))

const withUnit = (field, n) => (field.unit === '$' ? `$${compact(n)}` : `${compact(n)}${field.unit}`)

export function fmtRange(field, { min, max } = {}) {
  const lo = parseAmount(min)
  const hi = parseAmount(max)
  if (lo != null && hi != null) return `${withUnit(field, lo)} – ${withUnit(field, hi)}`
  if (lo != null) return `≥ ${withUnit(field, lo)}`
  if (hi != null) return `≤ ${withUnit(field, hi)}`
  return ''
}

/** Chips for the applied filter set: [{ id, label, value }] in display order. */
export function activeFilters(f = EMPTY_FILTERS) {
  const out = []
  if (f.status !== 'all') out.push({ id: 'status', label: 'Status', value: FILTER_STATUS.find((s) => s.id === f.status)?.label ?? f.status })
  if (f.type !== 'all') out.push({ id: 'type', label: 'Type', value: FILTER_TYPES.find((s) => s.id === f.type)?.label ?? f.type })
  if (parseKeywords(f.include).length) out.push({ id: 'include', label: 'Include', value: parseKeywords(f.include).join(', ') })
  if (parseKeywords(f.exclude).length) out.push({ id: 'exclude', label: 'Exclude', value: parseKeywords(f.exclude).join(', ') })
  for (const r of RANGE_FIELDS) {
    const s = fmtRange(r, f.ranges?.[r.id])
    if (s) out.push({ id: r.id, label: r.label, value: s })
  }
  return out
}

/** Remove one chip from a filter set, returning the new set. */
export function clearFilter(f, id) {
  if (id === 'status' || id === 'type') return { ...f, [id]: 'all' }
  if (id === 'include' || id === 'exclude') return { ...f, [id]: '' }
  const ranges = { ...f.ranges }
  delete ranges[id]
  return { ...f, ranges }
}
