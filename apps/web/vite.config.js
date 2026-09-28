import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// Dev proxy for Jupiter's token/holdings API. In production this becomes a
// route on our backend (same paths), so the frontend code never changes.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api/jup': {
        target: 'https://lite-api.jup.ag',
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/api\/jup/, ''),
      },
      // GeckoTerminal: free OHLCV history for any Solana pool.
      '/api/gt': {
        target: 'https://api.geckoterminal.com/api/v2',
        changeOrigin: true,
        rewrite: (p) => p.replace(/^\/api\/gt/, ''),
      },
      // Solana JSON-RPC for strategy test runs. Public mainnet endpoint in
      // dev; a keyed provider behind the same path in production.
      '/api/rpc': {
        target: 'https://api.mainnet-beta.solana.com',
        changeOrigin: true,
        rewrite: () => '/',
        // The public endpoint 403s any request that carries a browser Origin.
        configure: (proxy) =>
          proxy.on('proxyReq', (req) => {
            req.removeHeader('origin')
            req.removeHeader('referer')
          }),
      },
    },
  },
})
