import { explorerUrl, shortKey } from '@basketfun/solana'
import { Icon } from '../../components/ui/index.js'
import { useWallet } from './useWallet.js'
import { useCopy } from './useCopy.js'

/** Quiet one-line header for the portfolio: which wallet is attached, copy it, open it, drop it. */
export default function WalletStrip() {
  const { address, wallet, disconnect } = useWallet()
  const [copied, copy] = useCopy(address)
  if (!address) return null

  return (
    <div className="wallet-strip">
      <div className="wallet-strip-id">
        {wallet?.icon ? <img className="wallet-strip-icon" src={wallet.icon} alt="" /> : <span className="wallet-strip-dot" />}
        <span className="wallet-strip-name">{wallet?.name ?? 'Wallet'}</span>
        <span className="wallet-strip-addr mono" title={address}>
          {shortKey(address, 6)}
        </span>
        <button
          className={`icon-btn sq wallet-strip-btn ${copied ? 'ok' : ''}`}
          onClick={copy}
          aria-label="Copy address"
          title={copied ? 'Copied' : 'Copy address'}
        >
          {copied ? <Icon.Check /> : <Icon.Copy />}
        </button>
        <a
          className="icon-btn sq wallet-strip-btn"
          href={explorerUrl(address)}
          target="_blank"
          rel="noreferrer"
          aria-label="View on Solscan"
          title="View on Solscan"
        >
          <Icon.Link />
        </a>
      </div>
      <button className="wallet-strip-off" onClick={disconnect}>
        Disconnect
      </button>
    </div>
  )
}
