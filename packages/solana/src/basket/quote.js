// Mint and redeem quotes over a basket state snapshot (see
// `BasketClient.loadState`). Mirrors handle_mint / handle_redeem; BigInt.

import { BPS_TOTAL } from './constants.js'
import { binPriceQ64, lamportsForX } from './dlmm.js'
import { treasuryShares } from './math.js'

const BPS = BigInt(BPS_TOTAL)
const bpsDown = (amount, bps) => (amount * BigInt(bps)) / BPS
const ceilDiv = (a, b) => (a + b - 1n) / b

/** Step ratio until the pool's SOL side reaches the threshold, then the creator's ratio. */
export function effectiveRBps(sleeve, poolSol) {
  return poolSol < BigInt(sleeve.stepThresholdLamports.toString()) ? sleeve.stepRBps : sleeve.rBps
}

/**
 * What `mint` does with `deposits` (book order, base units): gross shares
 * by the creation-unit rule, the buyer/fee split, and the SOL leg priced
 * at the active bin.
 */
export function mintQuote(state, deposits) {
  if (deposits.length !== state.book.length) throw new Error('one deposit per book entry')
  let gross = null
  for (const [i, d] of deposits.entries()) {
    const v = state.book[i].available
    if (v === 0n) throw new Error(`empty vault for ${state.book[i].mint.toBase58()}`)
    const s = (BigInt(d) * state.holderShares) / v
    gross = gross === null || s < gross ? s : gross
  }
  const feeShares = bpsDown(gross, state.basket.fees.mintFeeBps)
  const rBps = effectiveRBps(state.basket.sleeve, state.poolSol)
  const treasury = treasuryShares(gross, rBps)
  const sleeveLamports = lamportsForX(treasury, binPriceQ64(state.activeId, state.binStep))
  const impliedTotalLamports = (sleeveLamports * BPS) / BigInt(rBps)
  const minLamports = BigInt(state.config.fees.minMintLamports.toString())
  return {
    grossShares: gross,
    toBuyer: gross - feeShares,
    feeShares,
    rBps,
    treasuryShares: treasury,
    sleeveLamports,
    impliedTotalLamports,
    /** The program rejects buys whose implied size (`sleeve / r`) is under `min_mint_lamports`. */
    minLamports,
    belowMinimum: sleeveLamports > 0n && impliedTotalLamports < minLamports,
  }
}

/** Smallest deposits that mint `grossShares` (rounded up per component). */
export function depositsForShares(state, grossShares) {
  return state.book.map((p) => ceilDiv(BigInt(grossShares) * p.available, state.holderShares))
}

/**
 * Slippage bounds for a mint quote: accept `toBuyer` down by `bps`, SOL
 * leg up by `bps` (the active bin can move before the transaction lands).
 */
export function mintBounds(quote, bps = 100) {
  return {
    minSharesOut: quote.toBuyer - bpsDown(quote.toBuyer, bps),
    maxSleeveLamports: quote.sleeveLamports + ceilDiv(quote.sleeveLamports * BigInt(bps), BPS),
  }
}

/**
 * What `redeem` pays for `shares`: the fee, components pro rata of what the
 * vaults hold for holders, and SOL pro rata of tight + idle. The program
 * tops the SOL up from idle when the bin-by-bin withdrawal falls short, so
 * `lamportsOut` is exact unless idle runs dry (then it is a ceiling).
 */
export function redeemQuote(state, shares) {
  const gross = BigInt(shares)
  const fee = bpsDown(gross, state.basket.fees.redeemFeeBps)
  const net = gross - fee
  const h = state.holderShares > net ? state.holderShares : net
  const poolSol = (state.tight?.amountY ?? 0n) + state.idleSol
  return {
    feeShares: fee,
    netShares: net,
    components: state.book.map((p) => (net * p.available) / h),
    lamportsOut: (net * poolSol) / h,
  }
}
