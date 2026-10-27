// Meteora DLMM helpers the basket client needs: pool / reserve / bin-array /
// position PDAs, bin geometry and the Q64.64 price math. Ported from
// programs/basket/src/dlmm.rs; the math is BigInt so quotes match the program
// to the lamport.

import { PublicKey } from '@solana/web3.js'
import { DLMM_BINS_PER_ARRAY, DLMM_BINS_PER_POSITION, DLMM_ILM_BASE, DLMM_PROGRAM_ID, DLMM_SEEDS } from './constants.js'

const enc = (s) => Buffer.from(s, 'utf8')
const pda = (seeds) => PublicKey.findProgramAddressSync(seeds, DLMM_PROGRAM_ID)[0]

const i32le = (n) => {
  const b = Buffer.alloc(4)
  b.writeInt32LE(n)
  return b
}
const i64le = (n) => {
  const b = Buffer.alloc(8)
  b.writeBigInt64LE(BigInt(n))
  return b
}

const sortMints = (a, b) => (Buffer.compare(a.toBuffer(), b.toBuffer()) <= 0 ? [a, b] : [b, a])

export const eventAuthority = () => pda([enc(DLMM_SEEDS.eventAuthority)])

/** `[ILM_BASE, min(x, y), max(x, y)]`: one customizable pool per pair. */
export function customizableLbPair(mintX, mintY) {
  const [lo, hi] = sortMints(mintX, mintY)
  return pda([DLMM_ILM_BASE.toBuffer(), lo.toBuffer(), hi.toBuffer()])
}

export const reserve = (lbPair, mint) => pda([lbPair.toBuffer(), mint.toBuffer()])
export const oracle = (lbPair) => pda([enc(DLMM_SEEDS.oracle), lbPair.toBuffer()])
export const bitmapExtension = (lbPair) => pda([enc(DLMM_SEEDS.bitmap), lbPair.toBuffer()])
export const binArray = (lbPair, index) => pda([enc(DLMM_SEEDS.binArray), lbPair.toBuffer(), i64le(index)])

/** `["position", lb_pair, base, lower, width]`, `width` capped at one array. */
export function basketPositionPda(lbPair, base, lowerBinId, upperBinId) {
  const width = Math.min(upperBinId - lowerBinId + 1, DLMM_BINS_PER_POSITION)
  return pda([enc(DLMM_SEEDS.position), lbPair.toBuffer(), base.toBuffer(), i32le(lowerBinId), i32le(width)])
}

/** Everything about the share/SOL pool `create_basket` opens. */
export function poolFor(shareMint, wsolMint) {
  const lbPair = customizableLbPair(shareMint, wsolMint)
  return {
    lbPair,
    mintX: shareMint,
    mintY: wsolMint,
    reserveX: reserve(lbPair, shareMint),
    reserveY: reserve(lbPair, wsolMint),
    oracle: oracle(lbPair),
  }
}

// ---- bin geometry ----

/** Bin array index holding `binId` (floor division). */
export function binArrayIndex(binId) {
  return Math.floor(binId / DLMM_BINS_PER_ARRAY)
}

export const arrayLowerBin = (index) => index * DLMM_BINS_PER_ARRAY
export const arrayUpperBin = (index) => index * DLMM_BINS_PER_ARRAY + DLMM_BINS_PER_ARRAY - 1

/** Bin-array indices covering `[lower, upper]`, inclusive. */
export function binArrayIndices(lower, upper) {
  const out = []
  for (let i = binArrayIndex(lower); i <= binArrayIndex(upper); i++) out.push(i)
  return out
}

// ---- price math (Q64.64) ----

export const ONE_Q64 = 1n << 64n
const U128_MAX = (1n << 128n) - 1n
const BASIS_POINT_MAX = 10_000n

const mulShr64 = (a, b) => {
  const p = (a * b) >> 64n
  return p > U128_MAX ? U128_MAX : p
}

function powQ64(base, exp) {
  if (exp === 0) return ONE_Q64
  let invert = exp < 0
  const e = Math.abs(exp)
  if (e >= 0x100000) return null
  let squared = base
  let result = ONE_Q64
  if (squared >= result) {
    squared = U128_MAX / squared
    invert = !invert
  }
  for (let bit = 0; bit < 20; bit++) {
    if (e & (1 << bit)) result = mulShr64(result, squared)
    squared = mulShr64(squared, squared)
  }
  if (result === 0n) return null
  return invert ? U128_MAX / result : result
}

/** `(1 + bin_step / 10⁴)^binId` in Q64.64: lamports per X base unit. */
export function binPriceQ64(binId, binStep) {
  const base = ONE_Q64 + (BigInt(binStep) << 64n) / BASIS_POINT_MAX
  return powQ64(base, binId)
}

/** Lamports `amountX` base units are worth at `priceQ64`, rounded up. */
export function lamportsForX(amountX, priceQ64) {
  const p = BigInt(amountX) * priceQ64
  const q = p / ONE_Q64
  return p % ONE_Q64 === 0n ? q : q + 1n
}

/** X base units `lamports` buy at `priceQ64`, rounded down. */
export function xForLamports(lamports, priceQ64) {
  return (BigInt(lamports) << 64n) / priceQ64
}

/** Lowest bin whose price is at least `lamportsPerUnit` (as a float). */
export function binForPrice(lamportsPerUnit, binStep) {
  return Math.ceil(Math.log(lamportsPerUnit) / Math.log(1 + binStep / 10_000)) | 0
}

// ---- account decoding ----

// LbPair: discriminator(8) + StaticParameters(32) + VariableParameters(32) +
// bump(1) + bin_step_seed(2) + pair_type(1), then active_id i32, bin_step u16.
const LB_PAIR_ACTIVE_ID = 8 + 32 + 32 + 1 + 2 + 1

export function decodeLbPair(data) {
  return { activeId: data.readInt32LE(LB_PAIR_ACTIVE_ID), binStep: data.readUInt16LE(LB_PAIR_ACTIVE_ID + 4) }
}
