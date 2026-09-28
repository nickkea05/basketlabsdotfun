import { useEffect } from 'react'
import { PHANTOM_INSTALL_URL } from '@basketfun/solana'
import { Icon } from '../../components/ui/index.js'
import { useWallet } from './useWallet.js'

export default function ConnectModal() {
  const { wallets, connect, connecting, error, closeConnect } = useWallet()

  useEffect(() => {
    const onKey = (e) => e.key === 'Escape' && closeConnect()
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [closeConnect])

  const none = wallets.length === 0

  return (
    <div className="wm-overlay" onMouseDown={(e) => e.target === e.currentTarget && closeConnect()}>
      <div className="wm raised" role="dialog" aria-modal="true" aria-label="Connect a wallet">
        <div className="wm-head">
          <div className="wm-title">
            Connect <span className="muted">a wallet</span>
          </div>
          <button className="icon-btn" onClick={closeConnect} aria-label="Close">
            <Icon.Close />
          </button>
        </div>

        {none ? (
          <div className="wm-none">
            <PhantomMark />
            <div>
              <div className="wm-none-title">No Solana wallet found</div>
              <p className="muted">Install Phantom, then come back and press Connect. Any Phantom wallet works, it does not need to hold SOL.</p>
            </div>
            <a className="btn btn-white" href={PHANTOM_INSTALL_URL} target="_blank" rel="noreferrer">
              Get Phantom <Icon.Link />
            </a>
          </div>
        ) : (
          <ul className="wm-list">
            {wallets.map((w) => {
              const busy = connecting === w.name
              return (
                <li key={w.name}>
                  <button className="wm-wallet" onClick={() => connect(w)} disabled={!!connecting}>
                    <img className="wm-icon" src={w.icon} alt="" />
                    <span className="wm-name">{w.name}</span>
                    <span className="wm-status muted">{busy ? 'Approve in wallet…' : 'Detected'}</span>
                    {busy ? <span className="spinner" /> : <Icon.Chevron />}
                  </button>
                </li>
              )
            })}
          </ul>
        )}

        {error && <div className="notice bad wm-err">{error}</div>}

        <p className="wm-fine">Connecting only shares your address. Nothing is signed until you launch or trade, and every signature is shown to you first.</p>
      </div>
    </div>
  )
}

function PhantomMark() {
  return (
    <div className="wm-phantom" aria-hidden="true">
      <svg width="26" height="26" viewBox="0 0 24 24" fill="none">
        <path
          d="M12 2.5c-5.2 0-9 3.9-9 9.2 0 1.2.2 2.4.6 3.5.3.9 1.3 1.3 2.1.9.6-.3 1.2-.4 1.6.2.5.8 1.3 1.3 2.2 1.3.9 0 1.7-.5 2.2-1.3.4-.6 1-.5 1.6-.2.8.4 1.8 0 2.1-.9.4-1.1.6-2.3.6-3.5 0-5.3-3.8-9.2-9-9.2h5z"
          fill="currentColor"
        />
        <circle cx="9" cy="11" r="1.3" fill="#111116" />
        <circle cx="14.5" cy="11" r="1.3" fill="#111116" />
      </svg>
    </div>
  )
}
