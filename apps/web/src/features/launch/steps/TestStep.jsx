import { useEffect, useRef, useState } from 'react'
import { Logo, Mint } from '../../../components/ui/index.js'
import { fmtCompact, fmtPct } from '../../../lib/format.js'
import { BUDGET, runStrategy } from '../strategy/runner.js'

// Runs the three functions once, live, and shows what the basket would hold.
// The console on the left is the run as it happened; the book on the right is
// what would be sent on-chain.

export default function TestStep({ draft, dispatch }) {
  const [running, setRunning] = useState(false)
  const [events, setEvents] = useState([])
  const [elapsed, setElapsed] = useState(0)
  const run = draft.strategyRun
  const consoleRef = useRef(null)

  useEffect(() => {
    consoleRef.current?.scrollTo({ top: consoleRef.current.scrollHeight })
  }, [events])

  const start = async () => {
    if (running) return
    setRunning(true)
    setEvents([{ type: 'sys', text: 'sandbox up · network off · ctx attached' }])
    const t0 = performance.now()
    const tick = setInterval(() => setElapsed(performance.now() - t0), 100)
    const res = await runStrategy(draft.strategy, {
      onEvent: (ev) => setEvents((list) => [...list, ev]),
    })
    clearInterval(tick)
    setElapsed(performance.now() - t0)
    setEvents((list) => [
      ...list,
      res.ok
        ? { type: 'sys', text: `done in ${res.ms}ms · ${res.calls.length} data calls · ${res.book.length} assets · hash ${res.hash.slice(0, 12)}…` }
        : { type: 'err', text: `${res.phase}: ${res.message}`, stack: res.stack },
    ])
    dispatch({ type: 'strategyRun', value: res })
    setRunning(false)
  }

  const calls = events.filter((e) => e.type === 'call').length
  const total = run?.ok ? run.book.reduce((s, r) => s + (r.token?.mcap ?? 0), 0) : 0

  return (
    <div className="step step-test">
      <div className="step-title">
        <h2>Test run</h2>
        <p>
          Runs your three functions right now against mainnet, inside the same capability-less sandbox the keeper will use. What comes back is exactly the book
          a rebalance would target today.
        </p>
        <button className={`btn ${run?.ok ? 'btn-ghost raised' : 'btn-accent'} auto-btn`} onClick={start} disabled={running}>
          {running ? (
            <>
              <span className="spinner" /> Running…
            </>
          ) : run ? (
            'Run again'
          ) : (
            '▶ Run strategy'
          )}
        </button>
      </div>

      <div className="test-grid">
        <div className="console">
          <div className="console-head">
            <span className="console-tab">run.log</span>
            <span className="console-meter mono">
              calls {calls}/{BUDGET.calls} · {(elapsed / 1000).toFixed(1)}s / {BUDGET.wallMs / 1000}s
            </span>
          </div>
          <div className="console-body" ref={consoleRef}>
            {events.length === 0 && <div className="console-idle">Nothing has run yet. Press Run strategy.</div>}
            {events.map((e, i) => (
              <Line key={i} e={e} />
            ))}
            {running && <div className="console-cursor" />}
          </div>
        </div>

        <div className="test-result">
          {!run ? (
            <div className="empty test-empty">
              <strong>The book appears here</strong>
              Every mint with its weight, priced, in the order it would be held.
            </div>
          ) : !run.ok ? (
            <div className="notice bad test-fail">
              <div className="test-fail-title">Run failed during {run.phase}</div>
              <pre>{run.message}</pre>
              {run.stack && <pre className="muted">{run.stack}</pre>}
              <p className="muted">Fix it in the previous step and run again. Nothing is saved from a failed run.</p>
            </div>
          ) : (
            <>
              <div className="test-summary">
                <div>
                  <div className="snap-label">Assets</div>
                  <div className="snap-val">{run.book.length}</div>
                </div>
                <div>
                  <div className="snap-label">Selected</div>
                  <div className="snap-val">{run.selected.length}</div>
                </div>
                <div>
                  <div className="snap-label">Combined mcap</div>
                  <div className="snap-val">{fmtCompact(total)}</div>
                </div>
                <div>
                  <div className="snap-label">Code hash</div>
                  <div className="snap-val mono" title={run.hash}>
                    {run.hash.slice(0, 8)}…{run.hash.slice(-6)}
                  </div>
                </div>
              </div>
              <div className="test-book">
                {run.book.map((r) => (
                  <div key={r.mint} className="rv-row">
                    <div className="alloc-bar" style={{ width: `${r.weight_bps / 100}%` }} />
                    <Logo token={r.token} size={26} />
                    <span className="rv-sym">{r.token.symbol}</span>
                    <span className="muted rv-tname">{r.token.name}</span>
                    <Mint mint={r.mint} />
                    <span className="mono rv-pct">{fmtPct(r.weight_bps, 2)}</span>
                  </div>
                ))}
              </div>
              {run.selected.length > run.book.length && (
                <p className="fine">
                  {run.selected.length - run.book.length} selected mint{run.selected.length - run.book.length > 1 ? 's were' : ' was'} dropped by weight() (zero
                  weight or no price).
                </p>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  )
}

function Line({ e }) {
  switch (e.type) {
    case 'sys':
      return <div className="cl cl-sys">{e.text}</div>
    case 'phase':
      return (
        <div className="cl cl-phase">
          <span className="cl-arrow">›</span> {e.phase}()
        </div>
      )
    case 'call':
      return (
        <div className={`cl cl-call ${e.ok ? '' : 'bad'}`}>
          <span className="cl-k">ctx</span> {e.label} <span className="cl-ms">{e.ms}ms</span>
          {!e.ok && <span className="cl-err"> {e.error}</span>}
        </div>
      )
    case 'log':
      return (
        <div className="cl cl-log">
          <span className="cl-k">log</span> {e.args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ')}
        </div>
      )
    case 'selected':
      return (
        <div className="cl cl-sys">
          select() returned {e.mints.length} mint{e.mints.length === 1 ? '' : 's'}
        </div>
      )
    case 'err':
      return (
        <div className="cl cl-bad">
          <div>{e.text}</div>
          {e.stack && <pre>{e.stack}</pre>}
        </div>
      )
    default:
      return null
  }
}
