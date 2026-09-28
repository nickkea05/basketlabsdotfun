import { test } from 'node:test'
import assert from 'node:assert/strict'
import { BasketType, SEARCH_PAGE_SIZE, looksLikeAddress, pageResults, scoreBasket, searchBaskets } from '../src/index.js'

const mk = (o) => ({ ca: '7xKq…9bA2', type: BasketType.Fixed, ageH: 100, marketCap: 1, volume24h: 1, change24h: 0, basket: [], ...o })

const items = [
  mk({ id: 'sol', name: 'Solana Blue', ticker: 'BLUE', marketCap: 4_000_000, volume24h: 600_000, ageH: 144, basket: [{ symbol: 'SOL' }, { symbol: 'JUP' }] }),
  mk({ id: 'season', name: 'Solana Season', ticker: 'SZN', marketCap: 61_000, volume24h: 88_000, ageH: 2, raised: 58, target: 85, type: BasketType.Mirror }),
  mk({ id: 'majors', name: 'Majors', ticker: 'MAJ', marketCap: 2_800_000, volume24h: 400_000, ageH: 216, basket: [{ symbol: 'wBTC' }, { symbol: 'SOL' }] }),
  mk({ id: 'dogs', name: 'Dog Park', ticker: 'DOGS', marketCap: 960_000, volume24h: 274_000, ageH: 336, type: BasketType.Managed, ca: 'DoGs…park' }),
]

test('empty query returns everything, biggest first', () => {
  const r = searchBaskets(items, '')
  assert.equal(r.length, items.length)
  assert.deepEqual(
    r.map((t) => t.id),
    ['sol', 'majors', 'dogs', 'season']
  )
})

test('exact ticker outranks a name prefix, which outranks a name substring', () => {
  assert.ok(scoreBasket(items[0], 'blue') > scoreBasket(items[0], 'solana'))
  assert.ok(scoreBasket(items[0], 'solana') > scoreBasket(items[0], 'ana b'))
  assert.equal(scoreBasket(items[0], '$blue'), scoreBasket(items[0], 'blue'))
})

test('query is matched case-insensitively across name, ticker, address and contents', () => {
  assert.deepEqual(
    searchBaskets(items, 'SOLANA').map((t) => t.id),
    ['sol', 'season']
  )
  assert.deepEqual(
    searchBaskets(items, 'maj').map((t) => t.id),
    ['majors']
  )
  // "SOL" is both a name prefix and a held asset; both baskets holding it appear.
  const sol = searchBaskets(items, 'sol').map((t) => t.id)
  assert.ok(sol.includes('majors'))
  assert.equal(sol[0], 'sol')
})

test('match restricts which field is searched', () => {
  const f = (match) => ({ sort: 'relevance', match, age: 'all', type: 'all', status: 'all' })
  assert.deepEqual(
    searchBaskets(items, 'sol', f('assets')).map((t) => t.id),
    ['sol', 'majors']
  )
  assert.deepEqual(
    searchBaskets(items, 'sol', f('ticker')).map((t) => t.id),
    []
  )
  assert.deepEqual(
    searchBaskets(items, 'dogs', f('address')).map((t) => t.id),
    ['dogs']
  )
})

test('truncated addresses match a full address with the same head and tail', () => {
  assert.ok(scoreBasket(items[0], '7xkqaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa9ba2', 'address') > 0)
  assert.equal(scoreBasket(items[0], '7xkqaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa9ba3', 'address'), 0)
})

test('age, type and status filters compose with the query', () => {
  const base = { sort: 'relevance', match: 'all', age: 'all', type: 'all', status: 'all' }
  assert.deepEqual(
    searchBaskets(items, '', { ...base, age: '24h' }).map((t) => t.id),
    ['season']
  )
  assert.deepEqual(
    searchBaskets(items, '', { ...base, type: 'Managed' }).map((t) => t.id),
    ['dogs']
  )
  assert.deepEqual(
    searchBaskets(items, '', { ...base, status: 'curve' }).map((t) => t.id),
    ['season']
  )
  assert.deepEqual(
    searchBaskets(items, 'solana', { ...base, status: 'graduated' }).map((t) => t.id),
    ['sol']
  )
})

test('explicit sorts ignore relevance', () => {
  const base = { sort: 'relevance', match: 'all', age: 'all', type: 'all', status: 'all' }
  assert.deepEqual(
    searchBaskets(items, '', { ...base, sort: 'new' }).map((t) => t.id),
    ['season', 'sol', 'majors', 'dogs']
  )
  assert.deepEqual(
    searchBaskets(items, '', { ...base, sort: 'old' }).map((t) => t.id),
    ['dogs', 'majors', 'sol', 'season']
  )
  assert.deepEqual(
    searchBaskets(items, '', { ...base, sort: 'vol' }).map((t) => t.id),
    ['sol', 'majors', 'dogs', 'season']
  )
})

test('paging clamps and reports 1-based ranges', () => {
  const many = Array.from({ length: 50 }, (_, i) => mk({ id: `b${i}`, name: `B${i}`, ticker: `B${i}` }))
  const p1 = pageResults(many, 1)
  assert.equal(p1.items.length, SEARCH_PAGE_SIZE)
  assert.deepEqual([p1.from, p1.to, p1.total, p1.pages], [1, 24, 50, 3])
  const p3 = pageResults(many, 99)
  assert.deepEqual([p3.page, p3.items.length, p3.from, p3.to], [3, 2, 49, 50])
  const empty = pageResults([], 1)
  assert.deepEqual([empty.page, empty.pages, empty.from, empty.to, empty.total], [1, 1, 0, 0, 0])
})

test('looksLikeAddress accepts base58 mints and rejects words', () => {
  assert.ok(looksLikeAddress('So11111111111111111111111111111111111111112'))
  assert.ok(looksLikeAddress(' EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v '))
  assert.equal(looksLikeAddress('solana blue'), false)
  assert.equal(looksLikeAddress('0x0000000000000000000000000000000000000000'), false)
})
