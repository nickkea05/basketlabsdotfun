// Basket program PDAs and token accounts. Mirrors the seeds in
// programs/basket/src/constants.rs.

import { PublicKey } from '@solana/web3.js'
import { ASSOCIATED_TOKEN_PROGRAM_ID, BASKET_PROGRAM_ID, METAPLEX_PROGRAM_ID, SEEDS, TOKEN_PROGRAM_ID } from './constants.js'

const enc = (s) => Buffer.from(s, 'utf8')
const pda = (seeds, program = BASKET_PROGRAM_ID) => PublicKey.findProgramAddressSync(seeds, program)[0]

const u64le = (n) => {
  const b = Buffer.alloc(8)
  b.writeBigUInt64LE(BigInt(n))
  return b
}

export const configPda = () => pda([enc(SEEDS.config)])
export const whitelistPda = () => pda([enc(SEEDS.whitelist)])
export const buybackVaultPda = () => pda([enc(SEEDS.buyback)])
export const prizeVaultPda = () => pda([enc(SEEDS.prize)])

/** The share mint is a PDA of the deployer and a per-deployer nonce. */
export const shareMintPda = (payer, nonce) => pda([enc(SEEDS.shareMint), payer.toBuffer(), u64le(nonce)])
export const basketPda = (shareMint) => pda([enc(SEEDS.basket), shareMint.toBuffer()])
export const shareAuthPda = (shareMint) => pda([enc(SEEDS.shareAuth), shareMint.toBuffer()])
export const componentPositionPda = (shareMint, mint) => pda([enc(SEEDS.position), shareMint.toBuffer(), mint.toBuffer()])
export const feeVaultPda = (shareMint) => pda([enc(SEEDS.fees), shareMint.toBuffer()])

export const metadataPda = (mint) => pda([enc('metadata'), METAPLEX_PROGRAM_ID.toBuffer(), mint.toBuffer()], METAPLEX_PROGRAM_ID)

/** Associated token account. Shares and wSOL are classic SPL Token; components may be Token-2022. */
export const ata = (owner, mint, tokenProgram = TOKEN_PROGRAM_ID) =>
  PublicKey.findProgramAddressSync([owner.toBuffer(), tokenProgram.toBuffer(), mint.toBuffer()], ASSOCIATED_TOKEN_PROGRAM_ID)[0]
