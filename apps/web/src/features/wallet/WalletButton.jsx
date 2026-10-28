import { useEffect, useRef, useState } from 'react'
import { shortKey } from '@basketfun/solana'
import { Icon } from '../../components/ui/index.js'
import { IS_FORK, explorerAddressUrl, fundTestWallet } from '../../lib/chain.js'
import { useWallet } from './useWallet.js'
import { useCopy } from './useCopy.js'
import { useSolBalance } from './useSolBalance.js'

const fmtSol = (lamports) => (lamports == null ? '–' : (lamports / 1e9).toLocaleString(undefined, { maximumFractionDigits: 3 }))

/** Nav control: "Connect" when idle, an address pill with a small menu once connected. */
export default function WalletButton({ go }) {
  const { connected, address, wallet, openConnect, disconnect } = useWallet()
  const [open, setOpen] = useState(false)
  const ref = useRef(null)
  const [copied, copy] = useCopy(address)
  const [lamports, refreshBalance] = useSolBalance(connected ? address : null)
  const [funding, setFunding] = useState(null) // null | 'busy' | 'done' | error message

  const getTestFunds = async () => {
    setFunding('busy')
    try {
      await fundTestWallet(address)
      await refreshBalance()
      setFunding('done')
    } catch (e) {
      setFunding(e.message || 'Could not reach the local fork.')
    }
  }

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
            <div className="wallet-menu-balance mono">
              {fmtSol(lamports)} SOL{IS_FORK && <span className="wallet-fork-tag">local fork</span>}
            </div>
          </div>
          {IS_FORK && (
            <>
              <button role="menuitem" onClick={getTestFunds} disabled={funding === 'busy'}>
                <Icon.Wallet /> {funding === 'busy' ? 'Funding…' : funding === 'done' ? 'Funded: 100 SOL + test tokens' : 'Get test funds'}
              </button>
              {funding && funding !== 'busy' && funding !== 'done' && <div className="wallet-menu-error">{funding}</div>}
            </>
          )}
          <button role="menuitem" onClick={copy}>
            <Icon.Copy /> {copied ? 'Copied' : 'Copy address'}
          </button>
          <a role="menuitem" href={explorerAddressUrl(address)} target="_blank" rel="noreferrer">
            <Icon.Link /> {IS_FORK ? 'View in explorer (fork)' : 'View on Solscan'}
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
