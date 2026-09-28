import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  BPS_TOTAL,
  BasketType,
  HostWeighting,
  MAX_ASSETS_FIXED,
  buildLaunchParams,
  combineHosts,
  maxAssetsFor,
  normalizeWeights,
  sumBps,
  validateParams,
} from '../src/index.js'

const tok = (mint) => ({ mint, symbol: mint.slice(0, 3) })
const host = (address, positions) => ({
  address,
  snapshot: { rows: positions.map(([mint, valueUsd]) => ({ token: tok(mint), valueUsd })) },
})

test('normalizeWeights scales to exactly BPS_TOTAL and keeps proportions', () => {
  const w = normalizeWeights({ a: 50, b: 35, c: 30 })
  assert.equal(sumBps(w), BPS_TOTAL)
  assert.ok(w.a > w.b && w.b > w.c)
})

test('normalizeWeights gives equal weights when everything is zero', () => {
  const w = normalizeWeights({ a: 0, b: 0, c: 0 })
  assert.equal(sumBps(w), BPS_TOTAL)
  // Remainder from integer division lands on one asset; spread is at most 1 bp.
  assert.ok(Math.max(w.a, w.b, w.c) - Math.min(w.a, w.b, w.c) <= 1)
})

test('combineHosts pools overlapping holdings', () => {
  const { weights } = combineHosts([host('h1', [['SOL', 100]]), host('h2', [['SOL', 100]])], HostWeighting.Equal)
  assert.equal(weights.SOL, BPS_TOTAL)
})

test('combineHosts Equal: each wallet counts 1/N regardless of size', () => {
  const hosts = [host('whale', [['SOL', 1_000_000]]), host('small', [['JUP', 100]])]
  const { weights } = combineHosts(hosts, HostWeighting.Equal)
  assert.equal(weights.SOL, BPS_TOTAL / 2)
  assert.equal(weights.JUP, BPS_TOTAL / 2)
})

test('combineHosts Value: dollars are pooled so the whale dominates', () => {
  const hosts = [host('whale', [['SOL', 900]]), host('small', [['JUP', 100]])]
  const { weights } = combineHosts(hosts, HostWeighting.Value)
  assert.equal(weights.SOL, 9000)
  assert.equal(weights.JUP, 1000)
})

test('combineHosts keeps every pooled position; the limit is on wallets, not assets', () => {
  const positions = Array.from({ length: 75 }, (_, i) => [`M${i}`, 1000 - i])
  const { assets, weights } = combineHosts([host('h', positions)])
  assert.equal(assets.length, 75)
  assert.equal(sumBps(weights), BPS_TOTAL)
})

test('combineHosts orders assets by combined weight, largest first', () => {
  const { assets } = combineHosts(
    [
      host('a', [
        ['X', 10],
        ['Y', 90],
      ]),
      host('b', [
        ['X', 50],
        ['Y', 50],
      ]),
    ],
    HostWeighting.Equal
  )
  assert.deepEqual(
    assets.map((t) => t.mint),
    ['Y', 'X']
  )
})

test('asset cap applies to Fixed baskets only', () => {
  assert.equal(maxAssetsFor(BasketType.Fixed), MAX_ASSETS_FIXED)
  assert.equal(maxAssetsFor(BasketType.Mirror), Infinity)
  assert.equal(maxAssetsFor(BasketType.Managed), Infinity)
})

const params = (type, n) => ({
  basket_type: type,
  name: 'Test',
  symbol: 'TST',
  assets: normalizeWeightsList(n),
  host_wallets: type === BasketType.Mirror ? ['h'] : [],
})
const normalizeWeightsList = (n) => {
  const w = normalizeWeights(Object.fromEntries(Array.from({ length: n }, (_, i) => [`M${i}`, 1])))
  return Object.entries(w).map(([mint, weight_bps]) => ({ mint, weight_bps }))
}

test('validateParams rejects a Fixed basket over the cap and accepts Managed/Mirror at the same size', () => {
  const n = MAX_ASSETS_FIXED + 1
  assert.ok(validateParams(params(BasketType.Fixed, n)).some((e) => /assets/.test(e)))
  assert.deepEqual(validateParams(params(BasketType.Managed, n)), [])
  assert.deepEqual(validateParams(params(BasketType.Mirror, n)), [])
})

test('buildLaunchParams stamps the connected wallet as creator, null when nobody is connected', () => {
  const draft = {
    basketType: BasketType.Fixed,
    assets: [{ mint: 'A' }, { mint: 'B' }],
    weights: { A: 5000, B: 5000 },
    hosts: [],
    hostWeighting: HostWeighting.Equal,
    meta: { name: 'n', symbol: 's', description: '', image: null, twitter: '', telegram: '', website: '' },
  }
  assert.equal(buildLaunchParams(draft).creator, null)
  assert.equal(buildLaunchParams(draft, { creator: 'Wallet111' }).creator, 'Wallet111')
})

test('validateParams accepts a Fixed basket exactly at the cap', () => {
  assert.deepEqual(validateParams(params(BasketType.Fixed, MAX_ASSETS_FIXED)), [])
})
