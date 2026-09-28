// Number and date formatting used across the app.

/** USD, compact: $1.23B, $4.5M, $12.3K, $0.42 */
export const fmtCompact = (n) => {
  if (n == null || !isFinite(n)) return '—'
  if (n >= 1e9) return `$${(n / 1e9).toFixed(2)}B`
  if (n >= 1e6) return `$${(n / 1e6).toFixed(2)}M`
  if (n >= 1e3) return `$${(n / 1e3).toFixed(1)}K`
  return `$${n.toFixed(2)}`
}

/** Token price with precision that scales down for small caps. */
export const fmtTokenPrice = (n) => {
  if (n == null || !isFinite(n)) return '—'
  if (n >= 1000) return `$${n.toLocaleString(undefined, { maximumFractionDigits: 2 })}`
  if (n >= 1) return `$${n.toFixed(2)}`
  if (n >= 0.01) return `$${n.toFixed(4)}`
  if (n >= 0.0001) return `$${n.toFixed(6)}`
  return `$${n.toExponential(2)}`
}

/** Basis points -> percent string. */
export const fmtPct = (bps, dp = 1) => `${(bps / 100).toFixed(dp)}%`

/** Plain number, compact: 3.32B, 12.4M, 840K. */
export const fmtNum = (n) => {
  if (n == null || !isFinite(n)) return '—'
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)}B`
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)}M`
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`
  return n.toLocaleString(undefined, { maximumFractionDigits: 2 })
}

/** Relative age from an ISO timestamp: 1.2y, 14d, 6h, 3m. */
export const fmtAge = (iso) => {
  if (!iso) return '—'
  const ms = Date.now() - new Date(iso).getTime()
  const d = Math.floor(ms / 86_400_000)
  if (d >= 365) return `${(d / 365).toFixed(1)}y`
  if (d >= 1) return `${d}d`
  const h = Math.floor(ms / 3_600_000)
  return h >= 1 ? `${h}h` : `${Math.max(1, Math.floor(ms / 60_000))}m`
}

/** Basket market cap style: $4.12M / $612.0K / $0.0412 */
export const fmtUsd = (n) => (n >= 1_000_000 ? `$${(n / 1_000_000).toFixed(2)}M` : n >= 1_000 ? `$${(n / 1_000).toFixed(1)}K` : `$${n.toFixed(n < 1 ? 4 : 2)}`)

export const fmtPrice = (n) => (n >= 1 ? `$${n.toFixed(4)}` : n >= 0.001 ? `$${n.toFixed(4)}` : `$${n.toFixed(6)}`)
