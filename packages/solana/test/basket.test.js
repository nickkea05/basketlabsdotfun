import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { test } from 'node:test'
import { BN } from '@anchor-lang/core'
import { Connection, Keypair, PublicKey } from '@solana/web3.js'
import {
  BasketClient,
  WSOL_MINT,
  basketPda,
  bookHash,
  chunkBook,
  componentPositionPda,
  dlmm,
  equalWeights,
  makeBook,
  shareMintPda,
  treasuryShares,
} from '../src/basket/index.js'

const keys = (n) => Array.from({ length: n }, () => Keypair.generate().publicKey)
const client = new BasketClient(new Connection('http://127.0.0.1:8899'))

test('binArrayIndex floors negative bins', () => {
  assert.equal(dlmm.binArrayIndex(0), 0)
  assert.equal(dlmm.binArrayIndex(69), 0)
  assert.equal(dlmm.binArrayIndex(70), 1)
  assert.equal(dlmm.binArrayIndex(-1), -1)
  assert.equal(dlmm.binArrayIndex(-70), -1)
  assert.equal(dlmm.binArrayIndex(-71), -2)
  assert.deepEqual(dlmm.binArrayIndices(-5, 5), [-1, 0])
})

test('binPriceQ64 is 1 at bin 0 and tracks (1 + step)^id', () => {
  assert.equal(dlmm.binPriceQ64(0, 100), dlmm.ONE_Q64)
  // Fixed-point squaring loses precision at extreme bins, on chain too.
  for (const [id, step, tol] of [[1, 100, 1e-12], [-1, 100, 1e-12], [250, 25, 1e-9], [-900, 10, 1e-9], [5000, 80, 2e-2]]) {
    const got = Number(dlmm.binPriceQ64(id, step)) / 2 ** 64
    const want = (1 + step / 10_000) ** id
    assert.ok(Math.abs(got / want - 1) < tol, `id ${id} step ${step}: ${got} vs ${want}`)
  }
  assert.equal(dlmm.binPriceQ64(12345, 80), null)
})

test('lamportsForX rounds up, xForLamports rounds down', () => {
  const p = dlmm.binPriceQ64(1, 100)
  assert.equal(dlmm.lamportsForX(100n, dlmm.ONE_Q64), 100n)
  assert.equal(dlmm.lamportsForX(100n, p), 101n)
  assert.equal(dlmm.lamportsForX(1n, p), 2n)
  assert.equal(dlmm.xForLamports(101n, p), 100n)
})

test('binForPrice inverts the bin price', () => {
  for (const step of [10, 25, 100]) {
    for (const id of [-300, 0, 7, 900]) {
      const price = Number(dlmm.binPriceQ64(id, step)) / 2 ** 64
      assert.equal(dlmm.binForPrice(price * (1 - 1e-12), step), id)
    }
  }
})

test('bookHash chains sha256 over (mint, weight le)', () => {
  const [a, b] = keys(2)
  const book = makeBook([a, b], [6000, 4000])
  let acc = Buffer.alloc(32)
  for (const p of book) {
    const wle = Buffer.alloc(2)
    wle.writeUInt16LE(p.weightBps)
    acc = createHash('sha256').update(Buffer.concat([acc, p.mint.toBuffer(), wle])).digest()
  }
  assert.deepEqual(bookHash(book), Array.from(acc))
})

test('equalWeights sums to 10000 and treasuryShares is H r/(1 - r)', () => {
  for (const n of [1, 3, 7, 30]) assert.equal(equalWeights(n).reduce((s, x) => s + x, 0), 10_000)
  assert.equal(treasuryShares(3_000_000_000n, 2500), 1_000_000_000n)
})

test('chunkBook packs 6 then 8 per transaction', () => {
  const book = makeBook(keys(23), equalWeights(23))
  const { first, rest } = chunkBook(book)
  assert.equal(first.length, 6)
  assert.deepEqual(rest.map((c) => c.length), [8, 8, 1])
})

test('lb pair PDA is order independent', () => {
  const [x] = keys(1)
  assert.equal(dlmm.customizableLbPair(x, WSOL_MINT).toBase58(), dlmm.customizableLbPair(WSOL_MINT, x).toBase58())
})

test('createBasketIx encodes args and appends [mint, position, vault] per entry', async () => {
  const payer = Keypair.generate().publicKey
  const book = makeBook(keys(3), equalWeights(3))
  const ix = await client.createBasketIx({ payer, args: sampleArgs(book), chunk: book })
  const shareMint = shareMintPda(payer, 7)
  const basket = basketPda(shareMint)
  assert.equal(ix.keys[0].pubkey.toBase58(), payer.toBase58())
  assert.equal(ix.keys[0].isSigner, true)
  const tail = ix.keys.slice(-9)
  assert.equal(tail[0].pubkey.toBase58(), book[0].mint.toBase58())
  assert.equal(tail[1].pubkey.toBase58(), componentPositionPda(shareMint, book[0].mint).toBase58())
  assert.equal(tail[0].isWritable, false)
  assert.equal(tail[2].isWritable, true)
  const decoded = client.program.coder.instruction.decode(ix.data)
  assert.equal(decoded.data.args.name, 'Test Basket')
  assert.deepEqual(Object.keys(decoded.data.args.gate), ['windowUntil'])
  assert.equal(decoded.data.args.nonce.toString(), '7')
  assert.equal(decoded.data.positions.length, 3)
  assert.ok(basket instanceof PublicKey)
})

test('seedIx appends components then the tight range bin arrays', async () => {
  const payer = Keypair.generate().publicKey
  const shareMint = shareMintPda(payer, 1)
  const book = makeBook(keys(2), equalWeights(2))
  const ix = await client.seedIx({
    payer,
    shareMint,
    treasury: Keypair.generate().publicKey,
    chunk: book,
    activeId: 0,
    tightHalfWidth: 5,
    deposits: [10n, 20n],
    initialShares: 1_000_000n,
    sleeveLamports: 333_334n,
  })
  const pool = dlmm.poolFor(shareMint, WSOL_MINT)
  const tail = ix.keys.slice(-(2 * 4 + 2))
  assert.equal(tail[3].pubkey.toBase58(), book[0].mint.toBase58())
  assert.deepEqual(
    tail.slice(-2).map((k) => k.pubkey.toBase58()),
    [dlmm.binArray(pool.lbPair, -1).toBase58(), dlmm.binArray(pool.lbPair, 0).toBase58()],
  )
  const decoded = client.program.coder.instruction.decode(ix.data)
  assert.deepEqual(decoded.data.args.deposits.map(String), ['10', '20'])
})

function sampleArgs(book) {
  return {
    nonce: 7,
    basketType: 0,
    fixedKind: 0,
    name: 'Test Basket',
    symbol: 'TEST',
    uri: 'https://basketlabs.fun/m/test.json',
    assetCount: book.length,
    bookHash: bookHash(book),
    hostsHash: Array(32).fill(0),
    hostWeighting: 0,
    strategyHash: Array(32).fill(0),
    sleeveRBps: 700,
    perfFeeBps: 0,
    gate: { windowUntil: { closeTs: new BN(1_900_000_000) } },
    schedule: null,
    launchActiveId: 0,
  }
}
