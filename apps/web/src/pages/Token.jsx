import { useState } from 'react'
import Chart from '../components/charts/Chart.jsx'
import { Orb } from '../components/ui/index.js'
import { series, activity } from '../mocks/baskets.js'
import { fmtUsd, fmtPrice } from '../lib/format.js'
import Alive from '../components/effects/Alive.jsx'
import Footer from '../components/layout/Footer.jsx'

const Pct = ({ v }) => (
  <small className={v >= 0 ? 'up' : 'down'}>
    {v >= 0 ? '+' : ''}
    {v.toFixed(1)}%
  </small>
)

export default function Token({ t, back, go }) {
  const [side, setSide] = useState('buy')
  const [range, setRange] = useState('1D')
  const isLive = t.raised != null
  const data = series(t.id.length * 13 + range.length + (t.marketCap % 97))
  const premium = ((t.price - t.nav) / t.nav) * 100

  return (
    <div className="page container">
      <button className="back" onClick={back}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="m15 18-6-6 6-6" />
        </svg>
        Back
      </button>

      <section className={`panel ${isLive ? 'alive' : ''}`}>
        {isLive && <Alive />}
        <div className="head">
          <div className="head-left">
            <div className="head-art">
              <Orb tint={t.tint} small />
            </div>
            <div>
              <h1>
                {t.name}
                <span className={`badge ${isLive ? 'live' : ''}`}>
                  {isLive && <i className="pulse" />}
                  {isLive ? 'On curve' : 'Graduated'}
                </span>
              </h1>
              <div className="head-meta">
                <span>${t.ticker}</span>
                <span className="dot" />
                <span>
                  Backed by{' '}
                  <span className="chips" style={{ display: 'inline-flex', marginLeft: 4 }}>
                    {t.basket.map((b) => (
                      <span key={b.symbol} className="chip">
                        {b.symbol}
                      </span>
                    ))}
                  </span>
                </span>
                <span className="dot" />
                <button className="mono copy">{t.ca}</button>
              </div>
            </div>
          </div>
          <div className="nav-right">
            <button className="btn btn-ghost raised">Explorer</button>
            <button className="btn btn-ghost raised">{isLive ? 'Curve' : 'Pool'}</button>
          </div>
        </div>

        <div className="stats">
          <div className="stat">
            <div className="stat-label">Market cap</div>
            <div className="stat-value">{fmtUsd(t.marketCap)}</div>
          </div>
          <div className="stat">
            <div className="stat-label">Price</div>
            <div className="stat-value">
              {fmtPrice(t.price)}
              <Pct v={t.change24h} />
            </div>
          </div>
          <div className="stat">
            <div className="stat-label">Basket value / token</div>
            <div className="stat-value">
              {fmtPrice(t.nav)}
              <Pct v={premium} />
            </div>
          </div>
          <div className="stat">
            <div className="stat-label">{isLive ? 'Raised' : '24h volume'}</div>
            <div className="stat-value">
              {isLive ? (
                <>
                  {t.raised} <small style={{ color: 'var(--muted)' }}>/ {t.target} SOL</small>
                </>
              ) : (
                fmtUsd(t.volume24h)
              )}
            </div>
            {isLive && (
              <div className="progress" style={{ marginTop: 10 }}>
                <div style={{ width: `${(t.raised / t.target) * 100}%` }} />
              </div>
            )}
          </div>
        </div>
      </section>

      <div className="two-col">
        <div className="side">
          <section className="panel chart-panel">
            <div className="chart-top">
              <div>
                <div className="price">{fmtPrice(t.price)}</div>
                <div className="price-sub">
                  <Pct v={t.change24h} /> past {range === '1D' ? 'day' : range.toLowerCase()}
                </div>
              </div>
              <div className="ranges">
                {['1H', '1D', '1W', '1M', 'ALL'].map((r) => (
                  <button key={r} className={`range ${range === r ? 'active' : ''}`} onClick={() => setRange(r)}>
                    {r}
                  </button>
                ))}
              </div>
            </div>
            <Chart data={data} />
          </section>

          <section className="panel activity">
            <div className="swap-head">
              <h3>Activity</h3>
              <span className="count">Live</span>
            </div>
            <table>
              <tbody>
                {activity.map((a, i) => (
                  <tr key={i}>
                    <td className={a.side === 'buy' ? 'up' : 'down'}>{a.side === 'buy' ? 'Buy' : 'Sell'}</td>
                    <td className="mono">{a.amount}</td>
                    <td className="mono muted">
                      {a.tokens} {t.ticker}
                    </td>
                    <td className="mono muted">{a.who}</td>
                    <td className="muted right">{a.ago}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </section>
        </div>

        <aside className="side">
          <section className="panel swap">
            <div className="swap-head">
              <h3>Swap</h3>
              <div className="seg">
                <button className={side === 'buy' ? 'active' : ''} onClick={() => setSide('buy')}>
                  Buy
                </button>
                <button className={side === 'sell' ? 'active' : ''} onClick={() => setSide('sell')}>
                  Sell
                </button>
              </div>
            </div>

            <div className="amount inset">
              <label>{side === 'buy' ? 'You pay' : 'You sell'}</label>
              <div className="amount-row">
                <input placeholder="0" inputMode="decimal" />
                <span className="asset">
                  <span className="asset-dot" />
                  {side === 'buy' ? 'SOL' : t.ticker}
                </span>
              </div>
              <div className="amount-foot">
                <span>$0.00</span>
                <span>Balance 0</span>
              </div>
            </div>

            <div className="quick">
              {['0.1', '0.5', '1', '5'].map((q) => (
                <button key={q}>{q} SOL</button>
              ))}
            </div>

            <button className="btn btn-accent btn-block">{side === 'buy' ? `Buy ${t.ticker}` : `Sell ${t.ticker}`}</button>
          </section>

          <section className="panel basket">
            <h3>Basket</h3>
            <p className="basket-sub">What one ${t.ticker} is pinned to.</p>
            <div className="basket-list">
              {t.basket.map((b) => (
                <div key={b.symbol}>
                  <div className="basket-row">
                    <span className="asset-dot" />
                    <span className="name">{b.symbol}</span>
                    <span className="weight">{b.weight}%</span>
                  </div>
                  <div className="bar">
                    <div style={{ width: `${b.weight}%` }} />
                  </div>
                </div>
              ))}
            </div>
            <div className="basket-nav">
              <span>Basket value</span>
              <strong>{fmtPrice(t.nav)}</strong>
            </div>
          </section>
        </aside>
      </div>

      <Footer go={go} />
    </div>
  )
}
