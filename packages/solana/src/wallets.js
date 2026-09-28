// Wallet Standard helpers. Pure functions over the wallet/account objects the
// standard exposes; nothing here touches the DOM.

const SOLANA_CHAIN = /^solana:/

export const PHANTOM_INSTALL_URL = 'https://phantom.com/download'

const isSolanaWallet = (w) => Array.isArray(w?.chains) && w.chains.some((c) => SOLANA_CHAIN.test(c)) && !!w.features?.['standard:connect']

/**
 * Wallets worth offering: Solana-capable, connectable, one entry per name.
 * Phantom first because that is what nearly everyone has; the rest by name so
 * the list is stable between page loads.
 */
export function rankWallets(wallets) {
  const seen = new Set()
  const out = []
  for (const w of wallets ?? []) {
    if (!isSolanaWallet(w) || seen.has(w.name)) continue
    seen.add(w.name)
    out.push(w)
  }
  return out.sort((a, b) => {
    if (a.name === 'Phantom') return -1
    if (b.name === 'Phantom') return 1
    return a.name.localeCompare(b.name)
  })
}

/** Addresses from a connect() result that can be used on Solana. */
export const solanaAddresses = (accounts) => (accounts ?? []).filter((a) => a.chains?.some((c) => SOLANA_CHAIN.test(c))).map((a) => a.address)

export const explorerUrl = (address, cluster = 'mainnet') => `https://solscan.io/account/${address}${cluster === 'mainnet' ? '' : `?cluster=${cluster}`}`
