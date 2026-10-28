import assert from 'node:assert/strict'
import { test } from 'node:test'
import { Keypair, PublicKey } from '@solana/web3.js'
import { base58Decode, base58Encode, isValidPubkey } from '../src/index.js'

test('base58Encode matches PublicKey.toBase58 and round-trips', () => {
  for (let i = 0; i < 50; i++) {
    const key = Keypair.generate().publicKey
    assert.equal(base58Encode(key.toBytes()), key.toBase58())
    assert.deepEqual(base58Decode(key.toBase58()), key.toBytes())
  }
})

test('leading zero bytes map to leading 1s both ways', () => {
  assert.equal(base58Encode(new Uint8Array([0, 0, 1])), '112')
  assert.deepEqual(base58Decode('11'), new Uint8Array([0, 0]))
  assert.equal(base58Encode(new Uint8Array([])), '')
})

test('the all-zero key (System Program) is a valid pubkey', () => {
  assert.equal(isValidPubkey(PublicKey.default.toBase58()), true)
  assert.equal(base58Decode(PublicKey.default.toBase58()).length, 32)
})
