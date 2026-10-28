// Off-chain mirror of the program's `PoolView`: what the basket's tight and
// backstop DLMM positions hold, read from raw position and bin-array data.
// Byte layouts follow programs/basket/src/dlmm.rs.

import { DLMM_BINS_PER_ARRAY } from './constants.js'
import { arrayLowerBin, binArrayIndex } from './dlmm.js'

const POSITION_LB_PAIR = 8
const POSITION_OWNER = 40
const POSITION_SHARES = 72
const POSITION_LOWER = 8 + 32 + 32 + 70 * 16 + 70 * 48 + 70 * 48
const POSITION_FIXED_LEN = 8112
const POSITION_BIN_DATA_LEN = 112
const BIN_ARRAY_HEADER = 8 + 8 + 1 + 7 + 32
const BIN_LEN = 144

const readU128 = (data, at) => data.readBigUInt64LE(at) | (data.readBigUInt64LE(at + 8) << 64n)
const mulDiv = (a, b, d, up) => (up ? (a * b + d - 1n) / d : (a * b) / d)

export function decodePositionHeader(data) {
  return {
    lbPair: data.subarray(POSITION_LB_PAIR, POSITION_LB_PAIR + 32),
    owner: data.subarray(POSITION_OWNER, POSITION_OWNER + 32),
    lowerBinId: data.readInt32LE(POSITION_LOWER),
    upperBinId: data.readInt32LE(POSITION_LOWER + 4),
  }
}

/** Liquidity share of the position's `offset`-th bin (0 = lower bin). */
export function positionBinShare(data, offset) {
  if (offset < DLMM_BINS_PER_ARRAY) return readU128(data, POSITION_SHARES + offset * 16)
  return readU128(data, 8 + POSITION_FIXED_LEN + (offset - DLMM_BINS_PER_ARRAY) * POSITION_BIN_DATA_LEN)
}

export function binAt(arrayData, slot) {
  const at = BIN_ARRAY_HEADER + slot * BIN_LEN
  return {
    amountX: arrayData.readBigUInt64LE(at),
    amountY: arrayData.readBigUInt64LE(at + 8),
    liquiditySupply: readU128(arrayData, at + 32),
  }
}

/**
 * Shares (X, rounded up) and SOL (Y, rounded down) a position holds.
 * `arrays` maps bin-array index to its raw data.
 */
export function positionAmounts(positionData, arrays) {
  const h = decodePositionHeader(positionData)
  let amountX = 0n
  let amountY = 0n
  for (let offset = 0; offset <= h.upperBinId - h.lowerBinId; offset++) {
    const share = positionBinShare(positionData, offset)
    if (share === 0n) continue
    const binId = h.lowerBinId + offset
    const idx = binArrayIndex(binId)
    const arr = arrays.get(idx)
    if (!arr) throw new Error(`bin array ${idx} not loaded`)
    const bin = binAt(arr, binId - arrayLowerBin(idx))
    if (bin.liquiditySupply === 0n) continue
    if (share >= bin.liquiditySupply) {
      amountX += bin.amountX
      amountY += bin.amountY
    } else {
      amountX += mulDiv(bin.amountX, share, bin.liquiditySupply, true)
      amountY += mulDiv(bin.amountY, share, bin.liquiditySupply, false)
    }
  }
  return { lowerBinId: h.lowerBinId, upperBinId: h.upperBinId, amountX, amountY }
}
