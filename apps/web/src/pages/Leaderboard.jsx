import { useState } from 'react'
import Footer from '../components/layout/Footer.jsx'

// Creator leaderboard. Structure only: the ranking, the stats and the score
// breakdown are all computed by the indexer once baskets exist on-chain.
// Nothing here is sample data; every value is intentionally empty.

const RANGES = ['7d', '30d', 'All']

const HEADLINE = [
  ['Creators ranked', 'With at least one basket past its curve'],
  ['Baskets launched', 'Across all ranked creators'],
  ['Avg. since launch', 'Mean return vs the $1,000 open'],
  ['Holder PnL', 'Everyone who bought a ranked basket'],
]

const COLUMNS = ['#', 'Creator', 'Baskets', 'Avg. peak', 'Since launch', 'Holder PnL', 'Score']

// What goes into a creator's score. Weights are decided once there is data to
// calibrate against; until then they are shown as pending.
const METRICS = [
  {
    name: 'Holder PnL',
    detail:
      'Realised and unrealised return of everyone who bought the creator’s baskets, weighted by position size. The number that matters most: did followers make money.',
  },
  {
    name: 'Average peak',
    detail: 'Mean of each basket’s highest gain from its $1,000 open. Rewards creators whose launches run, not just hold.',
  },
  {
    name: 'Since launch',
    detail: 'Mean current return across every basket the creator has launched. Penalises pump-and-fade patterns.',
  },
  {
    name: 'Consistency',
    detail: 'Share of the creator’s baskets trading above their open. One good launch does not carry ten bad ones.',
  },
  {
    name: 'Turnover discipline',
    detail: 'Managed baskets only: how often contents change and how much each change cost holders in fees and slippage.',
  },
]

export default function Leaderboard({ go }) {
  const [range, setRange] = useState('30d')

  return (
    <div className="page container">
      <section className="panel section lb-hero">
        <div className="section-head">
          <h2>Leaderboard</h2>
          <span className="count">Creators</span>
          <div className="sort">
            {RANGES.map((r) => (
              <button key={r} className={range === r ? 'active' : ''} onClick={() => setRange(r)}>
                {r}
              </button>
            ))}
          </div>
        </div>
        <p className="section-sub">
          Creators ranked by how their baskets did for the people who bought them. Back a track record instead of auditing every asset yourself.
        </p>
        <div className="stats lb-stats">
          {HEADLINE.map(([label, sub]) => (
            <div key={label} className="stat">
              <div className="stat-label">{label}</div>
              <div className="stat-value lb-pending">—</div>
              <div className="stat-sub">{sub}</div>
            </div>
          ))}
        </div>
      </section>

      <div className="two-col">
        <section className="panel lb-board">
          <div className="lb-cols">
            {COLUMNS.map((c) => (
              <span key={c}>{c}</span>
            ))}
          </div>
          <div className="empty">
            <strong>No creators ranked yet</strong>
            The board fills in as baskets launch, trade and graduate. Rankings need a track record, so the first entries appear after the first launches have
            some history behind them.
            <div>
              <button className="btn btn-ghost" onClick={() => go('create')}>
                Launch a basket
              </button>
            </div>
          </div>
        </section>

        <aside className="side">
          <section className="panel basket">
            <h3>How the score works</h3>
            <p className="basket-sub">Five inputs, one number. Weights are set once there is live data to calibrate against.</p>
            <div className="metric-list">
              {METRICS.map((m, i) => (
                <div key={m.name} className="metric">
                  <span className="metric-n">{i + 1}</span>
                  <div>
                    <strong>{m.name}</strong>
                    <p>{m.detail}</p>
                  </div>
                  <span className="weight">weight —</span>
                </div>
              ))}
            </div>
          </section>

          <section className="panel basket">
            <h3>Rising</h3>
            <p className="basket-sub">Creators whose latest launch is beating their own track record.</p>
            <div className="empty lb-mini">
              <strong>Nothing to show yet</strong>
              Needs at least two launches per creator.
            </div>
          </section>
        </aside>
      </div>

      <Footer go={go} />
    </div>
  )
}
