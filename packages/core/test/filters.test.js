import { test } from 'node:test'
import assert from 'node:assert/strict'
import { BasketType, EMPTY_FILTERS, RANGE_FIELDS, activeFilters, filterBaskets, fmtRange, parseAmount, parseKeywords } from '../src/index.js'

const mk = (o) => ({ type: BasketType.Fixed, ageH: 100, marketCap: 1, volume24h: 1, change24h: 0, basket: [], ...o })

const items = [
  mk({
    id: 'blue',
    name: 'Solana Blue',
    ticker: 'BLUE',
    marketCap: 4_000_000,
    volume24h: 600_000,
    change24h: 3.8,
    ageH: 144,
    basket: [{ symbol: 'SOL' }, { symbol: 'JUP' }, { symbol: 'RAY' }],
  }),
  mk({
    id: 'szn',
    name: 'Solana Season',
    ticker: 'SZN',
    marketCap: 61_000,
    volume24h: 88_000,
    change24h: 41,
    ageH: 2,
    raised: 58,
    target: 85,
    type: BasketType.Mirror,
    basket: [{ symbol: 'SOL' }],
  }),
  mk({
    id: 'dogs',
    name: 'Dog Park',
    ticker: 'DOGS',
    marketCap: 960_000,
    volume24h: 274_000,
    change24h: -6.9,
    ageH: 336,
    type: BasketType.Managed,
    basket: [{ symbol: 'WIF' }, { symbol: 'BONK' }],
  }),
]
const ids = (r) => r.map((t) => t.id)

test('parseAmount reads plain numbers, k/m/b suffixes, and ignores $ , % and spaces', () => {
  assert.equal(parseAmount('100'), 100)
  assert.equal(parseAmount('100k'), 100_000)
  assert.equal(parseAmount('1.5M'), 1_500_000)
  assert.equal(parseAmount('2b'), 2_000_000_000)
  assert.equal(parseAmount('$1,250,000'), 1_250_000)
  assert.equal(parseAmount(' -5 % '), -5)
  assert.equal(parseAmount('.5'), 0.5)
})

test('parseAmount returns null for empty or unreadable input', () => {
  for (const s of ['', '   ', 'abc', '1k2', '--3', null, undefined]) assert.equal(parseAmount(s), null, JSON.stringify(s))
})

test('parseKeywords splits on commas, trims, lowercases, drops empties', () => {
  assert.deepEqual(parseKeywords(' Dog, SOL ,, bonk '), ['dog', 'sol', 'bonk'])
  assert.deepEqual(parseKeywords(''), [])
})

test('empty filters pass everything through untouched', () => {
  assert.deepEqual(ids(filterBaskets(items, EMPTY_FILTERS)), ['blue', 'szn', 'dogs'])
})

test('ranges are inclusive, accept shorthand strings, and half-open when one side is blank', () => {
  const f = (ranges) => ({ ...EMPTY_FILTERS, ranges })
  assert.deepEqual(ids(filterBaskets(items, f({ marketCap: { min: '100k', max: '1m' } }))), ['dogs'])
  assert.deepEqual(ids(filterBaskets(items, f({ marketCap: { min: '960k' } }))), ['blue', 'dogs'])
  assert.deepEqual(ids(filterBaskets(items, f({ change24h: { max: '0' } }))), ['dogs'])
  assert.deepEqual(ids(filterBaskets(items, f({ ageH: { max: '24' } }))), ['szn'])
  assert.deepEqual(ids(filterBaskets(items, f({ assets: { min: '2', max: '2' } }))), ['dogs'])
})

test('an unreadable bound is ignored rather than filtering everything out', () => {
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, ranges: { marketCap: { min: 'lots' } } })), ['blue', 'szn', 'dogs'])
})

test('status and type narrow categorically', () => {
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, status: 'curve' })), ['szn'])
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, status: 'graduated' })), ['blue', 'dogs'])
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, type: 'Managed' })), ['dogs'])
})

test('include keywords match name, ticker or held asset (any); exclude removes on any match', () => {
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, include: 'sol' })), ['blue', 'szn'])
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, include: 'bonk, szn' })), ['szn', 'dogs'])
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, exclude: 'jup' })), ['szn', 'dogs'])
  assert.deepEqual(ids(filterBaskets(items, { ...EMPTY_FILTERS, include: 'sol', exclude: 'season' })), ['blue'])
})

test('filters compose', () => {
  const f = { ...EMPTY_FILTERS, status: 'graduated', include: 'sol', ranges: { marketCap: { min: '1m' } } }
  assert.deepEqual(ids(filterBaskets(items, f)), ['blue'])
})

test('fmtRange renders one- and two-sided bounds with the field unit', () => {
  const mc = RANGE_FIELDS.find((r) => r.id === 'marketCap')
  const chg = RANGE_FIELDS.find((r) => r.id === 'change24h')
  const age = RANGE_FIELDS.find((r) => r.id === 'ageH')
  assert.equal(fmtRange(mc, { min: '100k', max: '1m' }), '$100K – $1M')
  assert.equal(fmtRange(mc, { min: '2.5m' }), '≥ $2.5M')
  assert.equal(fmtRange(mc, { max: '500' }), '≤ $500')
  assert.equal(fmtRange(chg, { min: '-5', max: '10' }), '-5% – 10%')
  assert.equal(fmtRange(age, { max: '24' }), '≤ 24h')
})

test('activeFilters lists only what deviates from empty, in display order', () => {
  assert.deepEqual(activeFilters(EMPTY_FILTERS), [])
  const f = { ...EMPTY_FILTERS, type: 'Mirror', exclude: 'dog', ranges: { volume24h: { min: '50k' }, marketCap: {} } }
  assert.deepEqual(
    activeFilters(f).map((a) => [a.id, a.label, a.value]),
    [
      ['type', 'Type', 'Mirror'],
      ['exclude', 'Exclude', 'dog'],
      ['volume24h', '24h volume', '≥ $50K'],
    ]
  )
})
