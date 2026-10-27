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
import { Delta, Icon } from '../components/ui/index.js'
import { usePopover } from '../components/ui/usePopover.js'
import QuickSearch from '../features/search/QuickSearch.jsx'
import FilterPanel from '../features/filters/FilterPanel.jsx'

const TYPE_NAME = Object.fromEntries(BASKET_TYPES.map((t) => [t.id, t.name]))

const NEWEST_PAGE = 20
const PERFORMERS = 10

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

const Change = ({ v }) => (
  <small className={v >= 0 ? 'up' : 'down'}>
    {v >= 0 ? '+' : ''}
    {v}%
  </small>
)

const Chips = ({ t, max = 3 }) => (
  <>
    {t.basket.slice(0, max).map((b) => (
      <span key={b.symbol} className="chip">
        {b.symbol}
      </span>
    ))}
    {t.basket.length > max && <span className="chip">+{t.basket.length - max}</span>}
  </>
)

function Row({ t, onOpen }) {
  const open = isMintOpen(t)

  return (
    <button className="row-item" onClick={() => onOpen(t)}>
      <span className="sm-orb row-orb" style={{ '--tint': t.tint }} />
      <span className="row-main">
        <span className="row-name">
          {t.name}
          <small>${t.ticker}</small>
          <span className="badge type">{TYPE_NAME[t.type]}</span>
        </span>
        {open ? (
          <span className="row-window">
            <span className="progress">
              <span style={{ width: `${mintWindowLeft(t) * 100}%` }} />
            </span>
            {fmtHoursLeft(mintHoursLeft(t))} left
          </span>
        ) : (
          <span className="row-sub">
            <Chips t={t} />
            <span className="row-age">{t.age}</span>
          </span>
        )}
      </span>
      <span className="row-val">
        <span>
          {fmtUsd(t.marketCap)}
          <Change v={t.change24h} />
        </span>
        {open ? (
          <span className="badge live">
            <i className="pulse" />
            Mint
          </span>
        ) : (
          <PremiumBadge t={t} />
        )}
      </span>
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

function Rows({ items, openToken }) {
  return (
    <div className="rows">
      {items.map((t) => (
        <Row key={t.id} t={t} onOpen={openToken} />
      ))}
    </div>
  )
}

function Column({ title, count, tools, sub, children }) {
  return (
    <section className="panel section lit dash-col">
      <div className="section-head">
        <h2>{title}</h2>
        {count}
        <div className="head-right col-tools">{tools}</div>
      </div>
      <p className="section-sub">{sub}</p>
      <div className="col-body">{children}</div>
    </section>
  )
}

function Trending({ openToken }) {
  const [sort, setSort] = useState('vol1h')
  const items = useMemo(() => trendingBaskets(ALL, sort), [sort])

  return (
    <Column
      title="Trending"
      count={<span className="count">{items.length}</span>}
      tools={<SortBar options={TRENDING_SORTS} value={sort} onChange={setSort} />}
      sub="The baskets trading the most in the last hour."
    >
      {items.length === 0 ? (
        <div className="empty">
          <strong>Nothing trending yet</strong>
          The board fills in as baskets launch and trade.
        </div>
      ) : (
        <Rows items={items} openToken={openToken} />
      )}
    </Column>
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

  const tools = (
    <>
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
        <button className={`filter-btn ${open || active.length ? 'on' : ''}`} onClick={togglePop} aria-expanded={open} aria-label="Filters">
          <Icon.Sliders />
          {active.length > 0 && <span className="btn-n">{active.length}</span>}
        </button>
        {open && <FilterPanel items={ALL} applied={f} onApply={setF} onClose={close} />}
      </div>
    </>
  )

  return (
    <Column
      title="Newest"
      count={
        <span className="count">
          {items.length}
          {items.length !== ALL.length && <small> / {ALL.length}</small>}
        </span>
      }
      tools={tools}
      sub="Every basket launched on basketlabs.fun, newest first."
    >
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
          <Rows items={items.slice(0, shown)} openToken={openToken} />
          {items.length > shown && (
            <div className="more">
              <button className="btn btn-ghost" onClick={() => setShown((n) => n + NEWEST_PAGE)}>
                Show more
              </button>
            </div>
          )}
        </>
      )}
    </Column>
  )
}

function MintOpen({ openToken, go }) {
  const [sort, setSort] = useState('closing')
  const items = useMemo(() => mintOpenBaskets(ALL, sort), [sort])

  return (
    <Column
      title="Mint open"
      count={<span className="count live">{items.length}</span>}
      tools={<SortBar options={MINT_SORTS} value={sort} onChange={setSort} />}
      sub={`Mint at NAV for ${MINT_WINDOW_H}h after the first buy. Then supply is fixed and the pool sets the price.`}
    >
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
        <Rows items={items} openToken={openToken} />
      )}
    </Column>
  )
}

function TopPerformers({ openToken, go }) {
  const [win, setWin] = useState('7d')
  const w = PERFORMANCE_WINDOWS.find((x) => x.id === win)
  const items = useMemo(() => topPerformers(ALL, win, PERFORMERS), [win])

  return (
    <section className="panel section alive perf-panel">
      <Alive edge="top" />
      <div className="section-head">
        <h2>Top performers</h2>
        <span className="count">{items.length}</span>
        <div className="head-right col-tools">
          <SortBar options={PERFORMANCE_WINDOWS} value={win} onChange={setWin} />
          <button className="btn btn-ghost" onClick={() => go('leaderboard')}>
            Leaderboard
          </button>
        </div>
      </div>
      <p className="section-sub">
        Ranked by NAV return: what the holdings did, not what buyers paid on top. Needs {fmtSpan(w.minAgeH)} of history and $
        {PERFORMANCE_MIN_MC / 1000}K market cap.
      </p>
      {items.length === 0 ? (
        <div className="empty">
          <strong>No track records yet</strong>
          Baskets show up here once they have {fmtSpan(w.minAgeH)} of history.
        </div>
      ) : (
        <div className="perf-table" role="table">
          <div className="perf-tr perf-th" role="row">
            <span>#</span>
            <span>Basket</span>
            <span className="perf-holds">Holds</span>
            <span className="r">NAV return</span>
            <span className="r">Price vs NAV</span>
            <span className="r">Market cap</span>
            <span className="r perf-age">Age</span>
          </div>
          {items.map((t, i) => (
            <button key={t.id} className="perf-tr" role="row" onClick={() => openToken(t)}>
              <span className="perf-rank">{i + 1}</span>
              <span className="perf-name">
                <span className="sm-orb perf-orb" style={{ '--tint': t.tint }} />
                <span>
                  {t.name}
                  <small>${t.ticker}</small>
                </span>
                <span className="badge type">{TYPE_NAME[t.type]}</span>
              </span>
              <span className="perf-holds chips">
                <Chips t={t} max={4} />
              </span>
              <span className="r perf-ret">
                <Delta v={w.get(t)} dp={1} />
              </span>
              <span className="r">
                <Delta v={premiumPct(t)} dp={1} />
              </span>
              <span className="r">{fmtUsd(t.marketCap)}</span>
              <span className="r muted perf-age">{t.age}</span>
            </button>
          ))}
        </div>
      )}
    </section>
  )
}

export default function Explore({ openToken, go }) {
  return (
    <div className="page container wide">
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

      <div className="dash">
        <Trending openToken={openToken} />
        <Newest openToken={openToken} go={go} />
        <MintOpen openToken={openToken} go={go} />
      </div>

      <TopPerformers openToken={openToken} go={go} />

      <Footer go={go} />
    </div>
  )
}
