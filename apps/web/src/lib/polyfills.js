// web3.js and Anchor expect Node's Buffer. Imported first in main.jsx so it
// exists before any Solana module evaluates.
import { Buffer } from 'buffer'

if (!globalThis.Buffer) globalThis.Buffer = Buffer
