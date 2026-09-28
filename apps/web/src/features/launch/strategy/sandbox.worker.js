// Runs one strategy inside a dedicated worker with every network capability
// removed. The only way out is `ctx`, which round-trips through the main
// thread: same shape as the production isolate, minus the hard CPU kill
// (the main thread terminates us on timeout instead).

const post = self.postMessage.bind(self)

for (const k of ['fetch', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'importScripts', 'indexedDB', 'caches', 'navigator', 'Worker', 'SharedWorker']) {
  try {
    Object.defineProperty(self, k, { value: undefined, writable: false, configurable: false })
  } catch {}
}

const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'
const TOKEN_2022_PROGRAM = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'

const pending = new Map()
let seq = 0
let calls = 0
let budget = { calls: 40 }

const safe = (v) => {
  try {
    return JSON.parse(JSON.stringify(v))
  } catch {
    return String(v)
  }
}

function call(kind, ...args) {
  if (++calls > budget.calls) return Promise.reject(new Error(`data call budget of ${budget.calls} per run exceeded`))
  return new Promise((resolve, reject) => {
    const id = ++seq
    pending.set(id, { resolve, reject })
    post({ type: 'call', id, kind, args: safe(args) })
  })
}

function makeCtx() {
  return Object.freeze({
    rpc: (method, params = []) => call('rpc', method, params),
    data: Object.freeze({
      tokens: (mints) => call('tokens', mints),
      prices: (mints) => call('prices', mints),
      top: (list, opts) => call('top', list, opts),
    }),
    programs: Object.freeze({ TOKEN: TOKEN_PROGRAM, TOKEN_2022: TOKEN_2022_PROGRAM }),
    params: Object.freeze({}),
    memo: {},
    basket: Object.freeze({ current: Object.freeze({}), heldFor: () => 0 }),
    log: (...args) => post({ type: 'log', args: safe(args) }),
    now: () => Date.now(),
  })
}

self.onmessage = async (e) => {
  const msg = e.data
  if (msg.type === 'reply') {
    const p = pending.get(msg.id)
    if (!p) return
    pending.delete(msg.id)
    msg.ok ? p.resolve(msg.value) : p.reject(new Error(msg.error))
    return
  }
  if (msg.type !== 'run') return

  budget = { ...budget, ...msg.budget }
  const t0 = performance.now()
  let phase = 'load'
  try {
    const url = URL.createObjectURL(new Blob([msg.source], { type: 'text/javascript' }))
    const mod = await import(/* @vite-ignore */ url)
    URL.revokeObjectURL(url)
    const s = mod.default
    for (const fn of ['select', 'weight', 'hold']) if (typeof s?.[fn] !== 'function') throw new Error(`${fn} is not a function`)

    const ctx = makeCtx()

    phase = 'select'
    post({ type: 'phase', phase })
    const selectedRaw = await s.select(ctx)
    if (!Array.isArray(selectedRaw)) throw new Error('select() must return an array of mint addresses')
    const selected = [...new Set(selectedRaw.filter((m) => typeof m === 'string' && m.length >= 32 && m.length <= 44))]
    if (selected.length === 0) throw new Error('select() returned no mints')
    post({ type: 'selected', mints: selected })

    phase = 'weight'
    post({ type: 'phase', phase })
    const weights = await s.weight(ctx, selected)

    phase = 'hold'
    post({ type: 'phase', phase })
    // Nothing is held during a test run, so hold() has no real work. Call it
    // once anyway so a thrown error surfaces here rather than on-chain.
    const holdSample = s.hold(ctx, selected[0])
    if (typeof holdSample !== 'boolean') throw new Error('hold() must return true or false')

    post({ type: 'done', selected, weights: safe(weights), ms: Math.round(performance.now() - t0), calls })
  } catch (err) {
    post({ type: 'error', phase, message: err?.message || String(err), stack: err?.stack ? String(err.stack).split('\n').slice(0, 4).join('\n') : null })
  }
}
