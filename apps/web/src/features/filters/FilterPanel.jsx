import { useState } from 'react'
import { EMPTY_FILTERS, FILTER_STATUS, FILTER_TYPES, RANGE_FIELDS, activeFilters, filterBaskets } from '@basketfun/core'
import { Icon } from '../../components/ui/index.js'

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

const PLACEHOLDER = { $: ['Min, e.g. 100k', 'Max, e.g. 5m'], '%': ['Min, e.g. -10', 'Max, e.g. 50'], h: ['Min hours', 'Max hours'], '': ['Min', 'Max'] }

// Axiom-style filter form. Edits a draft; nothing applies until Apply, so
// half-typed numbers never flash the list around. Mounted only while open,
// so the draft always starts from what is currently applied.
export default function FilterPanel({ items, applied, onApply, onClose }) {
  const [d, setD] = useState(applied)
  const preview = filterBaskets(items, d).length
  const dirty = JSON.stringify(d) !== JSON.stringify(applied)
  const count = activeFilters(d).length

  const set = (k) => (v) => setD((s) => ({ ...s, [k]: v }))
  const setRange = (id, side) => (e) => {
    const v = e.target.value
    setD((s) => {
      const cur = { ...(s.ranges[id] ?? {}), [side]: v }
      if (!cur.min && !cur.max) {
        const ranges = { ...s.ranges }
        delete ranges[id]
        return { ...s, ranges }
      }
      return { ...s, ranges: { ...s.ranges, [id]: cur } }
    })
  }
  const apply = () => {
    onApply(d)
    onClose()
  }
  const onKey = (e) => e.key === 'Enter' && apply()

  return (
    <div className="pop filter-pop raised" onKeyDown={onKey}>
      <div className="fp-head">
        <span className="fp-title">
          Filters
          {count > 0 && <span className="btn-n">{count}</span>}
        </span>
        <button className="fp-reset" onClick={() => setD(EMPTY_FILTERS)} disabled={count === 0}>
          Reset all
        </button>
      </div>

      <div className="fp-row">
        <span className="filter-label">Status</span>
        <Seg options={FILTER_STATUS} value={d.status} onChange={set('status')} />
      </div>
      <div className="fp-row">
        <span className="filter-label">Type</span>
        <Seg options={FILTER_TYPES} value={d.type} onChange={set('type')} />
      </div>

      <div className="fp-grid">
        <label className="fp-field">
          <span>Include keywords</span>
          <div className="input">
            <input value={d.include} onChange={(e) => set('include')(e.target.value)} placeholder="sol, jup, dog…" spellCheck={false} />
          </div>
        </label>
        <label className="fp-field">
          <span>Exclude keywords</span>
          <div className="input">
            <input value={d.exclude} onChange={(e) => set('exclude')(e.target.value)} placeholder="meme, test…" spellCheck={false} />
          </div>
        </label>
      </div>
      <p className="fp-hint">Keywords match the basket name, ticker, or any asset it holds. Comma-separated.</p>

      <div className="fp-ranges">
        {RANGE_FIELDS.map((r) => {
          const v = d.ranges[r.id] ?? {}
          const [pmin, pmax] = PLACEHOLDER[r.unit]
          return (
            <div key={r.id} className="fp-range">
              <span className="fp-range-label">
                {r.label}
                {r.unit && <small>({r.unit === 'h' ? 'hours' : r.unit})</small>}
              </span>
              <div className="input">
                <input value={v.min ?? ''} onChange={setRange(r.id, 'min')} placeholder={pmin} inputMode="decimal" />
              </div>
              <div className="input">
                <input value={v.max ?? ''} onChange={setRange(r.id, 'max')} placeholder={pmax} inputMode="decimal" />
              </div>
            </div>
          )
        })}
      </div>

      <div className="filter-foot">
        <span className="muted">
          <strong className="fp-n">{preview}</strong> of {items.length} baskets
        </span>
        <div>
          <button className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="btn btn-white" onClick={apply} disabled={!dirty}>
            <Icon.Check />
            Apply
          </button>
        </div>
      </div>
    </div>
  )
}
