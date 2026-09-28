import { useEffect, useMemo, useRef, useState } from 'react'
import {
  SEARCH_AGES,
  BASKET_TYPES,
  SEARCH_DEFAULTS,
  SEARCH_MATCH,
  SEARCH_SORTS,
  SEARCH_STATUS,
  SEARCH_TYPES,
  looksLikeAddress,
  pageResults,
  searchBaskets,
} from '@basketfun/core'
import { all } from '../../mocks/baskets.js'
import { fmtUsd } from '../../lib/format.js'
import { Icon } from '../../components/ui/index.js'
import './search.css'

const TYPE_NAME = Object.fromEntries(BASKET_TYPES.map((t) => [t.id, t.name]))

function Seg({ options, value, onChange }) {
  return (
    <div className="seg">
      {options.map((o) => (
        <button key={o.id} className={value === o.id ? 'active' : ''} onClick={() => onChange(o.id)}>
          {o.label}
        </button>
      ))}
    </div>
  )
}

export default function SearchModal({ open, onClose, onPick, initialQuery = '' }) {
  const [q, setQ] = useState(initialQuery)
  const [f, setF] = useState(SEARCH_DEFAULTS)
  const [pageNo, setPageNo] = useState(1)
  const [cursor, setCursor] = useState(0)
  const inputRef = useRef(null)
  const listRef = useRef(null)

  const results = useMemo(() => searchBaskets(all, q, f), [q, f])
  const pg = pageResults(results, pageNo)
  const active = Object.keys(SEARCH_DEFAULTS).filter((k) => f[k] !== SEARCH_DEFAULTS[k] && k !== 'sort').length

  const set = (k) => (v) => {
    setF((s) => ({ ...s, [k]: v }))
    setPageNo(1)
    setCursor(0)
  }
  const setQuery = (v) => {
    setQ(v)
    setPageNo(1)
    setCursor(0)
  }
  const reset = () => {
    setF(SEARCH_DEFAULTS)
    setPageNo(1)
    setCursor(0)
  }
  const pick = (t) => {
    onClose()
    onPick(t)
  }

  useEffect(() => {
    if (!open) return
    inputRef.current?.focus()
    const onKey = (e) => e.key === 'Escape' && onClose()
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, onClose])

  // Keep the highlighted row in view while arrowing through the list.
  useEffect(() => {
    listRef.current?.children[cursor]?.scrollIntoView?.({ block: 'nearest' })
  }, [cursor])

  if (!open) return null

  const onInputKey = (e) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setCursor((c) => Math.min(pg.items.length - 1, c + 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setCursor((c) => Math.max(0, c - 1))
    } else if (e.key === 'Enter' && pg.items[cursor]) {
      pick(pg.items[cursor])
    }
  }

  const addressMiss = results.length === 0 && looksLikeAddress(q)

  return (
    <div className="sm-overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sm raised" role="dialog" aria-modal="true" aria-label="Search baskets">
        <div className="sm-head">
          <Icon.Search />
          <input
            ref={inputRef}
            value={q}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onInputKey}
            placeholder="Search name, ticker, address, or an asset inside"
            spellCheck={false}
            autoComplete="off"
          />
          {q && (
            <button className="sm-clear" onClick={() => setQuery('')} aria-label="Clear">
              <Icon.Close />
            </button>
          )}
          <span className="kbd">esc</span>
        </div>

        <div className="sm-filters">
          <div className="sm-row">
            <span className="sm-label">Sort</span>
            <Seg options={SEARCH_SORTS} value={f.sort} onChange={set('sort')} />
          </div>
          <div className="sm-row">
            <span className="sm-label">Match</span>
            <Seg options={SEARCH_MATCH} value={f.match} onChange={set('match')} />
          </div>
          <div className="sm-row">
            <span className="sm-label">Age</span>
            <Seg options={SEARCH_AGES} value={f.age} onChange={set('age')} />
            <span className="sm-label sm-label-2">Status</span>
            <Seg options={SEARCH_STATUS} value={f.status} onChange={set('status')} />
          </div>
          <div className="sm-row">
            <span className="sm-label">Type</span>
            <Seg options={SEARCH_TYPES} value={f.type} onChange={set('type')} />
            {active > 0 && (
              <button className="sm-reset" onClick={reset}>
                Clear {active} {active === 1 ? 'filter' : 'filters'}
              </button>
            )}
          </div>
        </div>

        <div className="sm-body">
          {pg.items.length === 0 ? (
            <div className="sm-empty">
              {addressMiss ? (
                <>
                  <strong>That address is not a basket</strong>
                  Only share mints created by basketlabs.fun show up here. Paste a basket's mint, or search by name.
                </>
              ) : (
                <>
                  <strong>No baskets match</strong>
                  {active > 0 ? 'Try clearing a filter.' : 'Try a different name, ticker or asset.'}
                </>
              )}
            </div>
          ) : (
            <ul className="sm-list" ref={listRef} role="listbox">
              {pg.items.map((t, i) => {
                const onCurve = t.raised != null
                return (
                  <li key={t.id} role="option" aria-selected={i === cursor}>
                    <button className={`sm-item ${i === cursor ? 'active' : ''}`} onMouseMove={() => setCursor(i)} onClick={() => pick(t)}>
                      <span className="sm-orb" style={{ '--tint': t.tint }} />
                      <span className="sm-main">
                        <span className="sm-name">
                          {t.name}
                          <span className="badge type">{TYPE_NAME[t.type]}</span>
                        </span>
                        <span className="sm-meta">
                          ${t.ticker}
                          <i />
                          {onCurve ? `${t.raised} / ${t.target} SOL` : `${fmtUsd(t.marketCap)} MC`}
                          <i />
                          {t.age}
                          <i />
                          <span className="mono">{t.ca}</span>
                        </span>
                      </span>
                      <span className="sm-right">
                        <span className={t.change24h >= 0 ? 'up' : 'down'}>
                          {t.change24h >= 0 ? '+' : ''}
                          {t.change24h}%
                        </span>
                        {onCurve ? (
                          <span className="badge live">
                            <i className="pulse" />
                            Curve
                          </span>
                        ) : (
                          <span className="chips sm-chips">
                            {t.basket.slice(0, 3).map((b) => (
                              <span key={b.symbol} className="chip">
                                {b.symbol}
                              </span>
                            ))}
                            {t.basket.length > 3 && <span className="chip">+{t.basket.length - 3}</span>}
                          </span>
                        )}
                      </span>
                      <Icon.Chevron />
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
        </div>

        <div className="sm-foot">
          <span className="muted">
            {pg.total ? `${pg.from} to ${pg.to} of ${pg.total.toLocaleString()}` : '0 results'}
            {pg.total === all.length && !q && ' · every basket on basketlabs.fun'}
          </span>
          <span className="sm-pager">
            <button disabled={pg.page <= 1} onClick={() => setPageNo(pg.page - 1)}>
              Previous
            </button>
            <span className="muted mono">
              {pg.page} / {pg.pages}
            </span>
            <button disabled={pg.page >= pg.pages} onClick={() => setPageNo(pg.page + 1)}>
              Next
            </button>
          </span>
        </div>
      </div>
    </div>
  )
}
