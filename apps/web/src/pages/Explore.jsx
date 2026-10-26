import { useMemo, useState } from 'react'
import {
  BASKET_TYPES,
  EMPTY_FILTERS,
  MINT_SORTS,
  MINT_WINDOW_H,
  PERFORMANCE_MIN_MC,
  PERFORMANCE_WINDOWS,
  SEARCH_DEFAULTS,
  TRENDING_SORTS,
  activeFilters,
  clearFilter,
  filterBaskets,
  fmtHoursLeft,
  isMintOpen,
  mintHoursLeft,
  mintOpenBaskets,
  mintWindowLeft,
  premiumPct,
  searchBaskets,
  topPerformers,
  trendingBaskets,
} from '@basketfun/core'
import { all as ALL } from '../mocks/baskets.js'
import { fmtUsd } from '../lib/format.js'
import Alive from '../components/effects/Alive.jsx'
import Footer from '../components/layout/Footer.jsx'
import { Delta, Icon, Orb } from '../components/ui/index.js'
import { usePopover } from '../components/ui/usePopover.js'
import QuickSearch from '../features/search/QuickSearch.jsx'
import FilterPanel from '../features/filters/FilterPanel.jsx'

const TYPE_NAME = Object.fromEntries(BASKET_TYPES.map((t) => [t.id, t.name]))

const NEWEST_PAGE = 20

const fmtSpan = (h) => (h >= 48 ? `${h / 24} days` : `${h} hours`)

// Inside half a percent the pool is tracking NAV; calling that a premium is noise.
function PremiumBadge({ t }) {
  const p = premiumPct(t)
  if (Math.abs(p) < 0.5) return <span className="badge">At NAV</span>
  return (
    <span className={`badge ${p > 0 ? 'prem' : 'disc'}`}>
      {p > 0 ? '+' : ''}
      {p.toFixed(1)}% vs NAV
    </span>
  )
}

function TokenCard({ t, onOpen }) {
  const open = isMintOpen(t)

  return (
    <button className="card raised" onClick={() => onOpen(t)}>
      <div className="art">
        <Orb tint={t.tint} small={open} />
        <div className="badges">
          {open ? (
            <span className="badge live">
              <i className="pulse" />
              Mint open
            </span>
          ) : (
            <PremiumBadge t={t} />
          )}
          <span className="badge type">{TYPE_NAME[t.type]}</span>
        </div>
      </div>
      <div className="card-name">{t.name}</div>
      <div className="card-ticker">${t.ticker}</div>
      <div className="card-mc">
        {fmtUsd(t.marketCap)}
        <small className={t.change24h >= 0 ? 'up' : 'down'}>
          {t.change24h >= 0 ? '+' : ''}
          {t.change24h}%
        </small>
      </div>
      {open && (
        <>
          <div className="progress">
            <div style={{ width: `${mintWindowLeft(t) * 100}%` }} />
          </div>
          <div className="card-window">
            <span>Mint at NAV</span>
            <span>{fmtHoursLeft(mintHoursLeft(t))} left</span>
          </div>
        </>
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

function SortBar({ options, value, onChange }) {
  return (
    <div className="sort">
      {options.map((s) => (
        <button key={s.id} className={value === s.id ? 'active' : ''} onClick={() => onChange(s.id)}>
          {s.label}
        </button>
      ))}
    </div>
  )
}

function Grid({ items, openToken }) {
  return (
    <div className="grid">
      {items.map((t) => (
        <TokenCard key={t.id} t={t} onOpen={openToken} />
      ))}
    </div>
  )
}

function MintOpen({ openToken, go }) {
  const [sort, setSort] = useState('new')
  const items = useMemo(() => mintOpenBaskets(ALL, sort), [sort])

  return (
    <section className="panel section alive">
      <Alive edge="bottom" />
      <div className="section-head">
        <h2>Mint open</h2>
        <span className="count live">{items.length}</span>
        <SortBar options={MINT_SORTS} value={sort} onChange={setSort} />
      </div>
      <p className="section-sub">
        New baskets mint at NAV for {MINT_WINDOW_H} hours after their first buy. After that the supply is fixed and the pool sets the price.
      </p>
      {items.length === 0 ? (
        <div className="empty">
          <strong>Nothing minting right now</strong>
          Every basket opens a {MINT_WINDOW_H}-hour mint window on its first buy.
          <div>
            <button className="btn btn-ghost" onClick={() => go('create')}>
              Launch one
            </button>
          </div>
        </div>
      ) : (
        <Grid items={items} openToken={openToken} />
      )}
    </section>
  )
}

function Trending({ openToken }) {
  const [sort, setSort] = useState('vol1h')
  const items = useMemo(() => trendingBaskets(ALL, sort), [sort])

  return (
    <section className="panel section alive">
      {/* Mint open's bottom edge runs right->left, this top edge left->right, so
          the two lights pass each other at the seam. */}
      <Alive edge="top" />
      <div className="section-head">
        <h2>Trending</h2>
        <span className="count">{items.length}</span>
        <SortBar options={TRENDING_SORTS} value={sort} onChange={setSort} />
      </div>
      <p className="section-sub">The baskets trading the most in the last hour.</p>
      {items.length === 0 ? (
        <div className="empty">
          <strong>Nothing trending yet</strong>
          The board fills in as baskets launch and trade.
        </div>
      ) : (
        <Grid items={items} openToken={openToken} />
      )}
    </section>
  )
}

function TopPerformers({ openToken, go }) {
  const [win, setWin] = useState('7d')
  const w = PERFORMANCE_WINDOWS.find((x) => x.id === win)
  const items = useMemo(() => topPerformers(ALL, win), [win])

  return (
    <section className="panel section">
      <div className="section-head">
        <h2>Top performers</h2>
        <span className="count">{items.length}</span>
        <SortBar options={PERFORMANCE_WINDOWS} value={win} onChange={setWin} />
      </div>
      <p className="section-sub">
        Ranked by NAV return: what the holdings did, not what buyers paid on top. Needs {fmtSpan(w.minAgeH)} of history and{' '}
        {fmtUsd(PERFORMANCE_MIN_MC)} market cap.
      </p>
      {items.length === 0 ? (
        <div className="empty">
          <strong>No track records yet</strong>
          Baskets show up here once they have {fmtSpan(w.minAgeH)} of history.
        </div>
      ) : (
        <>
          <ol className="perf">
            {items.map((t, i) => (
              <li key={t.id}>
                <button className="perf-row" onClick={() => openToken(t)}>
                  <span className="perf-rank">{i + 1}</span>
                  <span className="sm-orb perf-orb" style={{ '--tint': t.tint }} />
                  <span className="perf-name">
                    <span>
                      {t.name}
                      <span className="badge type">{TYPE_NAME[t.type]}</span>
                    </span>
                    <small>
                      ${t.ticker} · {fmtUsd(t.marketCap)}
                    </small>
                  </span>
                  <span className="perf-val">
                    <Delta v={w.get(t)} dp={1} />
                    <small>
                      <Delta v={premiumPct(t)} dp={1} /> vs NAV
                    </small>
                  </span>
                </button>
              </li>
            ))}
          </ol>
          <div className="perf-more">
            <button className="btn btn-ghost" onClick={() => go('leaderboard')}>
              Full leaderboard
            </button>
          </div>
        </>
      )}
    </section>
  )
}

function Newest({ openToken, go }) {
  const [q, setQ] = useState('')
  const [f, setF] = useState(EMPTY_FILTERS)
  const [shown, setShown] = useState(NEWEST_PAGE)
  const { open, toggle: togglePop, close, ref } = usePopover()
  const active = activeFilters(f)

  // Newest first until you type; then best match first.
  const items = useMemo(() => {
    const hits = searchBaskets(ALL, q, { ...SEARCH_DEFAULTS, sort: q.trim() ? 'relevance' : 'new' })
    return filterBaskets(hits, f)
  }, [q, f])

  const reset = () => {
    setQ('')
    setF(EMPTY_FILTERS)
  }

  return (
    <section className="panel section">
      <div className="section-head">
        <h2>Newest</h2>
        <span className="count">
          {items.length}
          {items.length !== ALL.length && <small> / {ALL.length}</small>}
        </span>
        <div className="head-right newest-tools">
          <label className="section-search">
            <Icon.Search />
            <input
              value={q}
              onChange={(e) => {
                setQ(e.target.value)
                setShown(NEWEST_PAGE)
              }}
              placeholder="Name, ticker, asset or address"
              spellCheck={false}
            />
            {q && (
              <button onClick={() => setQ('')} aria-label="Clear search">
                <Icon.Close />
              </button>
            )}
          </label>
          <div className="pop-anchor" ref={ref}>
            <button className={`filter-btn ${open || active.length ? 'on' : ''}`} onClick={togglePop} aria-expanded={open}>
              <Icon.Sliders />
              Filters
              {active.length > 0 && <span className="btn-n">{active.length}</span>}
            </button>
            {open && <FilterPanel items={ALL} applied={f} onApply={setF} onClose={close} />}
          </div>
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
          {q ? 'Try a different name, ticker or asset.' : 'Loosen a filter or two.'}
          <div>
            <button className="btn btn-ghost" onClick={reset}>
              Clear search and filters
            </button>
          </div>
        </div>
      ) : (
        <>
          <Grid items={items.slice(0, shown)} openToken={openToken} />
          {items.length > shown && (
            <div className="perf-more">
              <button className="btn btn-ghost" onClick={() => setShown((n) => n + NEWEST_PAGE)}>
                Show more
              </button>
            </div>
          )}
        </>
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

      <MintOpen openToken={openToken} go={go} />
      <Trending openToken={openToken} />
      <TopPerformers openToken={openToken} go={go} />
      <Newest openToken={openToken} go={go} />

      <Footer go={go} />
    </div>
  )
}
