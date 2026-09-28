// Strategy baskets: contents decided by three user-written functions that the
// keeper runs on a schedule inside a capability-less sandbox. This module is
// the contract. Nothing here executes code; the runner (browser today, isolate
// later) does that against exactly this shape.

import { normalizeWeights } from './params.js'

/**
 * The three sections, in run order. `signature` is what the section must
 * declare; `ask` is the one-line description shown next to the editor.
 */
export const STRATEGY_SECTIONS = [
  {
    id: 'select',
    file: 'select.js',
    signature: 'async function select(ctx): Promise<string[]>',
    title: 'Select',
    ask: 'Return the mints that belong in the basket right now.',
    detail:
      'Called first, once per rebalance. Read whatever you need through ctx and return an array of mint addresses. Duplicates are collapsed; unknown or unpriceable mints are dropped before weight() runs.',
  },
  {
    id: 'weight',
    file: 'weight.js',
    signature: 'async function weight(ctx, mints): Promise<Record<string, number>>',
    title: 'Weight',
    ask: 'Return a relative size for each selected mint.',
    detail:
      'Receives the mints select() returned. Return an object of mint → number on any scale; the runtime normalises to 100% in basis points. Zero or negative values remove the mint. Anything stashed on ctx.memo in select() is still there.',
  },
  {
    id: 'hold',
    file: 'hold.js',
    signature: 'function hold(ctx, mint): boolean',
    title: 'Hold',
    ask: 'Decide whether to keep a position that select() just dropped.',
    detail:
      'Called for each mint currently in the basket that select() left out. Return true to keep it at its current weight for one more cycle, false to sell. Use ctx.basket.heldFor(mint) for the seconds since entry. Return false to always follow select().',
  },
]

const SECTION_IDS = STRATEGY_SECTIONS.map((s) => s.id)

/** Join the sections into one ES module with a default export the runner imports. */
export function assembleStrategy(sections) {
  return `${SECTION_IDS.map((id) => sections[id] ?? '').join('\n\n')}\n\nexport default { select, weight, hold }\n`
}

// Capabilities the sandbox does not grant. Using one is an error at lint
// time rather than a mysterious failure at run time. Each entry: a regex and
// the word to show the author.
const FORBIDDEN = [
  [/(^|[^\w.$])import\s*[\s({'"*]/m, 'import'],
  [/(^|[^\w.$])export\s+/m, 'export'],
  [/(^|[^\w.$])eval\s*\(/, 'eval'],
  [/(^|[^\w.$])Function\s*\(/, 'Function'],
  [/(^|[^\w.$])globalThis\b/, 'globalThis'],
  [/(^|[^\w.$])self\s*[.[]/, 'self'],
  [/(^|[^\w.$])window\b/, 'window'],
  [/(^|[^\w.$])fetch\s*\(/, 'fetch'],
  [/(^|[^\w.$])XMLHttpRequest\b/, 'XMLHttpRequest'],
  [/(^|[^\w.$])WebSocket\b/, 'WebSocket'],
  [/(^|[^\w.$])importScripts\b/, 'importScripts'],
  [/(^|[^\w.$])postMessage\s*\(/, 'postMessage'],
  [/(^|[^\w.$])require\s*\(/, 'require'],
  [/(^|[^\w.$])process\s*\./, 'process'],
]

const declares = (src, id) => new RegExp(`(^|[^\\w.$])(async\\s+)?function\\s*\\*?\\s*${id}\\s*\\(|(^|[^\\w.$])(const|let|var)\\s+${id}\\s*=`, 'm').test(src)

/**
 * Static checks before anything runs. Returns human-readable problems, empty
 * when the sections are acceptable. Cheap, and turns most bad submissions
 * into an instant message instead of a runtime kill.
 */
export function lintStrategy(sections) {
  const errs = []
  for (const id of SECTION_IDS) {
    const src = sections?.[id] ?? ''
    if (!src.trim()) {
      errs.push(`${id}: section is empty`)
      continue
    }
    for (const [re, word] of FORBIDDEN) {
      if (re.test(src)) errs.push(`${id}: ${word} is not available inside a strategy`)
    }
    if (!declares(src, id)) errs.push(`${id}: must declare a function named ${id}`)
  }
  return errs
}

/**
 * Turn weight()'s return value into the on-chain book: positive weights only,
 * normalised to BPS_TOTAL, largest first. Throws with a message meant for
 * the author when the shape is wrong.
 */
export function bookFromWeights(out) {
  if (out == null || typeof out !== 'object' || Array.isArray(out)) throw new Error('weight() must return an object of { mint: number }')
  const raw = {}
  for (const [mint, v] of Object.entries(out)) {
    if (typeof v !== 'number' || !Number.isFinite(v)) throw new Error(`weight() returned a non-number for ${mint}`)
    if (v > 0) raw[mint] = v
  }
  if (Object.keys(raw).length === 0) throw new Error('weight() returned an empty book: every weight was zero or negative')
  const bps = normalizeWeights(raw)
  return Object.entries(bps)
    .map(([mint, weight_bps]) => ({ mint, weight_bps }))
    .sort((a, b) => b.weight_bps - a.weight_bps)
}

/** sha256 of the assembled module, hex. What gets pinned on-chain. */
export async function hashStrategy(sections) {
  const bytes = new TextEncoder().encode(assembleStrategy(sections))
  const digest = await globalThis.crypto.subtle.digest('SHA-256', bytes)
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}
