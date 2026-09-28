export default function Footer({ go }) {
  return (
    <footer className="panel footer">
      <div className="footer-grid">
        <div>
          <div className="footer-brand">
            basketlabs<span>.fun</span>
          </div>
          <p>Launch and explore tokens pinned to baskets of Solana assets. Your wallet submits every transaction. basketlabs.fun does not custody assets.</p>
        </div>
        <div>
          <div className="footer-label">Product</div>
          <ul>
            <li>
              <button onClick={() => go('explore')}>Explore</button>
            </li>
            <li>
              <button onClick={() => go('leaderboard')}>Leaderboard</button>
            </li>
            <li>
              <button onClick={() => go('portfolio')}>Portfolio</button>
            </li>
            <li>
              <button onClick={() => go('create')}>Launch</button>
            </li>
            <li>
              <button onClick={() => go('how')}>How it works</button>
            </li>
          </ul>
        </div>
        <div>
          <div className="footer-label">Legal</div>
          <ul>
            <li>
              <button>Privacy Policy</button>
            </li>
            <li>
              <button>Terms of Use</button>
            </li>
          </ul>
        </div>
        <div>
          <div className="footer-label">Risk notice</div>
          <p>
            Transactions are submitted through your wallet and may be irreversible. Tokens can be volatile or lose all value. basketlabs.fun does not provide
            custody, warranties, or financial advice.
          </p>
        </div>
      </div>
      <div className="footer-bottom">
        <span>© 2026 basketlabs.fun</span>
        <span className="footer-social">
          <button>@basketlabsfun</button>
          <button className="icon-btn sq" aria-label="X">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor">
              <path d="M18.9 2H22l-7.6 8.7L23 22h-6.8l-5.3-6.9L4.8 22H1.7l8.1-9.3L1 2h7l4.8 6.3L18.9 2Zm-1.2 18h1.9L7.4 3.9H5.4L17.7 20Z" />
            </svg>
          </button>
        </span>
      </div>
    </footer>
  )
}
