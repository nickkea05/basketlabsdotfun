import { useState } from 'react'
import { isValidPubkey, shortKey } from '@basketfun/solana'
import { walletHoldings } from '@basketfun/data'
import { HostWeighting, MAX_HOSTS, combineHosts } from '@basketfun/core'
import { Icon, Logo } from '../../../components/ui/index.js'
import { fmtCompact, fmtPct } from '../../../lib/format.js'

export default function WalletStep({ draft, dispatch }) {
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState(null)
  const addr = draft.hostWallet.trim()
  const valid = isValidPubkey(addr)
  const dupe = draft.hosts.some((h) => h.address === addr)
  const full = draft.hosts.length >= MAX_HOSTS
  const many = draft.hosts.length > 1

  const add = async () => {
    if (!valid || dupe || full || busy) return
    setBusy(true)
    setErr(null)
    try {
      const s = await walletHoldings(addr)
      if (s.rows.length === 0) throw new Error('That wallet holds nothing we can price. Nothing to mirror.')
      dispatch({ type: 'addHost', address: addr, snapshot: s })
    } catch (e) {
      setErr(e.message || 'Could not read that wallet.')
    } finally {
      setBusy(false)
    }
  }

  const combined = combineHosts(draft.hosts, draft.hostWeighting)

  return (
    <div className="step step-wallet">
      <div className="step-title">
        <h2>{many ? 'Which wallets does it mirror?' : 'Which wallet does it mirror?'}</h2>
        <p>Add up to {MAX_HOSTS} wallets to track. The basket holds what they hold, in the same proportions.</p>
      </div>

      <div className="wallet-row">
        <div className={`input ${addr && (!valid || dupe) ? 'bad' : ''}`}>
          <input
            autoFocus
            spellCheck={false}
            placeholder={draft.hosts.length ? 'Add another wallet' : 'Host wallet address'}
            value={draft.hostWallet}
            onChange={(e) => dispatch({ type: 'hostWallet', value: e.target.value })}
            onKeyDown={(e) => e.key === 'Enter' && add()}
            disabled={full}
          />
          {addr && <span className={`mini ${valid && !dupe ? 'ok' : 'bad'}`}>{dupe ? 'Already added' : valid ? 'Valid key' : 'Not a Solana address'}</span>}
        </div>
        <button className="btn btn-white tall" disabled={!valid || dupe || full || busy} onClick={add}>
          {busy ? 'Reading…' : draft.hosts.length ? '+ Add wallet' : 'Check wallet'}
        </button>
      </div>
      {err && <div className="notice bad">{err}</div>}
      {full && <div className="fine">Index is at the {MAX_HOSTS}-wallet limit.</div>}

      {draft.hosts.length > 0 && (
        <div className="hosts-grid">
          <div className="hosts-list">
            <div className="hosts-label">
              Hosts <span className="mono">{draft.hosts.length}</span>
              {many && (
                <div className="seg hosts-seg" title="How the wallets are combined">
                  <button
                    className={draft.hostWeighting === HostWeighting.Equal ? 'active' : ''}
                    onClick={() => dispatch({ type: 'hostWeighting', value: HostWeighting.Equal })}
                  >
                    Equal per wallet
                  </button>
                  <button
                    className={draft.hostWeighting === HostWeighting.Value ? 'active' : ''}
                    onClick={() => dispatch({ type: 'hostWeighting', value: HostWeighting.Value })}
                  >
                    By value
                  </button>
                </div>
              )}
            </div>
            {draft.hosts.map((h, i) => (
              <div key={h.address} className="host raised">
                <span className="host-n mono">{String(i + 1).padStart(2, '0')}</span>
                <div className="host-id">
                  <div className="host-addr mono">{shortKey(h.address, 6)}</div>
                  <div className="muted host-meta">
                    {h.snapshot.rows.length} positions · {fmtCompact(h.snapshot.totalUsd)}
                  </div>
                </div>
                <div className="host-logos">
                  {h.snapshot.rows.slice(0, 5).map((r) => (
                    <Logo key={r.token.mint} token={r.token} size={18} />
                  ))}
                  {h.snapshot.rows.length > 5 && <span className="muted">+{h.snapshot.rows.length - 5}</span>}
                </div>
                <button className="icon-btn host-x" onClick={() => dispatch({ type: 'removeHost', address: h.address })} aria-label="Remove wallet">
                  <Icon.Close />
                </button>
              </div>
            ))}
            {many && (
              <div className="fine">
                {draft.hostWeighting === HostWeighting.Equal
                  ? 'Each wallet\u2019s book counts the same, so one whale can\u2019t become the whole index.'
                  : 'Every wallet\u2019s dollars are pooled, so bigger wallets weigh more.'}
              </div>
            )}
          </div>

          <div className="snapshot panel">
            <div className="snap-head">
              <div>
                <div className="snap-label">{many ? 'Combined book' : 'Connected'}</div>
                <div className="snap-addr mono">{many ? `${draft.hosts.length} wallets` : shortKey(draft.hosts[0].address, 6)}</div>
              </div>
              <div className="snap-kpis">
                <div>
                  <div className="snap-label">Assets</div>
                  <div className="snap-val">{combined.assets.length}</div>
                </div>
                <div>
                  <div className="snap-label">Tracked value</div>
                  <div className="snap-val">{fmtCompact(draft.hosts.reduce((s, h) => s + h.snapshot.totalUsd, 0))}</div>
                </div>
              </div>
            </div>
            <div className="snap-list">
              {combined.assets.map((t) => (
                <div key={t.mint} className="snap-row">
                  <Logo token={t} size={26} />
                  <span className="snap-sym">{t.symbol}</span>
                  <span className="muted snap-name">{t.name}</span>
                  <span className="muted mono snap-holders">
                    {many ? `${draft.hosts.filter((h) => h.snapshot.rows.some((r) => r.token.mint === t.mint)).length}/${draft.hosts.length}` : ''}
                  </span>
                  <span className="mono snap-pct">{fmtPct(combined.weights[t.mint])}</span>
                </div>
              ))}
            </div>
            <div className="fine">
              Every priced position across all wallets is included; positions under $1 are ignored.{' '}
              {draft.hosts.some((h) => h.snapshot.truncated) && 'Some wallets have more than 300 token accounts; only the first 300 were priced.'}
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
