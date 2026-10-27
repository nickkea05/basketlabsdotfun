import { useEffect, useMemo, useState } from 'react'
import {
  BASKET_TYPES,
  EMPTY_FILTERS,
  MINT_SORTS,
  MINT_WINDOW_H,
  PERFORMANCE_MIN_MC,
  PERFORMANCE_WINDOWS,
  SEARCH_DEFAULTS,
  TRENDING_SIZE,
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

// The rail is fixed height; Mint open gets whatever Top performers leaves.
const RAIL_PERFORMERS = 5

const fmtSpan = (h) => (h >= 48 ? `${h / 24} days` : `${h} hours`)

// Which column has taken over the main area. Kept in the URL so a refresh
// lands back in the same view.
const VIEWS = ['trending', 'newest']
const readView = () => {
  const v = new URLSearchParams(window.location.search).get('view')
  return VIEWS.includes(v) ? v : null
}
const writeView = (v) => {
  const url = new URL(window.location.href)
  if (v) url.searchParams.set('view', v)
  else url.searchParams.delete('view')
  window.history.replaceState(null, '', url)
}

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
        <Change v={t.change24h} />
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

/** The dense list form used in the columns and the rail. */
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
            {t.basket.slice(0, 3).map((b) => (
              <span key={b.symbol} className="chip">
                {b.symbol}
              </span>
            ))}
            {t.basket.length > 3 && <span className="chip">+{t.basket.length - 3}</span>}
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

function Grid({ items, openToken }) {
  return (
    <div className="grid">
      {items.map((t) => (
        <TokenCard key={t.id} t={t} onOpen={openToken} />
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

function ExpandButton({ expanded, onToggle }) {
  return expanded ? (
    <button className="expand-btn on" onClick={onToggle} title="Back to the dashboard (Esc)">
      <Icon.Collapse />
      Collapse
      <span className="kbd">Esc</span>
    </button>
  ) : (
    <button className="expand-btn" onClick={onToggle} title="Expand to full view">
      <Icon.Expand />
    </button>
  )
}

/** A main-area section: a scrolling column on the dashboard, or the whole area when expanded. */
function Column({ title, count, tools, sub, expanded, onToggle, children }) {
  return (
    <section className={`panel section dash-col ${expanded ? 'expanded' : ''}`}>
      {expanded && <div className="expanded-pill">Expanded view · collapse to return to the dashboard</div>}
      <div className="section-head">
        <h2>{title}</h2>
        {count}
        <div className="head-right col-tools">
          {tools}
          <ExpandButton expanded={expanded} onToggle={onToggle} />
        </div>
      </div>
      {sub && <p className="section-sub">{sub}</p>}
      <div className="col-body">{children}</div>
    </section>
  )
}

function Trending({ openToken, expanded, onToggle }) {
  const [sort, setSort] = useState('vol1h')
  const items = useMemo(() => trendingBaskets(ALL, sort, expanded ? Infinity : TRENDING_SIZE), [sort, expanded])

  return (
    <Column
      title="Trending"
      count={<span className="count">{items.length}</span>}
      tools={<SortBar options={TRENDING_SORTS} value={sort} onChange={setSort} />}
      sub="The baskets trading the most in the last hour."
      expanded={expanded}
      onToggle={onToggle}
    >
      {items.length === 0 ? (
        <div className="empty">
          <strong>Nothing trending yet</strong>
          The board fills in as baskets launch and trade.
        </div>
      ) : expanded ? (
        <Grid items={items} openToken={openToken} />
      ) : (
        <Rows items={items} openToken={openToken} />
      )}
    </Column>
  )
}

function Newest({ openToken, go, expanded, onToggle }) {
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
        <button className={`filter-btn ${open || active.length ? 'on' : ''}`} onClick={togglePop} aria-expanded={open}>
          <Icon.Sliders />
          {expanded && 'Filters'}
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
      sub={expanded ? 'Every basket launched on basketlabs.fun, newest first.' : null}
      expanded={expanded}
      onToggle={onToggle}
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
          {expanded ? <Grid items={items.slice(0, shown)} openToken={openToken} /> : <Rows items={items.slice(0, shown)} openToken={openToken} />}
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

function TopPerformers({ openToken, go }) {
  const [win, setWin] = useState('7d')
  const w = PERFORMANCE_WINDOWS.find((x) => x.id === win)
  const items = useMemo(() => topPerformers(ALL, win, RAIL_PERFORMERS), [win])

  return (
    <section className="panel section rail-panel">
      <div className="section-head">
        <h2>Top performers</h2>
        <button className="rail-link" onClick={() => go('leaderboard')}>
          Leaderboard
        </button>
      </div>
      <SortBar options={PERFORMANCE_WINDOWS} value={win} onChange={setWin} />
      <p className="section-sub">
        By NAV return · min {fmtSpan(w.minAgeH)}, ${PERFORMANCE_MIN_MC / 1000}K cap
      </p>
      {items.length === 0 ? (
        <div className="empty">
          <strong>No track records yet</strong>
          Baskets show up here once they have {fmtSpan(w.minAgeH)} of history.
        </div>
      ) : (
        <ol className="perf">
          {items.map((t, i) => (
            <li key={t.id}>
              <button className="perf-row" onClick={() => openToken(t)}>
                <span className="perf-rank">{i + 1}</span>
                <span className="sm-orb perf-orb" style={{ '--tint': t.tint }} />
                <span className="perf-name">
                  <span>{t.name}</span>
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
      )}
    </section>
  )
}

function MintOpen({ openToken, go }) {
  const [sort, setSort] = useState('closing')
  const items = useMemo(() => mintOpenBaskets(ALL, sort), [sort])

  return (
    <section className="panel section alive rail-panel rail-fill">
      <Alive edge="top" />
      <div className="section-head">
        <h2>Mint open</h2>
        <span className="count live">{items.length}</span>
      </div>
      <SortBar options={MINT_SORTS} value={sort} onChange={setSort} />
      <p className="section-sub">Mint at NAV for {MINT_WINDOW_H}h after the first buy. Then supply is fixed and the pool sets the price.</p>
      <div className="col-body">
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
      </div>
    </section>
  )
}

export default function Explore({ openToken, go }) {
  const [view, setView] = useState(readView)
  const toggle = (v) => setView((cur) => (cur === v ? null : v))

  useEffect(() => writeView(view), [view])

  useEffect(() => {
    if (!view) return
    const onKey = (e) => e.key === 'Escape' && setView(null)
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [view])

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

      <div className={`dash ${view ? 'has-expanded' : ''}`}>
        <div className="dash-main">
          {(!view || view === 'trending') && <Trending openToken={openToken} expanded={view === 'trending'} onToggle={() => toggle('trending')} />}
          {(!view || view === 'newest') && <Newest openToken={openToken} go={go} expanded={view === 'newest'} onToggle={() => toggle('newest')} />}
        </div>
        <aside className="dash-rail">
          <TopPerformers openToken={openToken} go={go} />
          <MintOpen openToken={openToken} go={go} />
        </aside>
      </div>

      <Footer go={go} />
    </div>
  )
}
