import { useEffect } from 'react'
import { BASKET_TYPES, INITIAL_SHARE_PRICE_USD } from '@basketfun/core'
import { Icon } from '../../components/ui/index.js'

// The whole explainer, in one screen. Stands in for a docs site on purpose:
// the product is meant to be legible without one.
export default function HowItWorks({ open, onClose, go }) {
  useEffect(() => {
    if (!open) return
    const onKey = (e) => e.key === 'Escape' && onClose()
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, onClose])

  if (!open) return null

  return (
    <div className="hw-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="hw raised" role="dialog" aria-modal="true" aria-label="How basketlabs.fun works">
        <div className="hw-head">
          <div className="hw-title">
            How <span className="muted">basketlabs.fun works</span>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="Close">
            <Icon.Close />
          </button>
        </div>
        <p className="hw-lede">One token, pinned to many. Buy the idea, not twelve separate coins.</p>

        <div className="hw-scroll">
          <div className="hw-block">
            <h4>A basket is one token backed by real assets</h4>
            <p>
              Every basket token is backed by the assets it names, held in a vault owned by the program, not by us or the creator. Its price tracks the value of
              what is inside. Every share opens at ${INITIAL_SHARE_PRICE_USD.toLocaleString()}, so lifetime return reads at a glance.
            </p>
          </div>

          <div className="hw-block">
            <h4>Four ways to run one</h4>
            <p>Ranked from least to most trust required in the creator.</p>
            <div className="hw-types">
              {BASKET_TYPES.map((t) => (
                <div key={t.id} className="hw-type">
                  <strong>
                    {t.name}
                    <span className="risk">{t.risk} risk</span>
                  </strong>
                  <p>{t.tagline}</p>
                </div>
              ))}
            </div>
          </div>

          <div className="hw-block">
            <h4>What a creator can and cannot do</h4>
            <p>
              Fixed and Automated baskets cannot be touched after launch by anyone, the code and contents are pinned on-chain. Mirror baskets follow their host
              wallets and ignore the creator. Managed baskets are the exception: the creator can rebalance at will, so buy them from people with a track record.
              The leaderboard exists for exactly that.
            </p>
          </div>

          <div className="hw-block">
            <h4>Your wallet signs everything</h4>
            <p>basketlabs.fun never holds your funds. Launching, buying, selling and redeeming are all transactions you approve in your own wallet.</p>
          </div>
        </div>

        <div className="hw-actions">
          <button className="btn btn-ghost" onClick={onClose}>
            Got it
          </button>
          <button
            className="btn btn-white"
            onClick={() => {
              onClose()
              go('create')
            }}
          >
            + Launch
          </button>
        </div>
      </div>
    </div>
  )
}
