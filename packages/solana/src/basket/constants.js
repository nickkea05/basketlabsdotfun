// Program ids, PDA seeds and on-chain constants for the basket program. Keep in
// sync with programs/basket/src/constants.rs and dlmm.rs.

import { PublicKey } from '@solana/web3.js'

export const BASKET_PROGRAM_ID = new PublicKey('79SdBiQYuH2xU5pj8iQq3p7PMMTiaQ2PeWQ6vV7e4fnX')
export const DLMM_PROGRAM_ID = new PublicKey('LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo')
export const METAPLEX_PROGRAM_ID = new PublicKey('metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s')
export const TOKEN_PROGRAM_ID = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA')
export const TOKEN_2022_PROGRAM_ID = new PublicKey('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb')
export const ASSOCIATED_TOKEN_PROGRAM_ID = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL')
export const MEMO_PROGRAM_ID = new PublicKey('MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr')
export const SYSTEM_PROGRAM_ID = new PublicKey('11111111111111111111111111111111')
export const SYSVAR_RENT_ID = new PublicKey('SysvarRent111111111111111111111111111111111')
export const SYSVAR_INSTRUCTIONS_ID = new PublicKey('Sysvar1nstructions1111111111111111111111111')
export const WSOL_MINT = new PublicKey('So11111111111111111111111111111111111111112')

/** Base key of every customizable-permissionless DLMM pool (`ILM_BASE`). */
export const DLMM_ILM_BASE = new PublicKey('MFGQxwAmB91SwuYX36okv2Qmdc9aMuHTwWGUrp4AtB1')

export const SEEDS = Object.freeze({
  config: 'config',
  whitelist: 'whitelist',
  shareMint: 'mint',
  basket: 'basket',
  shareAuth: 'share_auth',
  position: 'position',
  fees: 'fees',
  pending: 'pending',
  claim: 'claim',
  lock: 'lock',
  redemption: 'redemption',
  buyback: 'buyback',
  prize: 'prize',
})

export const DLMM_SEEDS = Object.freeze({
  binArray: 'bin_array',
  bitmap: 'bitmap',
  oracle: 'oracle',
  position: 'position',
  eventAuthority: '__event_authority',
})

export const BasketTypeWire = Object.freeze({ Fixed: 0, Mirror: 1, Managed: 2, Strategy: 3 })
export const FixedKind = Object.freeze({ Index: 0 })

/** `Config.cluster` passed to `initialize_config`. */
export const Cluster = Object.freeze({ Mainnet: 0, Devnet: 1, Localnet: 2 })

export const BPS_TOTAL = 10_000
export const SHARE_DECIMALS = 6
export const LAMPORTS_PER_SOL = 1_000_000_000
export const MINT_WINDOW_S = 48 * 60 * 60
export const STEP_R_BPS = 2_500
export const MIN_SEED_LAMPORTS = LAMPORTS_PER_SOL

/** Positions per transaction: `create_basket` carries at most 6, `add_positions` 8. */
export const CREATE_BASKET_POSITIONS = 6
export const POSITIONS_PER_TX = 8

export const DLMM_BINS_PER_ARRAY = 70
export const DLMM_BINS_PER_POSITION = 70

/** Every sleeve instruction runs against this compute ceiling. */
export const CU_LIMIT = 1_400_000
