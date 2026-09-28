import { useMemo, useState } from 'react'
import { BPS_TOTAL, BasketType, mirrorWeights, normalizeWeights, sumBps } from '@basketfun/core'
import { Logo, Mint } from '../../../components/ui/index.js'
import { fmtPct } from '../../../lib/format.js'

export default function AllocationStep({ draft, dispatch, attempted }) {
  const mirror = draft.basketType === BasketType.Mirror
  const weights = mirror ? mirrorWeights(draft) : draft.weights
  const many = mirror && draft.hosts.length > 1
  const total = sumBps(weights)
  const over = total > BPS_TOTAL
  const exact = total === BPS_TOTAL

  // Re-sort on blur, not on every keystroke, so rows don't jump under the cursor.
  const [orderKey, setOrderKey] = useState(0)
  const ordered = useMemo(
    () => [...draft.assets].sort((a, b) => (weights[b.mint] ?? 0) - (weights[a.mint] ?? 0)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [draft.assets, orderKey, mirror]
  )

  const setPct = (mint, str) => {
    const v = parseFloat(str)
    dispatch({ type: 'weight', mint, bps: isNaN(v) ? 0 : Math.max(0, Math.round(v * 100)) })
  }

  const auto = () => {
    const base = Object.fromEntries(draft.assets.map((a) => [a.mint, draft.weights[a.mint] ?? 0]))
    dispatch({ type: 'weights', value: normalizeWeights(base) })
    setOrderKey((k) => k + 1)
  }

  return (
    <div className="step step-alloc">
      <div className="step-title">
        <h2>{mirror ? (many ? 'Weights, set by the hosts' : 'Weights, set by the host') : 'Set the weights'}</h2>
        <p>
          {mirror
            ? many
              ? `The combined book of ${draft.hosts.length} wallets, ${draft.hostWeighting === 0 ? 'each counted equally' : 'pooled by value'}. The basket keeps tracking them after launch; nothing here is editable.`
              : 'These are the host wallet\u2019s current shares by value. The basket will keep tracking them after launch; nothing here is editable.'
            : 'Percent of the basket each asset represents. Must total exactly 100%.'}
        </p>
        {!mirror && (
          <button className="btn btn-ghost raised auto-btn" onClick={auto} title="Scale to 100% keeping the same proportions">
            Auto-balance
          </button>
        )}
      </div>

      <div className="alloc-list">
        {ordered.map((a) => {
          const bps = weights[a.mint] ?? 0
          const pct = bps ? (bps / 100).toString() : ''
          return (
            <div key={a.mint} className={`alloc-row ${mirror ? 'ro' : ''}`}>
              <div className="alloc-bar" style={{ width: `${Math.min(100, bps / 100)}%` }} />
              <Logo token={a} size={32} />
              <div className="alloc-id">
                <div className="alloc-sym">
                  {a.symbol} <span className="muted alloc-name">{a.name}</span>
                </div>
                <Mint mint={a.mint} />
              </div>
              {mirror ? (
                <div className="alloc-ro mono">{fmtPct(bps, 2)}</div>
              ) : (
                <label className="alloc-input input">
                  <input
                    type="number"
                    inputMode="decimal"
                    min="0"
                    step="0.1"
                    placeholder="0"
                    value={pct}
                    onChange={(e) => setPct(a.mint, e.target.value)}
                    onBlur={() => setOrderKey((k) => k + 1)}
                  />
                  <span>%</span>
                </label>
              )}
            </div>
          )
        })}
      </div>

      <div className={`meter-wrap raised ${over ? 'over' : exact ? 'exact' : ''}`}>
        <div className="meter">
          <div className="meter-fill" style={{ width: `${Math.min(100, total / 100)}%` }} />
        </div>
        <div className="meter-row">
          <span className="meter-ticker mono">{fmtPct(total, 1)}</span>
          <span className="meter-state">
            {over ? `Overflow +${fmtPct(total - BPS_TOTAL, 1)}` : exact ? 'Ready' : mirror ? '' : `${fmtPct(BPS_TOTAL - total, 1)} left`}
          </span>
          {attempted && !exact && <span className="meter-err">Must total 100%</span>}
        </div>
      </div>
    </div>
  )
}
