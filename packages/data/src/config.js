// Base URLs for upstream data APIs. Defaults point at the app's own proxy
// routes (Vite dev proxy today, backend routes in production) so the browser
// never talks to third parties directly. Override for Node scripts/tests.

const cfg = {
  jupiter: '/api/jup',
  geckoterminal: '/api/gt',
}

export function configure(overrides) {
  Object.assign(cfg, overrides)
}

export const baseUrl = (key) => cfg[key]
