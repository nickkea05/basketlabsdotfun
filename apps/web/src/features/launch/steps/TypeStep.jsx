import { BASKET_TYPES } from '@basketfun/core'
import { Icon } from '../../../components/ui/index.js'

const RISK = { Lowest: 1, Medium: 2, Highest: 3 }

export default function TypeStep({ draft, dispatch }) {
  return (
    <div className="step">
      <div className="step-title">
        <h2>What kind of basket?</h2>
        <p>Four mechanisms. The first three are ranked by how much control anyone has after launch; the fourth hands control to code.</p>
      </div>

      <div className="type-grid">
        {BASKET_TYPES.map((t) => {
          const on = draft.basketType === t.id
          return (
            <button key={t.id} className={`type-card raised ${on ? 'on' : ''}`} onClick={() => dispatch({ type: 'basketType', value: t.id })}>
              <div className="type-top">
                <span className="type-index mono">
                  0{t.id + 1}
                  {t.advanced && <span className="type-chip">Advanced</span>}
                </span>
                <span className={`tick ${on ? 'on' : ''}`}>{on && <Icon.Check />}</span>
              </div>
              <div className="type-name">{t.name}</div>
              <div className="type-tag">{t.tagline}</div>
              <p className="type-body">{t.body}</p>
              <div className="type-foot">
                <div className="type-meter">
                  <span className="risk">
                    {[1, 2, 3].map((i) => (
                      <i key={i} className={i <= RISK[t.risk] ? 'lit' : ''} />
                    ))}
                  </span>
                  <span className="muted">{t.risk} trust required</span>
                </div>
                <div className="type-meter">
                  <span className="risk effort">
                    {[1, 2, 3].map((i) => (
                      <i key={i} className={i <= t.effort ? 'lit' : ''} />
                    ))}
                  </span>
                  <span className={t.advanced ? 'type-effort-hi' : 'muted'}>{t.effortLabel}</span>
                </div>
              </div>
            </button>
          )
        })}
      </div>
    </div>
  )
}
