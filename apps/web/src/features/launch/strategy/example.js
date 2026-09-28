// The worked example shown beside the editors: Mirror, written as a strategy
// against raw RPC. Deliberately plain so it reads as a template, not a trick.

export const EXAMPLE_WALLET = 'AVzP2GeRmqGphJsMxWoqjpUifPpCret7LqWhD8NWQK49'

export const EXAMPLE = {
  select: `// Mirror one wallet: whatever it holds, we hold.
async function select(ctx) {
  const owner = '${EXAMPLE_WALLET}'

  // Raw Solana RPC. Any read-only method works; this one lists
  // every SPL token account the wallet owns, already decoded.
  const { value } = await ctx.rpc('getTokenAccountsByOwner', [
    owner,
    { programId: ctx.programs.TOKEN },
    { encoding: 'jsonParsed' },
  ])

  // Stash balances for weight(); ctx.memo lives for the whole run.
  ctx.memo.amounts = {}
  for (const acc of value) {
    const info = acc.account.data.parsed.info
    const ui = info.tokenAmount.uiAmount
    if (ui > 0) ctx.memo.amounts[info.mint] = ui
  }

  return Object.keys(ctx.memo.amounts)
}`,
  weight: `// Size each position by its dollar value in the source wallet.
async function weight(ctx, mints) {
  const prices = await ctx.data.prices(mints) // { mint: usd }

  const out = {}
  for (const mint of mints) {
    const usd = (ctx.memo.amounts[mint] ?? 0) * (prices[mint] ?? 0)
    if (usd >= 1) out[mint] = usd // any scale; normalised to 100%
  }
  return out
}`,
  hold: `// Ride out a position for six hours after the wallet drops it,
// so a quick round-trip does not churn the basket.
function hold(ctx, mint) {
  return ctx.basket.heldFor(mint) < 6 * 3600
}`,
}
