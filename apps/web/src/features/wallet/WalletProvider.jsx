import { useCallback, useEffect, useRef, useState } from 'react'
import { getWallets } from '@wallet-standard/app'
import { PublicKey, VersionedTransaction } from '@solana/web3.js'
import { base58Encode, rankWallets } from '@basketfun/solana'
import { buildTransaction, confirmSignature } from '@basketfun/solana/basket'
import { IS_FORK, WALLET_CHAIN, getConnection } from '../../lib/chain.js'
import { WalletContext } from './context.js'
import ConnectModal from './ConnectModal.jsx'
import './wallet.css'

// Wallet Standard, directly. Phantom (and every other Solana wallet) registers
// itself on the page; we list what is there, connect to one, and remember the
// choice so the next visit reconnects silently. `sendInstructions` is the one
// path that asks the wallet to sign.

const SOLANA_CHAIN = /^solana:/
const solanaAccount = (accounts) => (accounts ?? []).find((a) => a.chains?.some((c) => SOLANA_CHAIN.test(c)))

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
  const [account, setAccount] = useState(null) // its Solana WalletAccount
  const address = account?.address ?? null
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
    setAccount(null)
    remember(null)
  }, [])

  const adopt = useCallback((w, accounts) => {
    const acc = solanaAccount(accounts)
    if (!acc) throw new Error('The wallet did not share a Solana account.')
    setWallet(w)
    setAccount(acc)
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
      const acc = solanaAccount(accounts)
      if (acc) setAccount(acc)
      else clear()
    })
  }, [wallet, clear])

  /**
   * Build, sign and land one transaction from the connected account.
   * Mainnet: the wallet signs and sends (what Phantom asks dapps to use).
   * Fork: the wallet only signs, since it cannot reach the local RPC, and
   * we submit. Resolves with the signature once confirmed.
   */
  const sendInstructions = useCallback(
    async (instructions, { lookupTables = [], cu } = {}) => {
      if (!wallet || !account) throw new Error('Connect a wallet first.')
      const connection = getConnection()
      const { transaction, lastValidBlockHeight } = await buildTransaction(connection, {
        payer: new PublicKey(account.address),
        instructions,
        lookupTables,
        cu,
      })
      const bytes = transaction.serialize()
      const chain = WALLET_CHAIN ? { chain: WALLET_CHAIN } : {}
      const signAndSend = wallet.features['solana:signAndSendTransaction']
      let signature
      try {
        if (!IS_FORK && signAndSend) {
          const [out] = await signAndSend.signAndSendTransaction({ account, transaction: bytes, ...chain })
          signature = base58Encode(out.signature)
        } else {
          const sign = wallet.features['solana:signTransaction']
          if (!sign) throw new Error(`${wallet.name} cannot sign transactions.`)
          const [out] = await sign.signTransaction({ account, transaction: bytes, ...chain })
          const signed = VersionedTransaction.deserialize(out.signedTransaction)
          signature = await connection.sendRawTransaction(signed.serialize(), { skipPreflight: false })
        }
      } catch (e) {
        throw new Error(friendly(e))
      }
      await confirmSignature(connection, signature, lastValidBlockHeight)
      return signature
    },
    [wallet, account]
  )

  const value = {
    wallets,
    wallet,
    address,
    connected: !!address,
    connecting,
    error,
    connect,
    disconnect,
    sendInstructions,
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
  return m || 'Wallet request failed.'
}
