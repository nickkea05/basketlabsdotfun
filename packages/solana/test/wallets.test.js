import { test } from 'node:test'
import assert from 'node:assert/strict'
import { explorerUrl, rankWallets, solanaAddresses } from '../src/index.js'

// Minimal Wallet Standard shapes. Only the fields our code reads.
const wallet = (name, chains, features = ['standard:connect']) => ({
  name,
  icon: 'data:image/svg+xml;base64,',
  chains,
  features: Object.fromEntries(features.map((f) => [f, {}])),
})

test('rankWallets drops wallets with no Solana chain', () => {
  const out = rankWallets([wallet('MetaMask', ['eip155:1']), wallet('Solflare', ['solana:mainnet'])])
  assert.deepEqual(
    out.map((w) => w.name),
    ['Solflare']
  )
})

test('rankWallets drops wallets that cannot connect', () => {
  const out = rankWallets([wallet('Broken', ['solana:mainnet'], [])])
  assert.equal(out.length, 0)
})

test('rankWallets puts Phantom first, then the rest alphabetically', () => {
  const out = rankWallets([wallet('Solflare', ['solana:mainnet']), wallet('Backpack', ['solana:mainnet']), wallet('Phantom', ['solana:mainnet', 'eip155:1'])])
  assert.deepEqual(
    out.map((w) => w.name),
    ['Phantom', 'Backpack', 'Solflare']
  )
})

test('rankWallets dedupes by name, keeping the first registration', () => {
  const a = wallet('Phantom', ['solana:mainnet'])
  const b = wallet('Phantom', ['solana:devnet'])
  const out = rankWallets([a, b])
  assert.equal(out.length, 1)
  assert.equal(out[0], a)
})

test('solanaAddresses returns only addresses usable on Solana', () => {
  const accounts = [
    { address: 'So1anaAddr', chains: ['solana:mainnet'] },
    { address: '0xevm', chains: ['eip155:1'] },
    { address: 'Another', chains: ['solana:devnet', 'solana:mainnet'] },
  ]
  assert.deepEqual(solanaAddresses(accounts), ['So1anaAddr', 'Another'])
})

test('explorerUrl points at the account page, with cluster only when not mainnet', () => {
  assert.equal(explorerUrl('abc'), 'https://solscan.io/account/abc')
  assert.equal(explorerUrl('abc', 'devnet'), 'https://solscan.io/account/abc?cluster=devnet')
})
