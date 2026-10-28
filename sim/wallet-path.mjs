// Exercises the browser's send path (WalletProvider.sendInstructions on the
// fork) without a browser: a Wallet Standard-shaped signer, the dev server's
// /api/chain proxy, the test-funds cheatcodes, and one real basket mint.
//
//   npm run dev -w @basketfun/web   then   node sim/wallet-path.mjs

import { Connection, Keypair, LAMPORTS_PER_SOL, PublicKey, VersionedTransaction } from '@solana/web3.js'
import { BasketClient, buildTransaction, confirmSignature, depositsForShares, mintBounds, mintQuote } from '@basketfun/solana/basket'
import { lastBasket } from './lib.mjs'

const RPC = process.env.SIM_WEB_RPC ?? 'http://localhost:5180/api/chain'
const connection = new Connection(RPC, 'confirmed')
const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'

async function rpc(method, params) {
  const res = await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) })
  const body = await res.json()
  if (body.error) throw new Error(`${method}: ${body.error.message}`)
  return body.result
}

/** What `solana:signTransaction` does: sign the serialized bytes, hand them back. */
function standardWallet(keypair) {
  const account = { address: keypair.publicKey.toBase58(), chains: ['solana:mainnet'] }
  return {
    account,
    features: {
      'solana:signTransaction': {
        signTransaction: async ({ transaction }) => {
          const tx = VersionedTransaction.deserialize(transaction)
          tx.sign([keypair])
          return [{ signedTransaction: tx.serialize() }]
        },
      },
    },
  }
}

const user = Keypair.generate()
const wallet = standardWallet(user)
console.log(`proxy slot ${await rpc('getSlot', [])}`)

await rpc('surfnet_setAccount', [wallet.account.address, { lamports: 100 * LAMPORTS_PER_SOL }])
const client = new BasketClient(connection)
const { shareMint, lut } = lastBasket()
const state = await client.loadState(shareMint)
const gross = state.holderShares / 5n
const deposits = depositsForShares(state, gross)
for (const [i, p] of state.book.entries()) await rpc('surfnet_setTokenAccount', [wallet.account.address, p.mint.toBase58(), { amount: Number(deposits[i]) }, TOKEN_PROGRAM])
console.log(`funded ${(await connection.getBalance(user.publicKey)) / LAMPORTS_PER_SOL} SOL and ${state.book.length} components`)

const q = mintQuote(state, deposits)
if (q.belowMinimum) throw new Error(`buy of ${q.impliedTotalLamports} lamports is under the ${q.minLamports} minimum`)
const ix = await client.mintIx({ payer: user.publicKey, state, deposits, ...mintBounds(q) })
const { transaction, lastValidBlockHeight } = await buildTransaction(connection, {
  payer: new PublicKey(wallet.account.address),
  instructions: [ix],
  lookupTables: [lut ? new PublicKey(lut) : null].filter(Boolean),
})
const [out] = await wallet.features['solana:signTransaction'].signTransaction({ account: wallet.account, transaction: transaction.serialize() })
const signature = await connection.sendRawTransaction(VersionedTransaction.deserialize(out.signedTransaction).serialize())
await confirmSignature(connection, signature, lastValidBlockHeight)
const shares = await connection.getTokenAccountBalance(PublicKey.findProgramAddressSync([user.publicKey.toBuffer(), new PublicKey(TOKEN_PROGRAM).toBuffer(), shareMint.toBuffer()], new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL'))[0])
const ok = BigInt(shares.value.amount) === q.toBuyer
console.log(`mint ${signature}\nshares ${shares.value.amount} (quote ${q.toBuyer}) ${ok ? 'ok' : 'MISMATCH'}`)
process.exit(ok ? 0 : 1)
