import { useReducer } from 'react'
import { BasketType, HostWeighting, STEPS, combineHosts } from '@basketfun/core'

export const emptyDraft = () => ({
  step: STEPS.Type,
  basketType: null,
  hostWallet: '', // address being typed
  hosts: [], // [{ address, snapshot: { rows, totalUsd, truncated, tokenAccounts } }]
  hostWeighting: HostWeighting.Equal,
  assets: [], // normalised tokens, in the order they were picked
  weights: {}, // mint -> bps (Fixed / Managed / Strategy)
  strategy: { select: '', weight: '', hold: '' }, // Strategy source, one entry per section
  strategyHash: null, // sha256 of the assembled module, set by a successful test run
  strategyRun: null, // last test run result
  meta: { name: '', symbol: '', description: '', image: null, twitter: '', telegram: '', website: '' },
  result: null,
})

function reducer(d, a) {
  switch (a.type) {
    case 'step':
      return { ...d, step: a.step }
    case 'basketType':
      return { ...d, basketType: a.value }
    case 'hostWallet':
      return { ...d, hostWallet: a.value }
    case 'addHost': {
      if (d.hosts.some((h) => h.address === a.address)) return d
      const hosts = [...d.hosts, { address: a.address, snapshot: a.snapshot }]
      return { ...d, hosts, hostWallet: '', assets: combineHosts(hosts, d.hostWeighting).assets }
    }
    case 'removeHost': {
      const hosts = d.hosts.filter((h) => h.address !== a.address)
      return { ...d, hosts, assets: combineHosts(hosts, d.hostWeighting).assets }
    }
    case 'hostWeighting':
      return { ...d, hostWeighting: a.value, assets: combineHosts(d.hosts, a.value).assets }
    case 'addAsset':
      if (d.assets.some((t) => t.mint === a.token.mint)) return d
      return { ...d, assets: [...d.assets, a.token] }
    case 'removeAsset': {
      const weights = { ...d.weights }
      delete weights[a.mint]
      return { ...d, assets: d.assets.filter((t) => t.mint !== a.mint), weights }
    }
    case 'weight':
      return { ...d, weights: { ...d.weights, [a.mint]: a.bps } }
    case 'weights':
      return { ...d, weights: a.value }
    // Editing code invalidates the last run: the book and hash must come from
    // the code that will actually be pinned.
    case 'strategySection':
      return { ...d, strategy: { ...d.strategy, [a.id]: a.value }, strategyRun: null, strategyHash: null, assets: [], weights: {} }
    case 'strategyLoad':
      return { ...d, strategy: { ...a.value }, strategyRun: null, strategyHash: null, assets: [], weights: {} }
    case 'strategyRun': {
      const r = a.value
      if (!r.ok) return { ...d, strategyRun: r, strategyHash: null, assets: [], weights: {} }
      return {
        ...d,
        strategyRun: r,
        strategyHash: r.hash,
        assets: r.book.map((row) => row.token),
        weights: Object.fromEntries(r.book.map((row) => [row.mint, row.weight_bps])),
      }
    }
    case 'meta':
      return { ...d, meta: { ...d.meta, ...a.value } }
    case 'result':
      return { ...d, result: a.value, step: STEPS.Result }
    case 'reset':
      return emptyDraft()
    default:
      return d
  }
}

export function useDraft() {
  return useReducer(reducer, undefined, emptyDraft)
}

/** Ordered step list for the chosen basket type. */
export function stepsFor(basketType) {
  switch (basketType) {
    case BasketType.Mirror:
      return [STEPS.Type, STEPS.Wallet, STEPS.Allocation, STEPS.Details, STEPS.Review]
    case BasketType.Strategy:
      return [STEPS.Type, STEPS.Strategy, STEPS.Test, STEPS.Details, STEPS.Review]
    default:
      return [STEPS.Type, STEPS.Assets, STEPS.Allocation, STEPS.Details, STEPS.Review]
  }
}
