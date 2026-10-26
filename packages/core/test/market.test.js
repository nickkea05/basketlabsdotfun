import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  BasketType,
  MINT_WINDOW_H,
  PERFORMANCE_MIN_MC,
  fmtHoursLeft,
  isMintOpen,
  mintHoursLeft,
  mintOpenBaskets,
  mintWindowLeft,
  premiumPct,
  topPerformers,
  trendingBaskets,
} from '../src/index.js'

const mk = (o) => ({ type: BasketType.Fixed, ageH: 100, marketCap: 1_000_000, change24h: 0, price: 1, nav: 1, basket: [], ...o })
const ids = (r) => r.map((t) => t.id)

test('mint window: open while hours remain, closed at zero, missing or negative', () => {
  assert.equal(isMintOpen(mk({ mintLeftH: 3 })), true)
  for (const mintLeftH of [0, -1, null, undefined]) assert.equal(isMintOpen(mk({ mintLeftH })), false, String(mintLeftH))
  assert.equal(mintHoursLeft(mk({ mintLeftH: -5 })), 0)
  assert.equal(mintWindowLeft(mk({ mintLeftH: MINT_WINDOW_H / 4 })), 0.25)
  assert.equal(mintWindowLeft(mk({ mintLeftH: MINT_WINDOW_H * 2 })), 1)
})

test('fmtHoursLeft reads hours and minutes, and says Closed at zero', () => {
  assert.equal(fmtHoursLeft(31.2), '31h 12m')
  assert.equal(fmtHoursLeft(2), '2h')
  assert.equal(fmtHoursLeft(0.63), '38m')
  assert.equal(fmtHoursLeft(0.001), '1m')
  assert.equal(fmtHoursLeft(0), 'Closed')
})

test('premiumPct is pool price over NAV, signed', () => {
  assert.ok(Math.abs(premiumPct(mk({ price: 1.05, nav: 1 })) - 5) < 1e-9)
  assert.ok(Math.abs(premiumPct(mk({ price: 0.98, nav: 1 })) + 2) < 1e-9)
  assert.equal(premiumPct(mk({ nav: 0 })), 0)
})

test('mintOpenBaskets keeps only open windows and sorts by the chosen key', () => {
  const items = [
    mk({ id: 'a', ageH: 10, mintLeftH: 38, marketCap: 5 }),
    mk({ id: 'b', ageH: 2, mintLeftH: 46, marketCap: 9 }),
    mk({ id: 'c', ageH: 47, mintLeftH: 1, marketCap: 7 }),
    mk({ id: 'shut', ageH: 60 }),
  ]
  assert.deepEqual(ids(mintOpenBaskets(items)), ['b', 'a', 'c'])
  assert.deepEqual(ids(mintOpenBaskets(items, 'closing')), ['c', 'a', 'b'])
  assert.deepEqual(ids(mintOpenBaskets(items, 'mc')), ['b', 'c', 'a'])
})

test('trendingBaskets drops anything without trades in the last hour and caps the list', () => {
  const items = [
    mk({ id: 'quiet', volume1h: 0, txns1h: 0, marketCap: 9e9 }),
    mk({ id: 'busy', volume1h: 50_000, txns1h: 40 }),
    mk({ id: 'whale', volume1h: 90_000, txns1h: 3 }),
  ]
  assert.deepEqual(ids(trendingBaskets(items)), ['whale', 'busy'])
  assert.deepEqual(ids(trendingBaskets(items, 'txns')), ['busy', 'whale'])
  assert.deepEqual(ids(trendingBaskets(items, 'mc', 1)), ['busy'])
})

test('topPerformers ranks by NAV return and needs enough history and size', () => {
  const items = [
    mk({ id: 'young', ageH: 30, navChange24h: 80, navChange7d: 80 }),
    mk({ id: 'tiny', ageH: 400, marketCap: PERFORMANCE_MIN_MC - 1, navChange7d: 90 }),
    mk({ id: 'good', ageH: 400, navChange24h: 2, navChange7d: 30 }),
    mk({ id: 'ok', ageH: 200, navChange24h: 5, navChange7d: 10 }),
    mk({ id: 'nodata', ageH: 400 }),
  ]
  assert.deepEqual(ids(topPerformers(items, '7d')), ['good', 'ok'])
  assert.deepEqual(ids(topPerformers(items, '24h')), ['young', 'ok', 'good'])
})
