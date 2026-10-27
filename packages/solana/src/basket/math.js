// Book hashing and share arithmetic mirrored from the program
// (state/basket.rs `chain_hash`, math.rs). BigInt throughout.

import { sha256 } from '@noble/hashes/sha256'
import { BPS_TOTAL } from './constants.js'

const u16le = (n) => {
  const b = Buffer.alloc(2)
  b.writeUInt16LE(n)
  return b
}

/** Chain hash over the ordered `(mint, weight_bps)` book: `acc = sha256(acc ‖ mint ‖ w_le)`. */
export function bookHash(book) {
  let acc = new Uint8Array(32)
  for (const p of book) acc = sha256(Buffer.concat([acc, p.mint.toBuffer(), u16le(p.weightBps)]))
  return Array.from(acc)
}

/** Weights summing to exactly `BPS_TOTAL`, remainder to the first entry. */
export function equalWeights(n) {
  const each = Math.floor(BPS_TOTAL / n)
  const w = Array(n).fill(each)
  w[0] += BPS_TOTAL - each * n
  return w
}

/** Book entries from mints and weights. */
export const makeBook = (mints, weights) => mints.map((mint, i) => ({ mint, weightBps: weights[i] }))

/** Treasury shares paired with `holderShares` at sleeve ratio `r`: `H·r/(1−r)`, rounded down. */
export function treasuryShares(holderShares, rBps) {
  return (BigInt(holderShares) * BigInt(rBps)) / BigInt(BPS_TOTAL - rBps)
}
