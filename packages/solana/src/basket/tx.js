// Versioned-transaction assembly and confirmation, shared by the browser,
// the keeper and the sim. Confirmation polls signature status so it works
// over plain HTTP (no websocket subscription).

import { TransactionMessage, VersionedTransaction } from '@solana/web3.js'
import { computeBudgetIxs } from './client.js'

/** Fetch lookup-table accounts; missing tables are skipped. */
export async function fetchLookupTables(connection, addresses = []) {
  const out = []
  for (const a of addresses) {
    const { value } = await connection.getAddressLookupTable(a)
    if (value) out.push(value)
  }
  return out
}

/**
 * Unsigned v0 transaction paid by `payer`, with a compute-unit limit
 * prepended unless `cu` is null.
 */
export async function buildTransaction(connection, { payer, instructions, lookupTables = [], cu }) {
  const tables = await fetchLookupTables(connection, lookupTables)
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash('confirmed')
  const all = cu === null ? instructions : [...computeBudgetIxs(cu), ...instructions]
  const message = new TransactionMessage({ payerKey: payer, recentBlockhash: blockhash, instructions: all }).compileToV0Message(tables)
  return { transaction: new VersionedTransaction(message), blockhash, lastValidBlockHeight }
}

/** Poll until `signature` is confirmed; throws on failure or expiry. */
export async function confirmSignature(connection, signature, lastValidBlockHeight, { intervalMs = 500 } = {}) {
  for (;;) {
    const { value } = await connection.getSignatureStatuses([signature])
    const s = value[0]
    if (s?.err) throw new Error(`transaction failed: ${JSON.stringify(s.err)}`)
    if (s?.confirmationStatus === 'confirmed' || s?.confirmationStatus === 'finalized') return s
    if ((await connection.getBlockHeight('confirmed')) > lastValidBlockHeight) throw new Error('transaction expired before confirming')
    await new Promise((r) => setTimeout(r, intervalMs))
  }
}
