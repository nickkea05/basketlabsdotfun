// Small Solana helpers with no dependencies.

export * from './wallets.js'

const ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
const MAP = Object.fromEntries([...ALPHABET].map((c, i) => [c, i]))

/** Decode base58 to bytes. Returns null on invalid input. */
export function base58Decode(str) {
  if (!str) return null
  let bytes = [0]
  for (const ch of str) {
    const v = MAP[ch]
    if (v === undefined) return null
    let carry = v
    for (let i = 0; i < bytes.length; i++) {
      carry += bytes[i] * 58
      bytes[i] = carry & 0xff
      carry >>= 8
    }
    while (carry) {
      bytes.push(carry & 0xff)
      carry >>= 8
    }
  }
  for (const ch of str) {
    if (ch !== '1') break
    bytes.push(0)
  }
  return Uint8Array.from(bytes.reverse())
}

/** True if `s` is a well-formed 32-byte Solana public key. */
export function isValidPubkey(s) {
  const b = base58Decode(s?.trim())
  return !!b && b.length === 32
}

export const shortKey = (k, n = 4) => (k ? `${k.slice(0, n)}…${k.slice(-n)}` : '')

export const TOKEN_PROGRAM = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'
export const TOKEN_2022_PROGRAM = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'
