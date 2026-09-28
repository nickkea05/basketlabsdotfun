import { BASKET_TYPES, INITIAL_SHARE_PRICE_USD, buildLaunchParams } from '@basketfun/core'
import { Logo, Mint } from '../../../components/ui/index.js'
import { fmtPct } from '../../../lib/format.js'

export default function ReviewStep({ draft, creator }) {
  const p = buildLaunchParams(draft, { creator })
  const type = BASKET_TYPES[p.basket_type]

  return (
    <div className="step step-review">
      <div className="step-title">
        <h2>Review</h2>
        <p>
          This is exactly what gets written on-chain. Weights are locked at launch
          {type.id === 2 ? ' but you can rebalance later' : type.id === 1 ? ' and then follow the host' : ' forever'}.
        </p>
      </div>

      <div className="review-grid">
        <div className="review-card raised">
          <div className="rv-art">{draft.meta.image ? <img src={draft.meta.image} alt="" /> : <div className="preview-blank" />}</div>
          <div className="rv-name">{p.name}</div>
          <div className="muted">${p.symbol}</div>
          <dl className="rv-facts">
            <dt>Type</dt>
            <dd>
              {type.name} <span className="muted">· {type.risk.toLowerCase()} trust</span>
            </dd>
            <dt>Opens at</dt>
            <dd>${INITIAL_SHARE_PRICE_USD.toLocaleString()}.00 / share</dd>
            {p.host_wallets.length > 0 && (
              <>
                <dt>{p.host_wallets.length > 1 ? 'Hosts' : 'Host'}</dt>
                <dd className="rv-hosts">
                  {p.host_wallets.map((h) => (
                    <Mint key={h} mint={h} />
                  ))}
                  {p.host_wallets.length > 1 && <span className="muted">{p.host_weighting === 0 ? 'equal per wallet' : 'by value'}</span>}
                </dd>
              </>
            )}
            {p.strategy && (
              <>
                <dt>Code hash</dt>
                <dd className="mono" title={p.strategy.hash}>
                  {p.strategy.hash.slice(0, 10)}…{p.strategy.hash.slice(-8)}
                </dd>
              </>
            )}
            <dt>Assets</dt>
            <dd>{p.assets.length}</dd>
            <dt>Creator</dt>
            <dd>{creator ? <Mint mint={creator} /> : <span className="muted">Connect a wallet to sign</span>}</dd>
          </dl>
          {p.metadata.description && <p className="rv-desc">{p.metadata.description}</p>}
        </div>

        <div className="review-list panel">
          {p.assets.map((a) => {
            const t = draft.assets.find((x) => x.mint === a.mint)
            return (
              <div key={a.mint} className="rv-row">
                <div className="alloc-bar" style={{ width: `${a.weight_bps / 100}%` }} />
                <Logo token={t} size={26} />
                <span className="rv-sym">{t.symbol}</span>
                <span className="muted rv-tname">{t.name}</span>
                <Mint mint={a.mint} />
                <span className="mono rv-pct">{fmtPct(a.weight_bps, 2)}</span>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}
