// Shared plumbing for the Surfpool simulation: RPC, sim keypairs, cheatcodes,
// the fork clock, real prices and a v0 sender with an address lookup table.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  AddressLookupTableProgram,
  Connection,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  TransactionMessage,
  VersionedTransaction,
} from '@solana/web3.js'
import { computeBudgetIxs, TOKEN_PROGRAM_ID } from '@basketfun/solana/basket'

const here = dirname(fileURLToPath(import.meta.url))
const KEYS_DIR = join(here, '.keys')

export const RPC_URL = process.env.SIM_RPC_URL ?? 'http://127.0.0.1:8899'
export const connection = new Connection(RPC_URL, 'confirmed')

export const readKeypair = (path) => Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(path, 'utf8'))))
export const adminKeypair = () => readKeypair(join(homedir(), '.config/solana/id.json'))

/** A named keypair under sim/.keys, created on first use. */
export function simKey(name) {
  const path = join(KEYS_DIR, `${name}.json`)
  if (existsSync(path)) return readKeypair(path)
  mkdirSync(KEYS_DIR, { recursive: true })
  const kp = Keypair.generate()
  writeFileSync(path, JSON.stringify(Array.from(kp.secretKey)))
  return kp
}

export async function rpc(method, params = []) {
  const res = await fetch(RPC_URL, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  })
  const body = await res.json()
  if (body.error) throw new Error(`${method}: ${JSON.stringify(body.error)}`)
  return body.result
}

/** Top up `pubkey` to at least `sol` SOL. */
export async function fundSol(pubkey, sol) {
  const have = await connection.getBalance(pubkey)
  const want = Math.round(sol * LAMPORTS_PER_SOL)
  if (have >= want) return
  await rpc('surfnet_setAccount', [pubkey.toBase58(), { lamports: want }])
}

/** Set `owner`'s ATA balance of `mint` to `amount` base units. */
export async function setTokenBalance(owner, mint, amount, tokenProgram = TOKEN_PROGRAM_ID) {
  await rpc('surfnet_setTokenAccount', [owner.toBase58(), mint.toBase58(), { amount: Number(amount) }, tokenProgram.toBase58()])
}

const CLOCK = new PublicKey('SysvarC1ock11111111111111111111111111111111')

/** The fork's `Clock::unix_timestamp`. */
export async function chainNow() {
  const info = await connection.getAccountInfo(CLOCK)
  return Number(info.data.readBigInt64LE(32))
}

/** Mint owner program and decimals, read from the fork. */
export async function mintInfo(mint) {
  const info = await connection.getAccountInfo(mint)
  if (!info) throw new Error(`mint ${mint.toBase58()} not found`)
  return { tokenProgram: info.owner, decimals: info.data[44] }
}

const WSOL = 'So11111111111111111111111111111111111111112'

/** Lamports per base unit of each mint, from Jupiter's live USD prices. */
export async function solPricesPerUnit(mints) {
  const ids = [...new Set([...mints.map((m) => m.toBase58()), WSOL])]
  const res = await fetch(`https://lite-api.jup.ag/price/v3?ids=${ids.join(',')}`)
  if (!res.ok) throw new Error(`jupiter price ${res.status}`)
  const body = await res.json()
  const solUsd = body[WSOL]?.usdPrice
  if (!solUsd) throw new Error('no SOL price')
  const out = new Map()
  for (const m of mints) {
    const p = body[m.toBase58()]
    if (!p) throw new Error(`no price for ${m.toBase58()}`)
    out.set(m.toBase58(), { usd: p.usdPrice, lamportsPerUnit: (p.usdPrice / solUsd) * LAMPORTS_PER_SOL / 10 ** p.decimals })
  }
  return { solUsd, prices: out }
}

/** Lookup table holding `addresses`, created and extended by `payer`. */
export async function createLookupTable(payer, addresses) {
  const slot = await connection.getSlot('finalized')
  const [create, table] = AddressLookupTableProgram.createLookupTable({ authority: payer.publicKey, payer: payer.publicKey, recentSlot: slot })
  await send([create], payer, [], { cu: null })
  for (let i = 0; i < addresses.length; i += 25) {
    const extend = AddressLookupTableProgram.extendLookupTable({
      lookupTable: table,
      authority: payer.publicKey,
      payer: payer.publicKey,
      addresses: addresses.slice(i, i + 25),
    })
    await send([extend], payer, [], { cu: null })
  }
  await new Promise((r) => setTimeout(r, 1_000))
  return table
}

/**
 * Send `ixs` as a v0 transaction paid by `payer`. Returns the signature; on
 * failure throws with the program logs attached.
 */
export async function send(ixs, payer, signers = [], { cu = undefined, lookupTables = [] } = {}) {
  const tables = []
  for (const t of lookupTables) {
    const { value } = await connection.getAddressLookupTable(t)
    if (value) tables.push(value)
  }
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash()
  const all = cu === null ? ixs : [...computeBudgetIxs(cu), ...ixs]
  const message = new TransactionMessage({ payerKey: payer.publicKey, recentBlockhash: blockhash, instructions: all }).compileToV0Message(tables)
  const tx = new VersionedTransaction(message)
  tx.sign([payer, ...signers])
  let sig
  try {
    sig = await connection.sendTransaction(tx, { skipPreflight: false })
  } catch (e) {
    const logs = e.logs ?? (await e.getLogs?.(connection).catch(() => null))
    const err = new Error(`${e.message}${logs ? `\n  ${logs.join('\n  ')}` : ''}`)
    err.logs = logs
    throw err
  }
  const res = await connection.confirmTransaction({ signature: sig, blockhash, lastValidBlockHeight }, 'confirmed')
  if (res.value.err) {
    const t = await connection.getTransaction(sig, { maxSupportedTransactionVersion: 0, commitment: 'confirmed' })
    throw new Error(`tx ${sig} failed: ${JSON.stringify(res.value.err)}\n  ${(t?.meta?.logMessages ?? []).join('\n  ')}`)
  }
  return sig
}

/** CU consumed by a confirmed transaction. */
export async function unitsUsed(sig) {
  const t = await connection.getTransaction(sig, { maxSupportedTransactionVersion: 0, commitment: 'confirmed' })
  return t?.meta?.computeUnitsConsumed ?? null
}

export const fmtSol = (lamports) => (Number(lamports) / LAMPORTS_PER_SOL).toFixed(4)
