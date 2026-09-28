import { CopyButton } from '../../../components/ui/index.js'

export default function ResultStep({ draft }) {
  const r = draft.result
  const json = JSON.stringify(r.params, null, 2)
  const dry = r.status === 'DRY_RUN'

  return (
    <div className="step step-result">
      <div className="step-title">
        <h2>{dry ? 'Dry run complete' : r.status === 'REJECTED' ? 'Rejected before send' : 'Launched'}</h2>
        <p>{r.message ?? (r.errors ? r.errors.join(' · ') : '')}</p>
      </div>

      <div className="result-status">
        <span className={`pill ${dry ? 'dry' : r.ok ? 'ok' : 'bad'}`}>
          <i />
          {r.status}
          {r.reason && <span className="muted"> · {r.reason}</span>}
        </span>
        <span className="muted">
          Instruction: <span className="mono">create_basket</span>
        </span>
        <CopyButton text={json} label="Copy payload" />
      </div>

      <pre className="payload inset">
        <code>{json}</code>
      </pre>

      <div className="fine">
        Field names match the Rust <span className="mono">CreateBasketArgs</span> struct. <span className="mono">creator</span> is filled by the wallet adapter
        at signing; <span className="mono">uri</span> after metadata upload.
      </div>
    </div>
  )
}
