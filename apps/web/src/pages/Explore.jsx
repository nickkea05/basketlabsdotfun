import { useMemo, useState } from 'react'
import { BASKET_TYPES, EMPTY_FILTERS, activeFilters, clearFilter, filterBaskets } from '@basketfun/core'
import { all as ALL } from '../mocks/baskets.js'
import { fmtUsd } from '../lib/format.js'
import Alive from '../components/effects/Alive.jsx'
import Footer from '../components/layout/Footer.jsx'
import { Icon, Orb } from '../components/ui/index.js'
import { usePopover } from '../components/ui/usePopover.js'
import QuickSearch from '../features/search/QuickSearch.jsx'
import FilterPanel from '../features/filters/FilterPanel.jsx'

const TYPE_NAME = Object.fromEntries(BASKET_TYPES.map((t) => [t.id, t.name]))

// Trending sort keys. "Most of that kind" = descending, except newest, which
// is ascending age. Live only ever uses `new`.
const SORTS = {
  mc: { label: 'Market cap', cmp: (a, b) => b.marketCap - a.marketCap },
  vol: { label: '24h volume', cmp: (a, b) => b.volume24h - a.volume24h },
  chg: { label: '24h change', cmp: (a, b) => b.change24h - a.change24h },
  new: { label: 'Newest', cmp: (a, b) => a.ageH - b.ageH },
}

const isOnCurve = (t) => t.raised != null

function TokenCard({ t, onOpen }) {
  const liveCurve = isOnCurve(t)
  const pct = liveCurve ? Math.round((t.raised / t.target) * 100) : null

  return (
    <button className="card raised" onClick={() => onOpen(t)}>
      <div className="art">
        <Orb tint={t.tint} small={liveCurve} />
        <div className="badges">
          <span className={`badge ${liveCurve ? 'live' : ''}`}>
            {liveCurve && <i className="pulse" />}
            {liveCurve ? 'On curve' : 'Graduated'}
          </span>
          <span className="badge type">{TYPE_NAME[t.type]}</span>
        </div>
      </div>
      <div className="card-name">{t.name}</div>
      <div className="card-ticker">${t.ticker}</div>
      {liveCurve ? (
        <>
          <div className="card-mc">
            {t.raised} <small>/ {t.target} SOL</small>
          </div>
          <div className="progress">
            <div style={{ width: `${pct}%` }} />
          </div>
        </>
      ) : (
        <div className="card-mc">
          {fmtUsd(t.marketCap)}
          <small className={t.change24h >= 0 ? 'up' : 'down'}>
            {t.change24h >= 0 ? '+' : ''}
            {t.change24h}%
          </small>
        </div>
      )}
      <div className="card-foot">
        <div className="chips">
          {t.basket.map((b) => (
            <span key={b.symbol} className="chip">
              {b.symbol}
            </span>
          ))}
        </div>
        <span>{t.age}</span>
      </div>
    </button>
  )
}

function SortBar({ value, onChange }) {
  return (
    <div className="sort">
      {Object.entries(SORTS).map(([id, s]) => (
        <button key={id} className={value === id ? 'active' : ''} onClick={() => onChange(id)}>
          {s.label}
        </button>
      ))}
    </div>
  )
}

function Trending({ openToken }) {
  const [sort, setSort] = useState('mc')
  const items = useMemo(() => [...ALL].sort(SORTS[sort].cmp).slice(0, 10), [sort])

  return (
    <section className="panel section alive">
      <Alive edge="bottom" />
      <div className="section-head">
        <h2>Trending</h2>
        <span className="count live">{items.length}</span>
        <SortBar value={sort} onChange={setSort} />
      </div>
      <p className="section-sub">The baskets doing the most right now. Pick what "most" means.</p>
      {items.length === 0 ? (
        <div className="empty">
          <strong>Nothing trending yet</strong>
          The board fills in as baskets launch and trade.
        </div>
      ) : (
        <div className="grid">
          {items.map((t) => (
            <TokenCard key={t.id} t={t} onOpen={openToken} />
          ))}
        </div>
      )}
    </section>
  )
}

function Live({ openToken, go }) {
  const [f, setF] = useState(EMPTY_FILTERS)
  const { open, toggle: togglePop, close, ref } = usePopover()
  const active = activeFilters(f)

  // Newest first, always. Live is the firehose, not a ranking.
  const items = useMemo(() => filterBaskets(ALL, f).sort(SORTS.new.cmp), [f])

  return (
    <section className="panel section alive">
      {/* Same rotation as Trending: its bottom edge runs right->left, this top edge
          left->right, so the two lights pass each other at the seam. */}
      <Alive edge="top" />
      <div className="section-head">
        <h2>Live</h2>
        <span className="count">
          {items.length}
          {items.length !== ALL.length && <small> / {ALL.length}</small>}
        </span>
        <div className="pop-anchor head-right" ref={ref}>
          <button className={`filter-btn ${open || active.length ? 'on' : ''}`} onClick={togglePop} aria-expanded={open}>
            <Icon.Sliders />
            Filters
            {active.length > 0 && <span className="btn-n">{active.length}</span>}
          </button>
          {open && <FilterPanel items={ALL} applied={f} onApply={setF} onClose={close} />}
        </div>
      </div>
      <p className="section-sub">Every basket launched on basketlabs.fun, newest first.</p>

      {active.length > 0 && (
        <div className="active-filters">
          {active.map((a) => (
            <button key={a.id} className="afilter" onClick={() => setF((s) => clearFilter(s, a.id))}>
              <span className="muted">{a.label}</span>
              {a.value}
              <Icon.Close />
            </button>
          ))}
          <button className="reset" onClick={() => setF(EMPTY_FILTERS)}>
            Clear all
          </button>
        </div>
      )}

      {ALL.length === 0 ? (
        <div className="empty">
          <strong>Nothing launched yet</strong>
          New baskets show up here the moment they open.
          <div>
            <button className="btn btn-ghost" onClick={() => go('create')}>
              Launch the first
            </button>
          </div>
        </div>
      ) : items.length === 0 ? (
        <div className="empty">
          <strong>No baskets match</strong>
          Loosen a filter or two.
          <div>
            <button className="btn btn-ghost" onClick={() => setF(EMPTY_FILTERS)}>
              Clear filters
            </button>
          </div>
        </div>
      ) : (
        <div className="grid">
          {items.map((t) => (
            <TokenCard key={t.id} t={t} onOpen={openToken} />
          ))}
        </div>
      )}
    </section>
  )
}

export default function Explore({ openToken, go }) {
  return (
    <div className="page container">
      <div className="toolbar">
        <QuickSearch onPick={openToken} onAdvanced={(q) => go('search', q)} />
        <button className="btn btn-flat tall" onClick={() => go('search')}>
          <Icon.Sliders />
          Advanced
        </button>
        <button className="btn btn-white tall" onClick={() => go('create')}>
          + Launch
        </button>
      </div>

      <Trending openToken={openToken} />
      <Live openToken={openToken} go={go} />

      <Footer go={go} />
    </div>
  )
}
