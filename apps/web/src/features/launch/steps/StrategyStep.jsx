import { useMemo, useState } from 'react'
import { STRATEGY_SECTIONS, lintStrategy } from '@basketfun/core'
import CodeEditor from '../../../components/code/CodeEditor.jsx'
import { Icon } from '../../../components/ui/index.js'
import { EXAMPLE } from '../strategy/example.js'

const SPEC_URL = '/strategy-spec.md'

export default function StrategyStep({ draft, dispatch }) {
  const lint = useMemo(() => lintStrategy(draft.strategy), [draft.strategy])
  const errorsFor = (id) => lint.filter((e) => e.startsWith(`${id}:`)).map((e) => e.slice(id.length + 1).trim())
  const set = (id) => (value) => dispatch({ type: 'strategySection', id, value })
  const empty = STRATEGY_SECTIONS.every((s) => !draft.strategy[s.id].trim())

  return (
    <div className="step step-strategy">
      <div className="step-title">
        <h2>Write the strategy</h2>
        <p>
          Three functions decide the book. Left is a complete, working strategy — a wallet mirror over raw RPC — split into the same three sections you fill in
          on the right. The code is hashed at launch and runs unchanged for the life of the basket.
        </p>
      </div>

      <AiBar onLoadExample={() => dispatch({ type: 'strategyLoad', value: EXAMPLE })} dirty={!empty} />

      <div className="strat-sections">
        {STRATEGY_SECTIONS.map((s, i) => {
          const errs = errorsFor(s.id)
          const src = draft.strategy[s.id]
          const state = !src.trim() ? null : errs.length ? 'bad' : 'ok'
          return (
            <section key={s.id} className="strat-section">
              <header className="strat-head">
                <span className="strat-index mono">0{i + 1}</span>
                <div className="strat-head-text">
                  <div className="strat-title">
                    {s.title} <span className="muted">— {s.ask}</span>
                  </div>
                  <code className="strat-sig">{s.signature}</code>
                </div>
              </header>

              <div className="strat-row">
                <div className="strat-example">
                  <CodeEditor
                    value={EXAMPLE[s.id]}
                    file={`example/${s.file}`}
                    minLines={6}
                    actions={
                      <button className="code-act wide" onClick={() => set(s.id)(EXAMPLE[s.id])} title="Copy this section into your editor">
                        Use <Icon.Chevron />
                      </button>
                    }
                  />
                </div>
                <div className="strat-editor">
                  <p className="strat-detail">{s.detail}</p>
                  <CodeEditor
                    value={src}
                    onChange={set(s.id)}
                    file={s.file}
                    minLines={6}
                    placeholder={`${s.signature.replace(/:.*$/, '')} {\n  \n}`}
                    status={
                      state === 'ok' ? (
                        <span className="strat-status ok">
                          <Icon.Check /> lint clean
                        </span>
                      ) : state === 'bad' ? (
                        <span className="strat-status bad">
                          {errs.length} problem{errs.length > 1 ? 's' : ''}
                        </span>
                      ) : null
                    }
                  />
                  {errs.length > 0 && (
                    <ul className="strat-errs">
                      {errs.map((e) => (
                        <li key={e}>{e}</li>
                      ))}
                    </ul>
                  )}
                </div>
              </div>
            </section>
          )
        })}
      </div>

      <p className="fine">
        Lint only checks for capabilities the sandbox does not grant. Whether the logic does what you meant is what the next step, Test, is for.
      </p>
    </div>
  )
}

function AiBar({ onLoadExample, dirty }) {
  const [copied, setCopied] = useState(false)
  const [confirm, setConfirm] = useState(false)

  const copySpec = async () => {
    try {
      const spec = await fetch(SPEC_URL).then((r) => r.text())
      const prompt = `${spec}\n\n---\n\nMy idea, in plain English:\n\n<describe the basket you want here>\n`
      await navigator.clipboard.writeText(prompt)
      setCopied(true)
      setTimeout(() => setCopied(false), 1600)
    } catch {}
  }

  const load = () => {
    if (dirty && !confirm) {
      setConfirm(true)
      setTimeout(() => setConfirm(false), 3000)
      return
    }
    setConfirm(false)
    onLoadExample()
  }

  return (
    <div className="ai-bar raised">
      <div className="ai-bar-text">
        <div className="ai-bar-title">Rather describe it than write it?</div>
        <p className="muted">
          Copy the authoring spec, paste it into Claude, Cursor or any capable model together with your idea in plain English, and paste the three functions it
          returns into the editors. The spec is written for the model: it defines the contract, the <code>ctx</code> API, the budgets and the exact reply
          format, so what comes back drops straight in.
        </p>
      </div>
      <div className="ai-bar-actions">
        <button className="btn btn-white" onClick={copySpec}>
          {copied ? <Icon.Check /> : <Icon.Copy />} {copied ? 'Copied with prompt' : 'Copy spec for AI'}
        </button>
        <a className="btn btn-ghost raised" href={SPEC_URL} target="_blank" rel="noreferrer">
          Read spec <Icon.Link />
        </a>
        <button className={`btn btn-ghost raised ${confirm ? 'confirm' : ''}`} onClick={load}>
          {confirm ? 'Replace your code?' : 'Load example'}
        </button>
      </div>
    </div>
  )
}
