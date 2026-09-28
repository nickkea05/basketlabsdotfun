import { useMemo, useState } from 'react'
import { looksLikeAddress, searchBaskets } from '@basketfun/core'
import { all } from '../../mocks/baskets.js'
import { fmtUsd } from '../../lib/format.js'
import { Icon } from '../../components/ui/index.js'
import { usePopover } from '../../components/ui/usePopover.js'

const LIMIT = 6

// The plain toolbar search: type or paste, results drop down underneath.
// Enter opens the top hit; the last row hands the query to Advanced search.
export default function QuickSearch({ onPick, onAdvanced }) {
  const [q, setQ] = useState('')
  const [cursor, setCursor] = useState(0)
  const { open, setOpen, close, ref } = usePopover()

  const results = useMemo(() => (q.trim() ? searchBaskets(all, q) : []), [q])
  const shown = results.slice(0, LIMIT)
  const showing = open && q.trim().length > 0

  const setQuery = (v) => {
    setQ(v)
    setCursor(0)
    setOpen(true)
  }
  const pick = (t) => {
    close()
    setQ('')
    onPick(t)
  }
  const advanced = () => {
    close()
    onAdvanced(q)
  }

  const onKey = (e) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setCursor((c) => Math.min(shown.length, c + 1))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setCursor((c) => Math.max(0, c - 1))
    } else if (e.key === 'Enter') {
      if (cursor < shown.length && shown[cursor]) pick(shown[cursor])
      else advanced()
    }
  }

  return (
    <div className="search-anchor" ref={ref}>
      <div className={`search raised ${showing ? 'open' : ''}`}>
        <Icon.Search />
        <input
          value={q}
          onChange={(e) => setQuery(e.target.value)}
          onFocus={() => setOpen(true)}
          onKeyDown={onKey}
          placeholder="Search baskets or paste an address"
          spellCheck={false}
          autoComplete="off"
          aria-expanded={showing}
        />
        {q ? (
          <button className="sm-clear" onClick={() => setQuery('')} aria-label="Clear">
            <Icon.Close />
          </button>
        ) : (
          <span className="kbd">⌘K</span>
        )}
      </div>

      {showing && (
        <div className="pop qs-pop raised">
          {shown.length === 0 ? (
            <div className="pop-empty">
              {looksLikeAddress(q) ? (
                <>
                  <strong>That address is not a basket</strong>
                  Only share mints created by basketlabs.fun show up here.
                </>
              ) : (
                <>
                  <strong>No baskets match</strong>
                  Try a name, ticker, or an asset held inside.
                </>
              )}
            </div>
          ) : (
            <ul className="qs-list" role="listbox">
              {shown.map((t, i) => {
                const onCurve = t.raised != null
                return (
                  <li key={t.id} role="option" aria-selected={i === cursor}>
                    <button className={`qs-item ${i === cursor ? 'active' : ''}`} onMouseMove={() => setCursor(i)} onClick={() => pick(t)}>
                      <span className="sm-orb qs-orb" style={{ '--tint': t.tint }} />
                      <span className="qs-name">
                        {t.name}
                        <small>${t.ticker}</small>
                      </span>
                      <span className="qs-val mono">
                        {onCurve ? `${t.raised} / ${t.target} SOL` : fmtUsd(t.marketCap)}
                        <small className={t.change24h >= 0 ? 'up' : 'down'}>
                          {t.change24h >= 0 ? '+' : ''}
                          {t.change24h}%
                        </small>
                      </span>
                    </button>
                  </li>
                )
              })}
            </ul>
          )}
          <button className={`qs-more ${cursor === shown.length ? 'active' : ''}`} onMouseMove={() => setCursor(shown.length)} onClick={advanced}>
            <Icon.Sliders />
            {results.length > LIMIT ? `All ${results.length} results in Advanced search` : 'Open in Advanced search'}
            <Icon.Chevron />
          </button>
        </div>
      )}
    </div>
  )
}
