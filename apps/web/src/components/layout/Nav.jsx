import WalletButton from '../../features/wallet/WalletButton.jsx'
import { Mark } from '../ui/index.js'

export default function Nav({ route, go }) {
  const tabs = [
    { id: 'explore', label: 'Explore' },
    { id: 'leaderboard', label: 'Leaderboard' },
    { id: 'portfolio', label: 'Portfolio' },
  ]
  return (
    <header className="nav">
      <div className="container nav-inner">
        <button className="brand" onClick={() => go('explore')}>
          <Mark size={36} />
          basketlabs<span>.fun</span>
        </button>

        <nav className="tabs">
          {tabs.map((t) => (
            <button key={t.id} className={`tab ${route === t.id ? 'active' : ''}`} onClick={() => go(t.id)}>
              {t.label}
            </button>
          ))}
        </nav>

        <div className="nav-right">
          <WalletButton go={go} />
        </div>
      </div>
    </header>
  )
}
