import { useEffect, useRef, useState } from 'react'
import { explorerUrl, shortKey } from '@basketfun/solana'
import { Icon } from '../../components/ui/index.js'
import { useWallet } from './useWallet.js'
import { useCopy } from './useCopy.js'

/** Nav control: "Connect" when idle, an address pill with a small menu once connected. */
export default function WalletButton({ go }) {
  const { connected, address, wallet, openConnect, disconnect } = useWallet()
  const [open, setOpen] = useState(false)
  const ref = useRef(null)
  const [copied, copy] = useCopy(address)

  useEffect(() => {
    if (!open) return
    const close = (e) => ref.current && !ref.current.contains(e.target) && setOpen(false)
    const onKey = (e) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('mousedown', close)
    document.addEventListener('keydown', onKey)
    return () => {
      document.removeEventListener('mousedown', close)
      document.removeEventListener('keydown', onKey)
    }
  }, [open])

  if (!connected) {
    return (
      <button className="btn btn-accent" onClick={openConnect}>
        Connect
      </button>
    )
  }

  return (
    <div className="wallet-anchor" ref={ref}>
      <button className={`wallet-pill raised ${open ? 'open' : ''}`} onClick={() => setOpen((o) => !o)} aria-haspopup="menu" aria-expanded={open}>
        {wallet?.icon && <img className="wallet-pill-icon" src={wallet.icon} alt="" />}
        <span className="mono">{shortKey(address)}</span>
        <Icon.Chevron />
      </button>

      {open && (
        <div className="wallet-menu raised" role="menu">
          <div className="wallet-menu-head">
            <div className="wallet-menu-label">{wallet?.name ?? 'Wallet'}</div>
            <div className="wallet-menu-addr mono" title={address}>
              {shortKey(address, 8)}
            </div>
          </div>
          <button role="menuitem" onClick={copy}>
            <Icon.Copy /> {copied ? 'Copied' : 'Copy address'}
          </button>
          <a role="menuitem" href={explorerUrl(address)} target="_blank" rel="noreferrer">
            <Icon.Link /> View on Solscan
          </a>
          <button
            role="menuitem"
            onClick={() => {
              setOpen(false)
              go('portfolio')
            }}
          >
            <Icon.Wallet /> Portfolio
          </button>
          <div className="wallet-menu-sep" />
          <button
            role="menuitem"
            className="danger"
            onClick={() => {
              setOpen(false)
              disconnect()
            }}
          >
            <Icon.Power /> Disconnect
          </button>
        </div>
      )}
    </div>
  )
}
