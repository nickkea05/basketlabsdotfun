import Footer from '../components/layout/Footer.jsx'
import WalletStrip from '../features/wallet/WalletStrip.jsx'
import { useWallet } from '../features/wallet/useWallet.js'

export default function Portfolio({ go }) {
  const { connected, openConnect } = useWallet()

  return (
    <div className="page container">
      <WalletStrip />

      <section className="panel section">
        <div className="section-head">
          <h2>Portfolio</h2>
        </div>
        <p className="section-sub">Baskets you hold, baskets you've launched, and what they're pinned to.</p>

        {connected ? (
          // Holdings come from the indexer: this wallet's token accounts
          // intersected with the share mints our program has created. Nothing
          // to intersect against until the program exists.
          <div className="empty">
            <strong>No baskets in this wallet yet</strong>
            Anything launched or bought on basketlabs.fun shows up here on its own. Other tokens stay out of the way.
            <div style={{ display: 'flex', gap: 8, justifyContent: 'center' }}>
              <button className="btn btn-ghost raised" onClick={() => go('explore')}>
                Explore baskets
              </button>
              <button className="btn btn-white" onClick={() => go('create')}>
                + Launch
              </button>
            </div>
          </div>
        ) : (
          <div className="empty">
            <strong>No wallet connected</strong>
            Connect to see your positions and creator earnings.
            <div>
              <button className="btn btn-accent" onClick={openConnect}>
                Connect
              </button>
            </div>
          </div>
        )}
      </section>

      <div className="two-col" style={{ gridTemplateColumns: '1fr 1fr 1fr' }}>
        {[
          ['Holdings', '—', 'Total value across baskets'],
          ['Creator earnings', '—', 'Claimable fees from your launches'],
          ['Launched', '—', 'Baskets you created'],
        ].map(([label, value, sub]) => (
          <section key={label} className="panel kpi">
            <div className="stat-label">{label}</div>
            <div className="price">{value}</div>
            <div className="basket-sub">{sub}</div>
          </section>
        ))}
      </div>

      <Footer go={go} />
    </div>
  )
}
