import { test } from 'node:test'
import assert from 'node:assert/strict'
import {
  BPS_TOTAL,
  BasketType,
  HostWeighting,
  STRATEGY_SECTIONS,
  assembleStrategy,
  bookFromWeights,
  buildLaunchParams,
  hashStrategy,
  lintStrategy,
  maxAssetsFor,
  sumBps,
  validateParams,
} from '../src/index.js'

const good = {
  select: `async function select(ctx) {\n  const { value } = await ctx.rpc('getTokenAccountsByOwner', ['x', { programId: ctx.programs.TOKEN }, { encoding: 'jsonParsed' }])\n  return value.map((a) => a.account.data.parsed.info.mint)\n}`,
  weight: `async function weight(ctx, mints) {\n  return Object.fromEntries(mints.map((m) => [m, 1]))\n}`,
  hold: `function hold(ctx, mint) {\n  return ctx.basket.heldFor(mint) < 6 * 3600\n}`,
}

test('the strategy contract is exactly select, weight, hold in that order', () => {
  assert.deepEqual(
    STRATEGY_SECTIONS.map((s) => s.id),
    ['select', 'weight', 'hold']
  )
})

test('assembleStrategy joins the three sections and exports them as one module', () => {
  const src = assembleStrategy(good)
  assert.ok(src.includes(good.select))
  assert.ok(src.includes(good.weight))
  assert.ok(src.includes(good.hold))
  assert.ok(/export default \{\s*select,\s*weight,\s*hold\s*\}/.test(src))
})

test('lintStrategy passes the worked example', () => {
  assert.deepEqual(lintStrategy(good), [])
})

test('lintStrategy rejects each capability the sandbox does not grant, naming the section', () => {
  const bad = (id, body) => lintStrategy({ ...good, [id]: body })
  assert.match(bad('select', `import x from 'y'\n${good.select}`)[0], /select.*import/i)
  assert.match(bad('weight', `async function weight(ctx, m) { return fetch('https://x') }`)[0], /weight.*fetch/i)
  assert.match(bad('hold', `function hold() { return eval('1') }`)[0], /hold.*eval/i)
  assert.match(bad('hold', `function hold() { return new Function('return 1')() }`)[0], /hold.*Function/i)
  assert.match(bad('select', `async function select() { return globalThis.secret }`)[0], /select.*globalThis/i)
})

test('lintStrategy requires each section to declare its function', () => {
  const errs = lintStrategy({ ...good, weight: `const w = 1` })
  assert.equal(errs.length, 1)
  assert.match(errs[0], /weight.*declare/i)
})

test('lintStrategy reports empty sections', () => {
  const errs = lintStrategy({ ...good, hold: '   ' })
  assert.match(errs[0], /hold.*empty/i)
})

test('bookFromWeights drops non-positive weights, normalises to BPS_TOTAL, sorts largest first', () => {
  const book = bookFromWeights({ a: 3, b: 0, c: -1, d: 1 })
  assert.deepEqual(
    book.map((r) => r.mint),
    ['a', 'd']
  )
  assert.equal(sumBps(Object.fromEntries(book.map((r) => [r.mint, r.weight_bps]))), BPS_TOTAL)
  assert.equal(book[0].weight_bps, 7500)
})

test('bookFromWeights rejects non-object or non-numeric output with a readable error', () => {
  assert.throws(() => bookFromWeights([1, 2]), /object/i)
  assert.throws(() => bookFromWeights({ a: 'lots' }), /number/i)
  assert.throws(() => bookFromWeights({}), /empty/i)
})

test('hashStrategy is a stable 64-hex sha256 of the assembled module', async () => {
  const h1 = await hashStrategy(good)
  const h2 = await hashStrategy({ ...good })
  const h3 = await hashStrategy({ ...good, hold: 'function hold() { return false }' })
  assert.match(h1, /^[0-9a-f]{64}$/)
  assert.equal(h1, h2)
  assert.notEqual(h1, h3)
})

test('Strategy baskets are uncapped and carry the code hash and source in the payload', async () => {
  assert.equal(maxAssetsFor(BasketType.Strategy), Infinity)
  const draft = {
    basketType: BasketType.Strategy,
    assets: [{ mint: 'A' }, { mint: 'B' }],
    weights: { A: 6000, B: 4000 },
    hosts: [],
    hostWeighting: HostWeighting.Equal,
    strategy: good,
    strategyHash: await hashStrategy(good),
    meta: { name: 'n', symbol: 's', description: '', image: null, twitter: '', telegram: '', website: '' },
  }
  const p = buildLaunchParams(draft)
  assert.equal(p.basket_type, BasketType.Strategy)
  assert.equal(p.strategy.hash, draft.strategyHash)
  assert.deepEqual(p.strategy.source, good)
  assert.deepEqual(validateParams(p), [])
  assert.match(validateParams({ ...p, strategy: null }).join(' '), /strategy/i)
})
