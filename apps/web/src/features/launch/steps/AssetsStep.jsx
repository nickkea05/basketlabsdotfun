import { useEffect, useMemo, useRef, useState } from 'react'
import { searchTokens, topTokens, xstocks } from '@basketfun/data'
import { TOKEN_2022_PROGRAM, TOKEN_PROGRAM } from '@basketfun/solana'
import { MIN_ASSETS, maxAssetsFor } from '@basketfun/core'
import { Icon, Logo, Mint } from '../../../components/ui/index.js'
import { fmtCompact, fmtTokenPrice } from '../../../lib/format.js'
import TokenInfo from '../TokenInfo.jsx'

const CHAINS = [
  { id: 'solana', label: 'Solana' },
  { id: 'base', label: 'Base', soon: true },
  { id: 'ethereum', label: 'Ethereum', soon: true },
  { id: 'bsc', label: 'BNB', soon: true },
]

const isMajor = (t) => t.tags.includes('major') || (t.verified && (t.mcap ?? 0) >= 5e9)

const defaultAdv = { chain: 'solana', mcapMin: '', mcapMax: '', liqMin: '', holdersMin: '', program: 'any' }

// Search results per query, shared across mounts of the step.
const searchCache = new Map()

export default function AssetsStep({ draft, dispatch }) {
  const [query, setQuery] = useState('')
  const [chips, setChips] = useState({ majors: false, big: false, xstocks: false, verified: false })
  const [adv, setAdv] = useState(defaultAdv)
  const [advOpen, setAdvOpen] = useState(false)
  const [metric, setMetric] = useState('price') // 'price' | 'mcap'
  const [browse, setBrowse] = useState(null)
  const [search, setSearch] = useState({ q: '', results: null, error: null }) // last completed search
  const [browseErr, setBrowseErr] = useState(null)
  const [info, setInfo] = useState(null) // token shown in the side panel
  const advRef = useRef(null)

  // Everything about the current query is derived, so changing it never
  // needs a state reset inside an effect.
  const q = query.trim()
  const results = !q ? null : search.q === q ? search.results : (searchCache.get(q) ?? null)
  const busy = !!q && results == null && search.q !== q
  const err = browseErr ?? (search.q === q ? search.error : null)

  // Default browse set: most-traded tokens plus every xStock, by market cap.
  useEffect(() => {
    let dead = false
    Promise.all([topTokens(100), xstocks()])
      .then(([top, xs]) => {
        if (dead) return
        const seen = new Set()
        const all = [...top, ...xs].filter((t) => (seen.has(t.mint) ? false : seen.add(t.mint)))
        all.sort((a, b) => (b.mcap ?? 0) - (a.mcap ?? 0))
        setBrowse(all)
      })
      .catch((e) => !dead && setBrowseErr(e.message))
    return () => {
      dead = true
    }
  }, [])

  // Debounced search. Mints, names and symbols all go through the same call.
  useEffect(() => {
    if (!q || searchCache.has(q)) return
    const id = setTimeout(async () => {
      try {
        const r = await searchTokens(q)
        searchCache.set(q, r)
        setSearch({ q, results: r, error: null })
      } catch (e) {
        setSearch({ q, results: null, error: e.message })
      }
    }, 280)
    return () => clearTimeout(id)
  }, [q])

  useEffect(() => {
    if (!advOpen) return
    const close = (e) => advRef.current && !advRef.current.contains(e.target) && setAdvOpen(false)
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [advOpen])

  const advActive = Object.keys(defaultAdv).some((k) => adv[k] !== defaultAdv[k])

  const shown = useMemo(() => {
    let list = results ?? browse ?? []
    if (chips.majors) list = list.filter(isMajor)
    if (chips.xstocks) list = list.filter((t) => t.tags.includes('xstocks'))
    if (chips.big) list = list.filter((t) => (t.mcap ?? 0) >= 1e9)
    if (chips.verified) list = list.filter((t) => t.verified)
    const num = (v) => (v === '' ? null : Number(v))
    const [mn, mx, lq, hd] = [num(adv.mcapMin), num(adv.mcapMax), num(adv.liqMin), num(adv.holdersMin)]
    if (mn != null) list = list.filter((t) => (t.mcap ?? 0) >= mn)
    if (mx != null) list = list.filter((t) => (t.mcap ?? 0) <= mx)
    if (lq != null) list = list.filter((t) => (t.liquidity ?? 0) >= lq)
    if (hd != null) list = list.filter((t) => (t.holders ?? 0) >= hd)
    if (adv.program === 'spl') list = list.filter((t) => t.tokenProgram === TOKEN_PROGRAM)
    if (adv.program === 't22') list = list.filter((t) => t.tokenProgram === TOKEN_2022_PROGRAM)
    return list
  }, [results, browse, chips, adv])

  const picked = new Set(draft.assets.map((t) => t.mint))
  const cap = maxAssetsFor(draft.basketType)
  const capped = Number.isFinite(cap)
  const full = draft.assets.length >= cap
  const toggleChip = (k) => setChips((c) => ({ ...c, [k]: !c[k] }))

  return (
    <div className={`step step-assets ${info ? 'with-info' : ''}`}>
      <div className="assets-main">
        <div className="step-title">
          <h2>Pick the assets</h2>
          <p>
            Search by name, ticker or mint.{' '}
            {capped ? `${MIN_ASSETS}–${cap} assets, fixed for good.` : `At least ${MIN_ASSETS} assets; you can change them later.`} Weights come next.
          </p>
        </div>

        <div className="asset-toolbar">
          <div className="search raised">
            <Icon.Search />
            <input autoFocus placeholder="Search tokens or paste a mint" value={query} onChange={(e) => setQuery(e.target.value)} />
            {busy && <span className="spinner" />}
          </div>
          <div className="seg">
            <button className={metric === 'price' ? 'active' : ''} onClick={() => setMetric('price')}>
              Price
            </button>
            <button className={metric === 'mcap' ? 'active' : ''} onClick={() => setMetric('mcap')}>
              MC
            </button>
          </div>
        </div>

        <div className="filter-row">
          <button className={`fchip ${chips.majors ? 'on' : ''}`} onClick={() => toggleChip('majors')}>
            Majors
          </button>
          <button className={`fchip ${chips.big ? 'on' : ''}`} onClick={() => toggleChip('big')}>
            &gt; $1B cap
          </button>
          <button className={`fchip ${chips.xstocks ? 'on' : ''}`} onClick={() => toggleChip('xstocks')}>
            xStocks
          </button>
          <button className={`fchip ${chips.verified ? 'on' : ''}`} onClick={() => toggleChip('verified')}>
            Verified
          </button>

          <div className="adv" ref={advRef}>
            <button className={`fchip ${advActive || advOpen ? 'on' : ''}`} onClick={() => setAdvOpen((o) => !o)}>
              <Icon.Sliders /> Filters {advActive && <i className="fdot" />}
              <Icon.Chevron />
            </button>
            {advOpen && (
              <div className="adv-panel panel">
                <div className="adv-label">Chain</div>
                <div className="adv-chains">
                  {CHAINS.map((c) => (
                    <button key={c.id} className={`fchip ${adv.chain === c.id ? 'on' : ''}`} disabled={c.soon} onClick={() => setAdv({ ...adv, chain: c.id })}>
                      {c.label}
                      {c.soon && <small>soon</small>}
                    </button>
                  ))}
                </div>
                <div className="adv-label">Market cap (USD)</div>
                <div className="adv-pair">
                  <div className="input">
                    <input type="number" placeholder="Min" value={adv.mcapMin} onChange={(e) => setAdv({ ...adv, mcapMin: e.target.value })} />
                  </div>
                  <div className="input">
                    <input type="number" placeholder="Max" value={adv.mcapMax} onChange={(e) => setAdv({ ...adv, mcapMax: e.target.value })} />
                  </div>
                </div>
                <div className="adv-pair">
                  <div>
                    <div className="adv-label">Min liquidity</div>
                    <div className="input">
                      <input type="number" placeholder="USD" value={adv.liqMin} onChange={(e) => setAdv({ ...adv, liqMin: e.target.value })} />
                    </div>
                  </div>
                  <div>
                    <div className="adv-label">Min holders</div>
                    <div className="input">
                      <input type="number" placeholder="0" value={adv.holdersMin} onChange={(e) => setAdv({ ...adv, holdersMin: e.target.value })} />
                    </div>
                  </div>
                </div>
                <div className="adv-label">Token program</div>
                <div className="seg">
                  {[
                    ['any', 'Any'],
                    ['spl', 'SPL'],
                    ['t22', 'Token-2022'],
                  ].map(([v, l]) => (
                    <button key={v} className={adv.program === v ? 'active' : ''} onClick={() => setAdv({ ...adv, program: v })}>
                      {l}
                    </button>
                  ))}
                </div>
                <div className="adv-foot">
                  <button className="link" onClick={() => setAdv(defaultAdv)}>
                    Reset
                  </button>
                  <span className="muted">{shown.length} match</span>
                </div>
              </div>
            )}
          </div>

          <span className="muted result-count">{browse == null && results == null ? 'Loading…' : `${shown.length} tokens`}</span>
        </div>

        {err && <div className="notice bad">{err}</div>}

        <div className="asset-scroll">
          {browse == null && results == null && !err ? (
            <div className="tile-grid">
              {Array.from({ length: 12 }).map((_, i) => (
                <div key={i} className="tile skeleton" />
              ))}
            </div>
          ) : shown.length === 0 ? (
            <div className="empty">
              <strong>No tokens match</strong>
              Try a different search or loosen the filters.
            </div>
          ) : (
            <div className="tile-grid">
              {shown.map((t) => (
                <Tile
                  key={t.mint}
                  t={t}
                  metric={metric}
                  picked={picked.has(t.mint)}
                  disabled={full && !picked.has(t.mint)}
                  onPick={() => dispatch({ type: 'addAsset', token: t })}
                  onRemove={() => dispatch({ type: 'removeAsset', mint: t.mint })}
                  onInfo={() => setInfo(t)}
                  active={info?.mint === t.mint}
                />
              ))}
            </div>
          )}
        </div>

        <Tray assets={draft.assets} cap={capped ? cap : null} onRemove={(mint) => dispatch({ type: 'removeAsset', mint })} />
      </div>

      {info && (
        <TokenInfo
          key={info.mint}
          token={info}
          picked={picked.has(info.mint)}
          full={full}
          onAdd={() => dispatch({ type: 'addAsset', token: info })}
          onRemove={() => dispatch({ type: 'removeAsset', mint: info.mint })}
          onClose={() => setInfo(null)}
        />
      )}
    </div>
  )
}

function Tile({ t, metric, picked, disabled, active, onPick, onRemove, onInfo }) {
  return (
    <div className={`tile raised ${picked ? 'picked' : ''} ${disabled ? 'disabled' : ''} ${active ? 'active' : ''}`}>
      <button className="tile-hit" disabled={picked || disabled} onClick={onPick} aria-label={`Add ${t.symbol}`}>
        <div className="tile-art">
          <Logo token={t} size={46} />
          {t.tags.includes('xstocks') ? <span className="tile-tag">xStock</span> : t.verified ? <span className="tile-tag ok">✓</span> : null}
        </div>
        <div className="tile-sym">{t.symbol}</div>
        <div className="tile-name">{t.name}</div>
        <div className="tile-mint">
          <Mint mint={t.mint} />
        </div>
        <div className="tile-val">
          {metric === 'price' ? fmtTokenPrice(t.price) : fmtCompact(t.mcap)}
          <small>{metric === 'price' ? '' : 'MC'}</small>
        </div>
      </button>
      <button className="tile-info" onClick={onInfo} aria-label={`About ${t.symbol}`} title="Details">
        <Icon.Info />
      </button>
      {picked && (
        <button className="notch" onClick={onRemove} aria-label={`Remove ${t.symbol}`}>
          <Icon.Close />
        </button>
      )}
    </div>
  )
}

function Tray({ assets, cap, onRemove }) {
  return (
    <div className="tray raised">
      <div className="tray-label">
        Selected <span className="mono">{assets.length}</span>
        {cap != null && <span className="muted"> / {cap}</span>}
      </div>
      <div className="tray-logos">
        {assets.length === 0 && <span className="muted">Nothing yet — click a token to add it.</span>}
        {assets.map((t) => (
          <button key={t.mint} className="tray-item" title={`${t.symbol} — remove`} onClick={() => onRemove(t.mint)}>
            <Logo token={t} size={30} />
            <span className="tray-x">
              <Icon.Close />
            </span>
          </button>
        ))}
      </div>
      {assets.length > 0 && assets.length < MIN_ASSETS && <span className="muted">Add at least {MIN_ASSETS}</span>}
    </div>
  )
}
