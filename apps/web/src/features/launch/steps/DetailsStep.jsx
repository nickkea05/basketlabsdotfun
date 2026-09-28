import { useRef } from 'react'
import { DESCRIPTION_MAX, INITIAL_SHARE_PRICE_USD, NAME_MAX, SYMBOL_MAX, BasketType } from '@basketfun/core'
import { Icon, Logo } from '../../../components/ui/index.js'

export default function DetailsStep({ draft, dispatch }) {
  const m = draft.meta
  const set = (k) => (e) => dispatch({ type: 'meta', value: { [k]: e.target.value } })
  const fileRef = useRef(null)

  const onFile = (file) => {
    if (!file || !file.type.startsWith('image/')) return
    const r = new FileReader()
    r.onload = () => dispatch({ type: 'meta', value: { image: r.result } })
    r.readAsDataURL(file)
  }

  return (
    <div className="step step-details">
      <div className="step-title">
        <h2>Name it</h2>
        <p>What buyers see on the card. Everything here lives in the token metadata.</p>
      </div>

      <div className="details-grid">
        <div className="details-form">
          <div className="row-2">
            <div className="field">
              <label>Name</label>
              <div className="input">
                <input autoFocus maxLength={NAME_MAX} placeholder="Solana Blue" value={m.name} onChange={set('name')} />
                <span className="count-hint mono">
                  {m.name.length}/{NAME_MAX}
                </span>
              </div>
            </div>
            <div className="field">
              <label>Ticker</label>
              <div className="input">
                <span className="muted">$</span>
                <input
                  maxLength={SYMBOL_MAX}
                  placeholder="BLUE"
                  value={m.symbol}
                  onChange={(e) => dispatch({ type: 'meta', value: { symbol: e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, '') } })}
                />
              </div>
            </div>
          </div>

          <div className="field">
            <label>Description</label>
            <div className="input textarea">
              <textarea
                maxLength={DESCRIPTION_MAX}
                rows={3}
                placeholder="One or two lines on the thesis."
                value={m.description}
                onChange={set('description')}
              />
            </div>
          </div>

          <div className="row-3">
            <div className="field">
              <label>X / Twitter</label>
              <div className="input">
                <input placeholder="@handle" value={m.twitter} onChange={set('twitter')} />
              </div>
            </div>
            <div className="field">
              <label>Telegram</label>
              <div className="input">
                <input placeholder="t.me/…" value={m.telegram} onChange={set('telegram')} />
              </div>
            </div>
            <div className="field">
              <label>Website</label>
              <div className="input">
                <input placeholder="https://" value={m.website} onChange={set('website')} />
              </div>
            </div>
          </div>

          <div className="price-note raised">
            <div>
              <div className="pn-label">Opening share price</div>
              <div className="pn-val">${INITIAL_SHARE_PRICE_USD.toLocaleString()}.00</div>
            </div>
            <p className="muted">
              Every basket opens at the same round number, so lifetime return reads straight off the price. Supply is sized to match at launch.
            </p>
          </div>
        </div>

        <div className="details-preview">
          <div className="pv-label">Card preview</div>
          <div className="preview-card raised">
            <div
              className={`upload ${m.image ? 'has' : ''}`}
              onClick={() => fileRef.current?.click()}
              onDragOver={(e) => e.preventDefault()}
              onDrop={(e) => {
                e.preventDefault()
                onFile(e.dataTransfer.files?.[0])
              }}
              title="Upload a logo"
            >
              {m.image ? (
                <img src={m.image} alt="" />
              ) : (
                <>
                  <Icon.Image />
                  <span>Add logo</span>
                  <small className="muted">PNG or SVG, square</small>
                </>
              )}
              <input ref={fileRef} type="file" accept="image/*" hidden onChange={(e) => onFile(e.target.files?.[0])} />
            </div>
            <div className="card-name">{m.name || <span className="faint">Name</span>}</div>
            <div className="card-ticker">${m.symbol || '—'}</div>
            <div className="card-mc">
              ${INITIAL_SHARE_PRICE_USD.toLocaleString()} <small>{['Fixed', 'Mirror', 'Managed'][draft.basketType]}</small>
            </div>
            <div className="preview-logos">
              {draft.assets.slice(0, 8).map((a) => (
                <Logo key={a.mint} token={a} size={18} />
              ))}
              {draft.assets.length > 8 && <span className="muted">+{draft.assets.length - 8}</span>}
            </div>
            {draft.basketType === BasketType.Strategy && <div className="muted preview-note mono">automated · {draft.strategyHash?.slice(0, 10) ?? '—'}</div>}
            {draft.basketType === BasketType.Mirror && (
              <div className="fine">
                {draft.hosts.length > 1
                  ? `Mirrors ${draft.hosts.length} wallets`
                  : `Mirrors ${draft.hosts[0]?.address.slice(0, 4)}…${draft.hosts[0]?.address.slice(-4)}`}
              </div>
            )}
          </div>
          {m.image && (
            <button className="link" onClick={() => dispatch({ type: 'meta', value: { image: null } })}>
              Remove logo
            </button>
          )}
        </div>
      </div>
    </div>
  )
}
