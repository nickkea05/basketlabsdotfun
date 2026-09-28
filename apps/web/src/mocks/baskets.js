// Sample baskets for the prototype UI. Replaced by indexer data once baskets
// exist on-chain; nothing here is real.
//
// `type` is a BasketType wire value (0 Fixed, 1 Mirror, 2 Managed, 3 Strategy);
// `ageH` is hours since launch so filters can sort by newest.

const mk = (o) => ({ ca: '7xKq…9bA2', ...o })

export const graduated = [
  mk({
    id: 'blue',
    type: 0,
    ageH: 144,
    name: 'Solana Blue',
    ticker: 'BLUE',
    marketCap: 4_120_000,
    price: 0.0412,
    change24h: 3.8,
    volume24h: 612_000,
    nav: 0.0398,
    age: '6d',
    tint: '215 12%',
    basket: [
      { symbol: 'SOL', weight: 50 },
      { symbol: 'JUP', weight: 25 },
      { symbol: 'RAY', weight: 25 },
    ],
  }),
  mk({
    id: 'majors',
    type: 0,
    ageH: 216,
    name: 'Majors',
    ticker: 'MAJ',
    marketCap: 2_870_000,
    price: 0.0287,
    change24h: -1.2,
    volume24h: 401_000,
    nav: 0.0291,
    age: '9d',
    tint: '30 6%',
    basket: [
      { symbol: 'wBTC', weight: 50 },
      { symbol: 'wETH', weight: 30 },
      { symbol: 'SOL', weight: 20 },
    ],
  }),
  mk({
    id: 'yield',
    type: 1,
    ageH: 264,
    name: 'Yield Stack',
    ticker: 'YLD',
    marketCap: 1_940_000,
    price: 0.0194,
    change24h: 0.6,
    volume24h: 188_000,
    nav: 0.019,
    age: '11d',
    tint: '150 8%',
    basket: [
      { symbol: 'JitoSOL', weight: 40 },
      { symbol: 'mSOL', weight: 30 },
      { symbol: 'INF', weight: 30 },
    ],
  }),
  mk({
    id: 'perps',
    type: 2,
    ageH: 72,
    name: 'Perp Desk',
    ticker: 'PERP',
    marketCap: 1_310_000,
    price: 0.0131,
    change24h: 12.4,
    volume24h: 522_000,
    nav: 0.0119,
    age: '3d',
    tint: '340 10%',
    basket: [
      { symbol: 'DRIFT', weight: 40 },
      { symbol: 'JUP', weight: 35 },
      { symbol: 'ZETA', weight: 25 },
    ],
  }),
  mk({
    id: 'dogs',
    type: 0,
    ageH: 336,
    name: 'Dog Park',
    ticker: 'DOGS',
    marketCap: 960_000,
    price: 0.0096,
    change24h: -6.9,
    volume24h: 274_000,
    nav: 0.0103,
    age: '14d',
    tint: '45 10%',
    basket: [
      { symbol: 'WIF', weight: 34 },
      { symbol: 'BONK', weight: 33 },
      { symbol: 'MYRO', weight: 33 },
    ],
  }),
  mk({
    id: 'infra',
    type: 3,
    ageH: 432,
    name: 'Infra',
    ticker: 'INFRA',
    marketCap: 742_000,
    price: 0.0074,
    change24h: 2.1,
    volume24h: 96_000,
    nav: 0.0072,
    age: '18d',
    tint: '200 6%',
    basket: [
      { symbol: 'PYTH', weight: 40 },
      { symbol: 'HNT', weight: 30 },
      { symbol: 'RENDER', weight: 30 },
    ],
  }),
  mk({
    id: 'lst',
    type: 0,
    ageH: 504,
    name: 'Staked',
    ticker: 'STK',
    marketCap: 611_000,
    price: 0.0061,
    change24h: 0.2,
    volume24h: 41_000,
    nav: 0.0061,
    age: '21d',
    tint: '170 5%',
    basket: [
      { symbol: 'JitoSOL', weight: 50 },
      { symbol: 'bSOL', weight: 50 },
    ],
  }),
  mk({
    id: 'stables',
    type: 0,
    ageH: 600,
    name: 'Dollar',
    ticker: 'USDX',
    marketCap: 488_000,
    price: 1.0004,
    change24h: 0.0,
    volume24h: 22_000,
    nav: 1.0001,
    age: '25d',
    tint: '0 0%',
    basket: [
      { symbol: 'USDC', weight: 60 },
      { symbol: 'USDT', weight: 30 },
      { symbol: 'PYUSD', weight: 10 },
    ],
  }),
  mk({
    id: 'ai',
    type: 2,
    ageH: 48,
    name: 'Agents',
    ticker: 'AGNT',
    marketCap: 355_000,
    price: 0.0035,
    change24h: 27.3,
    volume24h: 301_000,
    nav: 0.0028,
    age: '2d',
    tint: '270 10%',
    basket: [
      { symbol: 'AI16Z', weight: 50 },
      { symbol: 'GRIFFAIN', weight: 30 },
      { symbol: 'ARC', weight: 20 },
    ],
  }),
  mk({
    id: 'memes',
    type: 1,
    ageH: 720,
    name: 'Blue Chip Memes',
    ticker: 'MEME',
    marketCap: 290_000,
    price: 0.0029,
    change24h: -3.4,
    volume24h: 88_000,
    nav: 0.003,
    age: '30d',
    tint: '20 8%',
    basket: [
      { symbol: 'WIF', weight: 40 },
      { symbol: 'POPCAT', weight: 30 },
      { symbol: 'BONK', weight: 30 },
    ],
  }),
]

export const live = [
  mk({
    id: 'l1',
    type: 0,
    ageH: 2,
    name: 'Solana Season',
    ticker: 'SZN',
    marketCap: 61_000,
    price: 0.00061,
    change24h: 41.0,
    volume24h: 88_000,
    nav: 0.00055,
    age: '2h',
    tint: '215 12%',
    raised: 58.4,
    target: 85,
    basket: [
      { symbol: 'SOL', weight: 70 },
      { symbol: 'JUP', weight: 30 },
    ],
  }),
  mk({
    id: 'l2',
    type: 1,
    ageH: 5,
    name: 'Cats Only',
    ticker: 'CATS',
    marketCap: 24_000,
    price: 0.00024,
    change24h: 9.5,
    volume24h: 31_000,
    nav: 0.00021,
    age: '5h',
    tint: '340 10%',
    raised: 22.1,
    target: 85,
    basket: [
      { symbol: 'POPCAT', weight: 50 },
      { symbol: 'MEW', weight: 50 },
    ],
  }),
  mk({
    id: 'l3',
    type: 3,
    ageH: 0.67,
    name: 'Real Yield',
    ticker: 'RY',
    marketCap: 9_800,
    price: 0.000098,
    change24h: 2.0,
    volume24h: 6_000,
    nav: 0.000097,
    age: '40m',
    tint: '150 8%',
    raised: 8.9,
    target: 85,
    basket: [
      { symbol: 'JitoSOL', weight: 60 },
      { symbol: 'INF', weight: 40 },
    ],
  }),
]

/** Every basket the site knows about. The indexer replaces this. */
export const all = [...live, ...graduated]

export const activity = [
  { side: 'buy', amount: '2.40 SOL', tokens: '58,252', who: '3Tmi…9jMo', ago: '12s' },
  { side: 'sell', amount: '0.85 SOL', tokens: '20,631', who: 'H92i…eiai', ago: '48s' },
  { side: 'buy', amount: '6.00 SOL', tokens: '145,630', who: '3hHv…dEhL', ago: '2m' },
  { side: 'buy', amount: '0.20 SOL', tokens: '4,854', who: '8V4k…Tf72', ago: '4m' },
  { side: 'sell', amount: '3.10 SOL', tokens: '75,242', who: 'GZet…J2vq', ago: '7m' },
]

// Deterministic, lightly smoothed series so the chart is stable between renders.
export function series(seed = 7, n = 140) {
  let s = seed
  const rand = () => {
    s = (s * 1664525 + 1013904223) % 4294967296
    return s / 4294967296
  }
  const raw = []
  let v = 100
  for (let i = 0; i < n; i++) {
    v = v * (1 + (rand() - 0.47) * 0.03)
    raw.push(v)
  }
  return raw.map((_, i) => {
    const a = raw[Math.max(0, i - 1)],
      b = raw[i],
      c = raw[Math.min(n - 1, i + 1)]
    return (a + 2 * b + c) / 4
  })
}
