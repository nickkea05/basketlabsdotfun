// Shared constants for the launch flow. The numeric values here are the wire
// values the Anchor program will use; keep them in sync with the on-chain
// enum `BasketType { Fixed = 0, Mirror = 1, Managed = 2, Strategy = 3 }`.

export const BasketType = Object.freeze({
  /** Contents fixed at launch. Nobody can change them, ever. */
  Fixed: 0,
  /** Contents mirror a host wallet's holdings by value share. Creator has no control after launch. */
  Mirror: 1,
  /** Creator can rebalance at will. Highest trust requirement. */
  Managed: 2,
  /** Contents decided by code pinned at launch and run by the keeper. Nobody can change the code. */
  Strategy: 3,
})

export const BASKET_TYPES = [
  {
    id: BasketType.Fixed,
    key: 'Fixed',
    name: 'Fixed',
    tagline: 'Set once, never changes.',
    body: 'Pick the assets and weights. Once launched the basket is immutable — no creator, no host, no rebalancing. The simplest and lowest-trust form.',
    risk: 'Lowest',
    effort: 1,
    effortLabel: 'Simplest to launch',
  },
  {
    id: BasketType.Mirror,
    key: 'Mirror',
    name: 'Mirror',
    tagline: 'Tracks wallets.',
    body: 'The basket continuously sizes its holdings to match one host wallet, or an index of several, by share of value. The creator cannot alter it after launch; it is managed solely by what the hosts do.',
    risk: 'Medium',
    effort: 1,
    effortLabel: 'Paste wallets and go',
  },
  {
    id: BasketType.Managed,
    key: 'Managed',
    name: 'Managed',
    tagline: 'Run by the creator.',
    body: 'The creator can change contents and weights at any time. This is a fund, not an index — only buy one from someone you trust.',
    risk: 'Highest',
    effort: 2,
    effortLabel: 'Needs ongoing management',
  },
  {
    id: BasketType.Strategy,
    key: 'Strategy',
    name: 'Automated',
    tagline: 'Run by code you write.',
    body: 'Three functions decide what the basket holds; we run them on a schedule and rebalance to the result. The code is hashed on-chain at launch and can never be changed. For the most ambitious ideas and technical deployers — anything you can express as a query, you can turn into a basket.',
    risk: 'Medium',
    effort: 3,
    effortLabel: 'Most advanced — you write the code',
    advanced: true,
  },
]

/** Weights are expressed in basis points on-chain and must sum to exactly this. */
export const BPS_TOTAL = 10_000

/** Every basket share opens at this USD price so lifetime return is legible at a glance. */
export const INITIAL_SHARE_PRICE_USD = 1000

export const MIN_ASSETS = 2

/**
 * Only Fixed baskets cap the asset count: their contents are hand-picked and
 * immutable, so a small legible list is the point. Mirror and Managed baskets
 * are as inclusive as the hosts or the creator make them; keeping them
 * tradeable (dust, fees, turnover) is the deployer's job via the tracking
 * filters, not a base-layer limit. A program-level ceiling, if one is needed
 * for compute, is decided after measuring on devnet.
 */
export const MAX_ASSETS_FIXED = 20

export const maxAssetsFor = (basketType) => (basketType === BasketType.Fixed ? MAX_ASSETS_FIXED : Infinity)

/** Mirror baskets can track up to this many host wallets (a "top 10" index, etc.). */
export const MAX_HOSTS = 20

/** How several hosts are combined. Wire values for the on-chain enum. */
export const HostWeighting = Object.freeze({
  /** Each wallet's book counts 1/N regardless of its size. Default for indexes. */
  Equal: 0,
  /** Pool every wallet's dollars; bigger wallets weigh more. */
  Value: 1,
})

export const NAME_MAX = 32
export const SYMBOL_MAX = 10
export const DESCRIPTION_MAX = 280

export const STEPS = Object.freeze({
  Type: 'type',
  Wallet: 'wallet',
  Strategy: 'strategy',
  Test: 'test',
  Assets: 'assets',
  Allocation: 'allocation',
  Details: 'details',
  Review: 'review',
  Result: 'result',
})
