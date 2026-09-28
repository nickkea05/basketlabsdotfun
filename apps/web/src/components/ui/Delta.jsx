/** Signed percent change, coloured up/down. */
export default function Delta({ v, dp = 2 }) {
  if (v == null || !isFinite(v)) return <span className="muted">—</span>
  return (
    <span className={v >= 0 ? 'up' : 'down'}>
      {v >= 0 ? '+' : ''}
      {v.toFixed(dp)}%
    </span>
  )
}
