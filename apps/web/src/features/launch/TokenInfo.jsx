import { useEffect, useState } from 'react'
import Chart from '../../components/charts/Chart.jsx'
import { RANGES, priceSeries, topPool } from '@basketfun/data'
import { TOKEN_2022_PROGRAM } from '@basketfun/solana'
import { CopyButton, Delta, Icon, Logo, Mint } from '../../components/ui/index.js'
import { fmtAge, fmtCompact, fmtNum, fmtTokenPrice } from '../../lib/format.js'

// DEX-style detail view for one token. Sits beside the picker; never over it.
export default function TokenInfo({ token: t, picked, full, onAdd, onRemove, onClose }) {
  const [range, setRange] = useState('24H')
  // Async results are tagged with what they were fetched for, so switching
  // token or range shows "loading" by derivation rather than a state reset.
  const [poolRes, setPoolRes] = useState({ mint: null, pool: undefined, error: null })
  const [seriesRes, setSeriesRes] = useState({ key: null, series: null, error: null })

  const pool = poolRes.mint === t.mint ? poolRes.pool : undefined // undefined = loading, null = none
  const seriesKey = pool ? `${pool.address}:${range}` : null
  const series = seriesKey && seriesRes.key === seriesKey ? seriesRes.series : null
  const chartErr = (poolRes.mint === t.mint ? poolRes.error : null) ?? (seriesRes.key === seriesKey ? seriesRes.error : null)

  useEffect(() => {
    let dead = false
    topPool(t.mint)
      .then((p) => !dead && setPoolRes({ mint: t.mint, pool: p, error: null }))
      .catch((e) => !dead && setPoolRes({ mint: t.mint, pool: null, error: e.message }))
    return () => {
      dead = true
    }
  }, [t.mint])

  useEffect(() => {
    if (!pool) return
    let dead = false
    const key = `${pool.address}:${range}`
    priceSeries(pool.address, range)
      .then((s) => !dead && setSeriesRes({ key, series: s, error: null }))
      .catch((e) => !dead && setSeriesRes({ key, series: null, error: e.message }))
    return () => {
      dead = true
    }
  }, [pool, range])

  const s = t.stats
  const vol24 = s.h24 ? (s.h24.buyVolume ?? 0) + (s.h24.sellVolume ?? 0) : null
  const closes = series?.map((p) => p.close) ?? null
  const rangeDelta = closes && closes.length > 1 ? ((closes[closes.length - 1] - closes[0]) / closes[0]) * 100 : null
  const a = t.audit ?? {}

  const rows = [
    ['Market cap', fmtCompact(t.mcap)],
    ['FDV', fmtCompact(t.fdv)],
    ['Liquidity', fmtCompact(t.liquidity)],
    ['Volume 24h', fmtCompact(vol24)],
    ['Holders', fmtNum(t.holders)],
    ['Circulating', fmtNum(t.circSupply)],
    ['Total supply', fmtNum(t.totalSupply)],
    ['Buys / sells 24h', s.h24 ? `${fmtNum(s.h24.numBuys)} / ${fmtNum(s.h24.numSells)}` : '—'],
    ['Traders 24h', s.h24 ? fmtNum(s.h24.numTraders) : '—'],
    ['Organic score', t.organicScore != null ? Math.round(t.organicScore) : '—'],
    ['Age', fmtAge(t.createdAt)],
    ['Program', t.tokenProgram === TOKEN_2022_PROGRAM ? 'Token-2022' : 'SPL'],
    ['Mint authority', a.mintAuthorityDisabled == null ? '—' : a.mintAuthorityDisabled ? 'Revoked' : 'Active'],
    ['Freeze authority', a.freezeAuthorityDisabled == null ? '—' : a.freezeAuthorityDisabled ? 'Revoked' : 'Active'],
    ['Top 10 holders', a.topHoldersPercentage != null ? `${a.topHoldersPercentage.toFixed(1)}%` : '—'],
    ['Pool', pool ? `${pool.name}${pool.dex ? ` · ${pool.dex}` : ''}` : pool === null ? 'None indexed' : '…'],
  ]

  return (
    <aside className="info panel" aria-label={`${t.symbol} details`}>
      <div className="info-head">
        <Logo token={t} size={44} />
        <div className="info-id">
          <div className="info-name">
            {t.name} <span className="muted">{t.symbol}</span>
          </div>
          <div className="info-mint">
            <Mint mint={t.mint} />
            <CopyButton text={t.mint} label="Copy" />
          </div>
        </div>
        <button className="btn btn-ghost info-clear" onClick={onClose}>
          Clear <Icon.Close />
        </button>
      </div>

      <div className="info-tags chips">
        {t.verified && <span className="chip ok">Verified</span>}
        {t.tags.includes('xstocks') && <span className="chip">xStock</span>}
        {t.tags.includes('major') && <span className="chip">Major</span>}
        {t.tags.includes('lst') && <span className="chip">LST</span>}
        {t.tokenProgram === TOKEN_2022_PROGRAM && <span className="chip">Token-2022</span>}
        {t.website && (
          <a className="chip link-chip" href={t.website} target="_blank" rel="noreferrer">
            Website <Icon.Link />
          </a>
        )}
        {t.twitter && (
          <a className="chip link-chip" href={t.twitter} target="_blank" rel="noreferrer">
            X <Icon.Link />
          </a>
        )}
      </div>

      <div className="info-price-row">
        <div>
          <div className="info-price">{fmtTokenPrice(t.price)}</div>
          <div className="info-sub">
            <Delta v={s.h24?.priceChange} /> <span className="muted">24h</span>
            {rangeDelta != null && range !== '24H' && (
              <>
                <span className="dot" />
                <Delta v={rangeDelta} /> <span className="muted">{range}</span>
              </>
            )}
          </div>
        </div>
        <div className="ranges">
          {Object.keys(RANGES).map((r) => (
            <button key={r} className={`range ${range === r ? 'active' : ''}`} onClick={() => setRange(r)}>
              {r}
            </button>
          ))}
        </div>
      </div>

      <div className="info-chart">
        {closes && closes.length > 1 ? (
          <Chart data={closes} height={200} />
        ) : pool === null || chartErr ? (
          <div className="info-nochart muted">{chartErr ? chartErr : 'No pool history for this token'}</div>
        ) : (
          <div className="info-nochart skeleton" />
        )}
      </div>

      <div className="info-deltas">
        {[
          ['5m', s.m5],
          ['1h', s.h1],
          ['6h', s.h6],
          ['24h', s.h24],
        ].map(([k, v]) => (
          <div key={k} className="info-delta">
            <span className="muted">{k}</span>
            <Delta v={v?.priceChange} />
          </div>
        ))}
      </div>

      <dl className="info-stats">
        {rows.map(([k, v]) => (
          <div key={k} className="info-stat">
            <dt>{k}</dt>
            <dd className="mono">{v}</dd>
          </div>
        ))}
      </dl>

      <div className="info-foot">
        {picked ? (
          <button className="btn btn-ghost raised btn-block" onClick={onRemove}>
            Remove from basket
          </button>
        ) : (
          <button className="btn btn-white btn-block" disabled={full} onClick={onAdd}>
            {full ? 'Basket is full' : 'Add to basket'}
          </button>
        )}
      </div>
    </aside>
  )
}
