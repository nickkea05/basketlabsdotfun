// Which chain the site talks to. Dev defaults to the local Surfpool mainnet
// fork (proxied at /api/chain); production builds default to mainnet.
// Override with VITE_CLUSTER=localnet|mainnet.

import { Connection, LAMPORTS_PER_SOL } from '@solana/web3.js'

export const CLUSTER = import.meta.env.VITE_CLUSTER ?? (import.meta.env.DEV ? 'localnet' : 'mainnet')
export const IS_FORK = CLUSTER === 'localnet'

/** Wallet Standard chain id; the fork has none a wallet would recognise. */
export const WALLET_CHAIN = IS_FORK ? null : 'solana:mainnet'

const RPC_PATH = IS_FORK ? '/api/chain' : '/api/rpc'
export const RPC_URL = new URL(RPC_PATH, globalThis.location?.origin ?? 'http://localhost').toString()

let connection
export const getConnection = () => (connection ??= new Connection(RPC_URL, 'confirmed'))

export function explorerAddressUrl(address) {
  if (!IS_FORK) return `https://solscan.io/account/${address}`
  return `https://explorer.solana.com/address/${address}?cluster=custom&customUrl=${encodeURIComponent('http://127.0.0.1:8899')}`
}

export function explorerTxUrl(signature) {
  if (!IS_FORK) return `https://solscan.io/tx/${signature}`
  return `https://explorer.solana.com/tx/${signature}?cluster=custom&customUrl=${encodeURIComponent('http://127.0.0.1:8899')}`
}

// ---- fork only: test funds through Surfpool cheatcodes ----

/** Tokens a test wallet gets, in whole units. Real mainnet mints. */
export const TEST_TOKENS = [
  { symbol: 'JUP', mint: 'JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN', decimals: 6, amount: 5_000 },
  { symbol: 'BONK', mint: 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263', decimals: 5, amount: 500_000_000 },
  { symbol: 'JitoSOL', mint: 'J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn', decimals: 9, amount: 20 },
  { symbol: 'WIF', mint: 'EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm', decimals: 6, amount: 5_000 },
  { symbol: 'mSOL', mint: 'mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So', decimals: 9, amount: 20 },
  { symbol: 'PYTH', mint: 'HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3', decimals: 6, amount: 20_000 },
  { symbol: 'JTO', mint: 'jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL', decimals: 9, amount: 1_000 },
  { symbol: 'RAY', mint: '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R', decimals: 6, amount: 1_000 },
  { symbol: 'ORCA', mint: 'orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE', decimals: 6, amount: 1_000 },
]
export const TEST_SOL = 100

const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'

async function cheat(method, params) {
  const res = await fetch(RPC_URL, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  })
  const body = await res.json()
  if (body.error) throw new Error(body.error.message ?? `${method} failed`)
  return body.result
}

/** Top `address` up to TEST_SOL and every TEST_TOKENS balance. Fork only. */
export async function fundTestWallet(address) {
  if (!IS_FORK) throw new Error('test funds exist only on the local fork')
  await cheat('surfnet_setAccount', [address, { lamports: TEST_SOL * LAMPORTS_PER_SOL }])
  for (const t of TEST_TOKENS) {
    const amount = BigInt(t.amount) * 10n ** BigInt(t.decimals)
    await cheat('surfnet_setTokenAccount', [address, t.mint, { amount: Number(amount) }, TOKEN_PROGRAM])
  }
}
