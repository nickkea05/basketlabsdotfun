// Mints and redeems against the basket the smoke test launched, checking
// every result against the SDK quote.
//
//   node sim/trade.mjs [shareMint]

import { PublicKey } from '@solana/web3.js'
import { BasketClient, ata, basketLookupAddresses, depositsForShares, mintBounds, mintQuote, redeemQuote } from '@basketfun/solana/basket'
import { connection, createLookupTable, fmtSol, fundSol, lastBasket, send, setTokenBalance, simKey, sleep, unitsUsed } from './lib.mjs'

const client = new BasketClient(connection)
const saved = process.argv[2] ? null : lastBasket()
const shareMint = saved ? saved.shareMint : new PublicKey(process.argv[2])
const log = (...a) => console.log(...a)
const TX_FEE = 5_000n
let failures = 0
const check = (label, got, want) => {
  const ok = got === want
  if (!ok) failures++
  log(`  ${ok ? 'ok  ' : 'FAIL'} ${label}: ${got}${ok ? '' : ` (quote ${want})`}`)
}

const tokenBalance = async (owner, mint, tokenProgram) => {
  const info = await connection.getAccountInfo(ata(owner, mint, tokenProgram))
  return info ? info.data.readBigUInt64LE(64) : 0n
}

async function mintFor(name, fractionBps) {
  const buyer = simKey(`buyer-${name}-${Date.now()}`)
  await fundSol(buyer.publicKey, 100)
  const state = await client.loadState(shareMint)
  const gross = (state.holderShares * BigInt(fractionBps)) / 10_000n
  const deposits = depositsForShares(state, gross)
  for (const [i, p] of state.book.entries()) await setTokenBalance(buyer.publicKey, p.mint, deposits[i], p.tokenProgram)
  const q = mintQuote(state, deposits)
  const solBefore = await connection.getBalance(buyer.publicKey)
  const ix = await client.mintIx({ payer: buyer.publicKey, state, deposits, ...mintBounds(q) })
  const sig = await send([ix], buyer, [], { lookupTables: [lut] })
  const shares = await tokenBalance(buyer.publicKey, shareMint)
  const spent = BigInt(solBefore - (await connection.getBalance(buyer.publicKey)))
  log(`\nmint ${name}: ${(fractionBps / 100).toFixed(1)}% of holder shares, r ${q.rBps / 100}%, ${await unitsUsed(sig)} CU`)
  check('shares to buyer', shares, q.toBuyer)
  log(`  SOL spent ${fmtSol(spent)} (sleeve ${fmtSol(q.sleeveLamports)} + rent and fees ${fmtSol(spent - q.sleeveLamports)})`)
  for (const [i, p] of state.book.entries()) check(`${p.mint.toBase58().slice(0, 6)} left in wallet`, await tokenBalance(buyer.publicKey, p.mint, p.tokenProgram), 0n)
  return { buyer, shares }
}

async function redeemFor(name, { buyer, shares }, fractionBps) {
  const amount = (shares * BigInt(fractionBps)) / 10_000n
  const state = await client.loadState(shareMint)
  const q = redeemQuote(state, amount)
  const before = await Promise.all(state.book.map((p) => tokenBalance(buyer.publicKey, p.mint, p.tokenProgram)))
  const solBefore = await connection.getBalance(buyer.publicKey)
  const sig = await send([await client.redeemIx({ holder: buyer.publicKey, state, shares: amount })], buyer, [], { lookupTables: [lut] })
  const sol = BigInt((await connection.getBalance(buyer.publicKey)) - solBefore)
  log(`\nredeem ${name}: ${(fractionBps / 100).toFixed(0)}% of position, ${await unitsUsed(sig)} CU`)
  check('shares left', await tokenBalance(buyer.publicKey, shareMint), shares - amount)
  for (const [i, p] of state.book.entries()) {
    const got = (await tokenBalance(buyer.publicKey, p.mint, p.tokenProgram)) - before[i]
    check(`${p.mint.toBase58().slice(0, 6)} paid`, got, q.components[i])
  }
  check('lamports out (+ 5000 tx fee)', sol + TX_FEE, q.lamportsOut)
}

const start = await client.loadState(shareMint)
const lut = saved?.lut ? new PublicKey(saved.lut) : await newLookupTable(start)

async function newLookupTable(state) {
  const payer = simKey('lut-payer')
  await fundSol(payer.publicKey, 2)
  return createLookupTable(payer, basketLookupAddresses(state))
}
log(`basket ${start.basketKey.toBase58()}: ${start.book.length} components, holder shares ${start.holderShares}, pool SOL ${fmtSol(start.poolSol)}, active bin ${start.activeId}`)

const small = await mintFor('small', 500)
const medium = await mintFor('medium', 2_500)
await mintFor('large', 10_000)

await sleep(1_500)
await redeemFor('small', small, 5_000)
await redeemFor('medium', medium, 10_000)

const end = await client.loadState(shareMint)
log(`\nend: holder shares ${end.holderShares}, pool SOL ${fmtSol(end.poolSol)}, active bin ${end.activeId}, tight SOL ${fmtSol(end.tight.amountY)}`)
log(failures ? `\n${failures} check(s) failed` : '\nall checks passed')
process.exit(failures ? 1 : 0)
