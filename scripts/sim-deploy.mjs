// Deploy target/deploy/basket.so onto a running Surfpool fork with the
// surfnet_writeProgram cheatcode. The CLI deploy path (thousands of buffer
// writes) does not confirm against Surfpool, so the bytes go in directly.
//
//   node scripts/sim-deploy.mjs [rpcUrl]

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { homedir } from 'node:os'
import { Keypair } from '@solana/web3.js'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const rpc = process.argv[2] ?? 'http://127.0.0.1:8899'
const CHUNK = 256 * 1024

const keypair = (p) => Keypair.fromSecretKey(Uint8Array.from(JSON.parse(readFileSync(p, 'utf8'))))
const programId = keypair(join(root, 'target/deploy/basket-keypair.json')).publicKey.toBase58()
const authority = keypair(join(homedir(), '.config/solana/id.json')).publicKey.toBase58()
const so = readFileSync(join(root, 'target/deploy/basket.so'))

async function call(method, params) {
  const res = await fetch(rpc, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
  })
  const body = await res.json()
  if (body.error) throw new Error(`${method}: ${JSON.stringify(body.error)}`)
  return body.result
}

for (let off = 0; off < so.length; off += CHUNK) {
  await call('surfnet_writeProgram', [programId, so.subarray(off, off + CHUNK).toString('hex'), off, authority])
}

const info = await call('getAccountInfo', [programId, { encoding: 'base64', dataSlice: { offset: 0, length: 0 } }])
if (!info.value?.executable) throw new Error(`${programId} is not executable after the write`)
console.log(`deployed ${programId} (${so.length} bytes), upgrade authority ${authority}`)
