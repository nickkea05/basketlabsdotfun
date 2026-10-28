// End-to-end smoke test on the Surfpool mainnet fork: config, treasury vaults,
// whitelist of real mints, one Fixed basket at live prices, and its first buy
// (seed) into the real Meteora DLMM program.
//
//   node sim/smoke.mjs            (program deployed with scripts/sim-deploy.mjs)

import { BN } from '@anchor-lang/core'
import { PublicKey } from '@solana/web3.js'
import {
  BasketClient,
  Cluster,
  DLMM_PROGRAM_ID,
  MINT_WINDOW_S,
  STEP_R_BPS,
  WSOL_MINT,
  ata,
  basketLookupAddresses,
  basketPda,
  bookHash,
  chunkBook,
  configPda,
  createAtaIdempotentIx,
  dlmm,
  equalWeights,
  makeBook,
  shareMintPda,
  treasuryShares,
} from '@basketfun/solana/basket'
import {
  adminKeypair,
  chainNow,
  connection,
  createLookupTable,
  fmtSol,
  fundSol,
  mintInfo,
  saveLastBasket,
  send,
  setTokenBalance,
  simKey,
  solPricesPerUnit,
  unitsUsed,
} from './lib.mjs'

const UNIVERSE = [
  ['JUP', 'JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN'],
  ['BONK', 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263'],
  ['JitoSOL', 'J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn'],
  ['WIF', 'EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm'],
  ['mSOL', 'mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So'],
  ['PYTH', 'HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3'],
  ['JTO', 'jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL'],
  ['RAY', '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R'],
  ['ORCA', 'orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE'],
]
/** Component count: `SIM_N=8 node sim/smoke.mjs`. */
const COMPONENTS = UNIVERSE.slice(0, Number(process.env.SIM_N ?? 3))
/** SOL value of the components in the first buy; the sleeve adds r/(1−r) on top. */
const COMPONENT_VALUE_LAMPORTS = 1_500_000_000n
const LAUNCH_ACTIVE_ID = 0

const log = (...a) => console.log(...a)
const step = (s) => log(`\n== ${s}`)

const admin = adminKeypair()
const treasury = simKey('treasury')
const keeper = simKey('keeper')
const deployer = simKey(`deployer-${Date.now()}`)
const client = new BasketClient(connection)

step('preflight')
const program = await connection.getAccountInfo(client.programId)
if (!program?.executable) throw new Error('basket program not deployed: run node scripts/sim-deploy.mjs')
const dlmmInfo = await connection.getAccountInfo(DLMM_PROGRAM_ID)
log(`basket program ${client.programId.toBase58()} ok; DLMM ${dlmmInfo?.executable ? 'ok' : 'MISSING'}`)
await fundSol(admin.publicKey, 50)
await fundSol(treasury.publicKey, 1)
await fundSol(keeper.publicKey, 10)
await fundSol(deployer.publicKey, 20)

step('config')
if (await connection.getAccountInfo(configPda())) {
  log('config exists')
} else {
  const ix = await client.initializeConfigIx({ admin: admin.publicKey, cluster: Cluster.Localnet, treasury: treasury.publicKey, keepers: [keeper.publicKey] })
  log('initialize_config', await send([ix], admin))
  log('init_treasury_vaults', await send([await client.initTreasuryVaultsIx({ admin: admin.publicKey })], admin))
}
const config = await client.fetchConfig()
const configTreasury = config.treasury
log(`treasury ${configTreasury.toBase58()}, keepers ${config.keepers.map((k) => k.toBase58()).join(', ')}`)

step('whitelist')
const mints = COMPONENTS.map(([, m]) => new PublicKey(m))
const listed = new Set((await client.fetchWhitelist()).entries.map((e) => e.mint.toBase58()))
const missing = mints.filter((m) => !listed.has(m.toBase58()))
if (missing.length) log('update_whitelist', await send([await client.updateWhitelistIx({ authority: admin.publicKey, add: missing.map((mint) => ({ mint, tier: 0 })) })], admin))
log(`whitelist has ${(await client.fetchWhitelist()).entries.length} entries`)

step('book at live prices')
const infos = await Promise.all(mints.map(mintInfo))
const { solUsd, prices } = await solPricesPerUnit(mints)
const weights = equalWeights(mints.length)
const book = makeBook(mints, weights).map((p, i) => ({ ...p, tokenProgram: infos[i].tokenProgram }))
const deposits = book.map((p) => {
  const value = (Number(COMPONENT_VALUE_LAMPORTS) * p.weightBps) / 10_000
  return BigInt(Math.floor(value / prices.get(p.mint.toBase58()).lamportsPerUnit))
})
log(`SOL $${solUsd.toFixed(2)}`)
COMPONENTS.forEach(([sym], i) =>
  log(`  ${sym.padEnd(8)} ${(weights[i] / 100).toFixed(2)}%  $${prices.get(mints[i].toBase58()).usd.toPrecision(6)}  deposit ${(Number(deposits[i]) / 10 ** infos[i].decimals).toPrecision(6)}`),
)

step('lookup table')
const nonce = Date.now()
const shareMint = shareMintPda(deployer.publicKey, nonce)
const basket = basketPda(shareMint)
const pool = dlmm.poolFor(shareMint, WSOL_MINT)
const lut = await createLookupTable(deployer, basketLookupAddresses({ shareMint, book }))
log(`lut ${lut.toBase58()}`)

step('create_basket')
const now = await chainNow()
const chunking = {}
if (process.env.SIM_FIRST) chunking.first = Number(process.env.SIM_FIRST)
if (process.env.SIM_PER) chunking.per = Number(process.env.SIM_PER)
const { first, rest } = chunkBook(book, chunking)
const createIx = await client.createBasketIx({
  payer: deployer.publicKey,
  chunk: first,
  args: {
    nonce,
    basketType: 0,
    fixedKind: 0,
    name: 'Sim Solana Trio',
    symbol: 'TRIO',
    uri: 'https://basketlabs.fun/m/trio.json',
    assetCount: book.length,
    bookHash: bookHash(book),
    hostsHash: Array(32).fill(0),
    hostWeighting: 0,
    strategyHash: Array(32).fill(0),
    sleeveRBps: 700,
    perfFeeBps: 0,
    gate: { windowUntil: { closeTs: new BN(now + MINT_WINDOW_S) } },
    schedule: null,
    launchActiveId: LAUNCH_ACTIVE_ID,
  },
})
const before = await connection.getBalance(deployer.publicKey)
const wsolAtaIx = createAtaIdempotentIx(deployer.publicKey, deployer.publicKey, WSOL_MINT)
const createSig = await send([wsolAtaIx, createIx], deployer, [], { lookupTables: [lut] })
log(`create_basket ${createSig} (${await unitsUsed(createSig)} CU), deployer spent ${fmtSol(before - (await connection.getBalance(deployer.publicKey)))} SOL`)
for (const chunk of rest) log('add_positions', await send([await client.addPositionsIx({ payer: deployer.publicKey, shareMint, chunk })], deployer, [], { lookupTables: [lut] }))

const b = await client.fetchBasket(shareMint)
const lb = await client.fetchLbPair(pool.lbPair)
log(`basket ${basket.toBase58()} share mint ${shareMint.toBase58()}`)
log(`pool ${pool.lbPair.toBase58()} active ${lb.activeId} bin step ${lb.binStep} (preset ${b.pool.preset.binStep}, tight ±${b.pool.preset.tightHalfWidthBins})`)
if (lb.activeId !== LAUNCH_ACTIVE_ID) throw new Error(`pool opened at ${lb.activeId}, expected ${LAUNCH_ACTIVE_ID}`)

step('fund components (cheatcode)')
for (const [i, p] of book.entries()) await setTokenBalance(deployer.publicKey, p.mint, deposits[i], p.tokenProgram)
for (const [i, p] of book.entries()) {
  const bal = await connection.getTokenAccountBalance(ata(deployer.publicKey, p.mint, p.tokenProgram))
  log(`  ${COMPONENTS[i][0].padEnd(8)} ${bal.value.uiAmountString}`)
}

step('seed (first buy)')
const price = dlmm.binPriceQ64(lb.activeId, lb.binStep)
const initialShares = dlmm.xForLamports(COMPONENT_VALUE_LAMPORTS, price)
const sleeveLamports = dlmm.lamportsForX(treasuryShares(initialShares, STEP_R_BPS), price)
log(`initial shares ${initialShares} base units, sleeve ${fmtSol(sleeveLamports)} SOL, implied total ${fmtSol(COMPONENT_VALUE_LAMPORTS + sleeveLamports)} SOL`)
const seedIx = await client.seedIx({
  payer: deployer.publicKey,
  shareMint,
  treasury: configTreasury,
  chunk: book,
  activeId: lb.activeId,
  tightHalfWidth: b.pool.preset.tightHalfWidthBins,
  deposits,
  initialShares,
  sleeveLamports,
})
const seedSig = await send([seedIx], deployer, [], { lookupTables: [lut] })
log(`seed ${seedSig} (${await unitsUsed(seedSig)} CU)`)

step('verify')
const after = await client.fetchBasket(shareMint)
const shares = await connection.getTokenAccountBalance(ata(deployer.publicKey, shareMint))
const lbAfter = await client.fetchLbPair(pool.lbPair)
const reserveY = await connection.getTokenAccountBalance(pool.reserveY)
const reserveX = await connection.getTokenAccountBalance(pool.reserveX)
log(`seeded ${after.seeded}, tight [${after.tight.lowerBinId}, ${after.tight.upperBinId}] key ${after.tight.key.toBase58()}`)
log(`deployer shares ${shares.value.uiAmountString}; pool reserves ${reserveX.value.uiAmountString} shares / ${reserveY.value.uiAmountString} SOL; active ${lbAfter.activeId}`)
for (const [i, p] of book.entries()) {
  const vault = await connection.getTokenAccountBalance(ata(basket, p.mint, p.tokenProgram))
  log(`  vault ${COMPONENTS[i][0].padEnd(8)} ${vault.value.uiAmountString}`)
}
saveLastBasket({ shareMint: shareMint.toBase58(), lut: lut.toBase58(), components: COMPONENTS.length })
log('\nsmoke ok')
