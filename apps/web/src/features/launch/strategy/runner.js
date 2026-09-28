import { assembleStrategy, bookFromWeights, hashStrategy, lintStrategy } from '@basketfun/core'
import { tokensByMints, topList } from '@basketfun/data'

// Host side of the test sandbox. Spawns the worker, answers its data calls
// (this is where the network actually happens), enforces the wall clock, and
// turns the raw output into a priced book the UI can show.

export const BUDGET = Object.freeze({ calls: 40, wallMs: 20_000 })

const RPC_URL = '/api/rpc'
let rpcSeq = 0

async function rpc(method, params) {
  const res = await fetch(RPC_URL, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: ++rpcSeq, method, params }),
  })
  if (!res.ok) throw new Error(`RPC ${res.status}`)
  const body = await res.json()
  if (body.error) throw new Error(`RPC ${method}: ${body.error.message}`)
  return body.result
}

// One data call prices at most this many mints. Past it the author is
// almost certainly forgetting to filter, and the run would time out anyway.
export const MAX_MINTS_PER_CALL = 500

const tokenMap = async (mints) => {
  if (mints.length > MAX_MINTS_PER_CALL) throw new Error(`at most ${MAX_MINTS_PER_CALL} mints per call (got ${mints.length}); filter before pricing`)
  const list = await tokensByMints(mints)
  return Object.fromEntries(list.map((t) => [t.mint, t]))
}

// The Token shape strategies see (documented in public/strategy-spec.md).
// Flattens the interval stats into volume / change / traders so authors do
// not have to know Jupiter's field names.
const vol = (s) => (s ? (s.buyVolume ?? 0) + (s.sellVolume ?? 0) : null)
export function ctxToken(t) {
  if (!t) return null
  const { stats, fdv, circSupply, totalSupply, audit, website, twitter, createdAt, ...rest } = t
  return {
    ...rest,
    fdv,
    createdAt,
    volume: { m5: vol(stats?.m5), h1: vol(stats?.h1), h6: vol(stats?.h6), h24: vol(stats?.h24) },
    change: {
      m5: stats?.m5?.priceChange ?? null,
      h1: stats?.h1?.priceChange ?? null,
      h6: stats?.h6?.priceChange ?? null,
      h24: stats?.h24?.priceChange ?? null,
    },
    traders: { m5: stats?.m5?.numTraders ?? null, h1: stats?.h1?.numTraders ?? null, h6: stats?.h6?.numTraders ?? null, h24: stats?.h24?.numTraders ?? null },
    supply: { circulating: circSupply, total: totalSupply },
    audit: audit ?? null,
    links: { website: website ?? null, twitter: twitter ?? null },
  }
}

async function serve(kind, args) {
  switch (kind) {
    case 'rpc':
      return rpc(args[0], args[1] ?? [])
    case 'top':
      return (await topList(args[0], args[1] ?? {})).map(ctxToken)
    case 'tokens': {
      const map = await tokenMap(args[0] ?? [])
      return (args[0] ?? []).map((m) => ctxToken(map[m]))
    }
    case 'prices': {
      const map = await tokenMap(args[0] ?? [])
      return Object.fromEntries((args[0] ?? []).map((m) => [m, map[m]?.price ?? null]))
    }
    default:
      throw new Error(`unknown data call ${kind}`)
  }
}

/**
 * Run the three sections once against live data.
 * `onEvent` receives { type: 'phase' | 'log' | 'call' | 'selected', ... } as
 * things happen. Resolves to { ok, ... }; never rejects.
 */
export function runStrategy(sections, { onEvent = () => {}, budget = BUDGET } = {}) {
  const lint = lintStrategy(sections)
  if (lint.length) return Promise.resolve({ ok: false, phase: 'lint', message: lint.join('\n') })

  return new Promise((resolve) => {
    const worker = new Worker(new URL('./sandbox.worker.js', import.meta.url), { type: 'module' })
    const t0 = performance.now()
    const callLog = []
    let settled = false

    const finish = (r) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      worker.terminate()
      resolve({ ...r, ms: Math.round(performance.now() - t0), calls: callLog })
    }

    const timer = setTimeout(() => finish({ ok: false, phase: 'timeout', message: `Run exceeded ${budget.wallMs / 1000}s and was stopped` }), budget.wallMs)

    worker.onerror = (e) => finish({ ok: false, phase: 'load', message: e.message || 'The strategy failed to load (syntax error?)' })

    worker.onmessage = async (e) => {
      const m = e.data
      switch (m.type) {
        case 'call': {
          const label =
            m.kind === 'rpc'
              ? `rpc ${m.args[0]}`
              : m.kind === 'top'
                ? `data.top(${m.args[0]}, ${JSON.stringify(m.args[1] ?? {})})`
                : `data.${m.kind}(${(m.args[0] ?? []).length})`
          const started = performance.now()
          try {
            const value = await serve(m.kind, m.args)
            const ms = Math.round(performance.now() - started)
            callLog.push({ label, ms, ok: true })
            onEvent({ type: 'call', label, ms, ok: true })
            worker.postMessage({ type: 'reply', id: m.id, ok: true, value })
          } catch (err) {
            const ms = Math.round(performance.now() - started)
            callLog.push({ label, ms, ok: false, error: err.message })
            onEvent({ type: 'call', label, ms, ok: false, error: err.message })
            worker.postMessage({ type: 'reply', id: m.id, ok: false, error: err.message })
          }
          break
        }
        case 'phase':
        case 'log':
        case 'selected':
          onEvent(m)
          break
        case 'error':
          finish({ ok: false, phase: m.phase, message: m.message, stack: m.stack })
          break
        case 'done': {
          try {
            const book = bookFromWeights(m.weights)
            const map = await tokenMap(book.map((r) => r.mint))
            const priced = book.map((r) => ({ ...r, token: map[r.mint] ?? { mint: r.mint, symbol: r.mint.slice(0, 4), name: 'Unknown token' } }))
            const hash = await hashStrategy(sections)
            finish({ ok: true, selected: m.selected, book: priced, workerMs: m.ms, hash, source: assembleStrategy(sections) })
          } catch (err) {
            finish({ ok: false, phase: 'weight', message: err.message })
          }
          break
        }
      }
    }

    worker.postMessage({ type: 'run', source: assembleStrategy(sections), budget })
  })
}
