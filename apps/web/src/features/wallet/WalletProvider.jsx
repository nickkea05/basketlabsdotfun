import { useCallback, useEffect, useRef, useState } from 'react'
import { getWallets } from '@wallet-standard/app'
import { rankWallets, solanaAddresses } from '@basketfun/solana'
import { WalletContext } from './context.js'
import ConnectModal from './ConnectModal.jsx'
import './wallet.css'

// Wallet Standard, directly. Phantom (and every other Solana wallet) registers
// itself on the page; we list what is there, connect to one, and remember the
// choice so the next visit reconnects silently. Nothing here signs anything.

const STORAGE_KEY = 'basketfun.wallet'

// getWallets() returns a page-wide singleton, so module scope is the right home.
const registry = getWallets()

const remember = (name) => {
  try {
    if (name) localStorage.setItem(STORAGE_KEY, name)
    else localStorage.removeItem(STORAGE_KEY)
  } catch {}
}
const remembered = () => {
  try {
    return localStorage.getItem(STORAGE_KEY)
  } catch {
    return null
  }
}

export function WalletProvider({ children }) {
  const [wallets, setWallets] = useState(() => rankWallets(registry.get()))
  const [wallet, setWallet] = useState(null) // the connected Wallet Standard object
  const [address, setAddress] = useState(null)
  const [connecting, setConnecting] = useState(null) // wallet name while a connect() is pending
  const [error, setError] = useState(null)
  const [modalOpen, setModalOpen] = useState(false)

  // Wallets register asynchronously, sometimes after first paint.
  useEffect(() => {
    const refresh = () => setWallets(rankWallets(registry.get()))
    const offs = [registry.on('register', refresh), registry.on('unregister', refresh)]
    return () => offs.forEach((off) => off())
  }, [])

  const clear = useCallback(() => {
    setWallet(null)
    setAddress(null)
    remember(null)
  }, [])

  const adopt = useCallback((w, accounts) => {
    const [addr] = solanaAddresses(accounts)
    if (!addr) throw new Error('The wallet did not share a Solana account.')
    setWallet(w)
    setAddress(addr)
    setModalOpen(false)
    remember(w.name)
  }, [])

  const connect = useCallback(
    async (w) => {
      setConnecting(w.name)
      setError(null)
      try {
        const { accounts } = await w.features['standard:connect'].connect()
        adopt(w, accounts)
      } catch (e) {
        setError(friendly(e))
      } finally {
        setConnecting(null)
      }
    },
    [adopt]
  )

  const disconnect = useCallback(async () => {
    const w = wallet
    clear()
    try {
      await w?.features['standard:disconnect']?.disconnect()
    } catch {}
  }, [wallet, clear])

  // Silent reconnect to the wallet used last time, once it has registered.
  // Everything here is async, so no state is touched during the effect itself.
  const triedAuto = useRef(false)
  useEffect(() => {
    if (triedAuto.current || address) return
    const name = remembered()
    if (!name) {
      triedAuto.current = true
      return
    }
    const w = wallets.find((x) => x.name === name)
    if (!w) return // not registered yet; try again when the list changes
    triedAuto.current = true
    w.features['standard:connect']
      .connect({ silent: true })
      .then(({ accounts }) => adopt(w, accounts))
      .catch(() => remember(null))
  }, [wallets, address, adopt])

  // Follow account switches and in-wallet disconnects.
  useEffect(() => {
    if (!wallet) return
    const events = wallet.features['standard:events']
    if (!events) return
    return events.on('change', ({ accounts }) => {
      if (!accounts) return
      const [addr] = solanaAddresses(accounts)
      if (addr) setAddress(addr)
      else clear()
    })
  }, [wallet, clear])

  const value = {
    wallets,
    wallet,
    address,
    connected: !!address,
    connecting,
    error,
    connect,
    disconnect,
    openConnect: () => {
      setError(null)
      setModalOpen(true)
    },
    closeConnect: () => setModalOpen(false),
  }

  return (
    <WalletContext.Provider value={value}>
      {children}
      {modalOpen && <ConnectModal />}
    </WalletContext.Provider>
  )
}

function friendly(e) {
  const m = String(e?.message || '')
  if (/reject|denied|cancel/i.test(m)) return 'Request was rejected in the wallet.'
  return m || 'Could not connect.'
}
