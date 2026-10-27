// Instruction builders for the basket program. Each builder takes intent
// (payer, book, amounts) and fills in the fixed accounts plus the
// remaining-account layout the handler reads. Mirrors the Rust harness in
// programs/basket/tests/common.

import { AnchorProvider, BN, Program } from '@anchor-lang/core'
import { ComputeBudgetProgram, Keypair, TransactionInstruction } from '@solana/web3.js'
import idl from '../../idl/basket.json' with { type: 'json' }
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  CREATE_BASKET_POSITIONS,
  CU_LIMIT,
  DLMM_PROGRAM_ID,
  MEMO_PROGRAM_ID,
  METAPLEX_PROGRAM_ID,
  POSITIONS_PER_TX,
  SYSTEM_PROGRAM_ID,
  SYSVAR_INSTRUCTIONS_ID,
  SYSVAR_RENT_ID,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  WSOL_MINT,
} from './constants.js'
import * as dlmm from './dlmm.js'
import {
  ata,
  basketPda,
  buybackVaultPda,
  componentPositionPda,
  configPda,
  feeVaultPda,
  metadataPda,
  prizeVaultPda,
  shareAuthPda,
  shareMintPda,
  whitelistPda,
} from './pda.js'

/** @typedef {{ mint: PublicKey, weightBps: number, tokenProgram?: PublicKey }} BookEntry */

const w = (pubkey) => ({ pubkey, isSigner: false, isWritable: true })
const r = (pubkey) => ({ pubkey, isSigner: false, isWritable: false })

const bn = (v) => (v instanceof BN ? v : new BN(BigInt(v).toString()))

const tokenPrograms = {
  tokenProgram: TOKEN_PROGRAM_ID,
  token2022Program: TOKEN_2022_PROGRAM_ID,
  associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
  systemProgram: SYSTEM_PROGRAM_ID,
}

/** Read-only wallet for building instructions without a signer. */
const readOnlyWallet = (publicKey = Keypair.generate().publicKey) => ({
  publicKey,
  signTransaction: async () => {
    throw new Error('read-only wallet')
  },
  signAllTransactions: async () => {
    throw new Error('read-only wallet')
  },
})

/** `create_basket` remaining accounts: `[mint, position, vault]` per entry. */
export function positionMetas(basket, shareMint, chunk) {
  return chunk.flatMap((p) => [r(p.mint), w(componentPositionPda(shareMint, p.mint)), w(ata(basket, p.mint, p.tokenProgram))])
}

/** Seed / mint / redeem remaining accounts: `[position, vault, owner ata, mint]` per entry. */
export function componentMetas(basket, shareMint, owner, chunk) {
  return chunk.flatMap((p) => [
    w(componentPositionPda(shareMint, p.mint)),
    w(ata(basket, p.mint, p.tokenProgram)),
    w(ata(owner, p.mint, p.tokenProgram)),
    r(p.mint),
  ])
}

/** Bin arrays covering `[lower, upper]`, whether or not they exist yet. */
export function binArrayMetas(lbPair, lower, upper) {
  return dlmm.binArrayIndices(lower, upper).map((i) => w(dlmm.binArray(lbPair, i)))
}

export const computeBudgetIxs = (units = CU_LIMIT) => [ComputeBudgetProgram.setComputeUnitLimit({ units })]

/** Associated Token Program `CreateIdempotent`: a no-op when the account exists. */
export function createAtaIdempotentIx(payer, owner, mint, tokenProgram = TOKEN_PROGRAM_ID) {
  return new TransactionInstruction({
    programId: ASSOCIATED_TOKEN_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      w(ata(owner, mint, tokenProgram)),
      r(owner),
      r(mint),
      r(SYSTEM_PROGRAM_ID),
      r(tokenProgram),
    ],
    data: Buffer.from([1]),
  })
}

/** Split a book into the `create_basket` chunk and the `add_positions` chunks. */
export function chunkBook(book) {
  const first = book.slice(0, CREATE_BASKET_POSITIONS)
  const rest = []
  for (let i = CREATE_BASKET_POSITIONS; i < book.length; i += POSITIONS_PER_TX) rest.push(book.slice(i, i + POSITIONS_PER_TX))
  return { first, rest }
}

const toPositionArgs = (chunk) => chunk.map((p) => ({ mint: p.mint, weightBps: p.weightBps }))

export class BasketClient {
  /**
   * @param {import('@solana/web3.js').Connection} connection
   * @param {object} [wallet] Anchor-style wallet; read-only when omitted.
   */
  constructor(connection, wallet = readOnlyWallet()) {
    this.connection = connection
    this.provider = new AnchorProvider(connection, wallet, { commitment: 'confirmed' })
    this.program = new Program(idl, this.provider)
  }

  get programId() {
    return this.program.programId
  }

  // ---- reads ----

  fetchConfig() {
    return this.program.account.config.fetch(configPda())
  }

  fetchWhitelist() {
    return this.program.account.whitelist.fetch(whitelistPda())
  }

  fetchBasket(shareMint) {
    return this.program.account.basket.fetch(basketPda(shareMint))
  }

  async fetchLbPair(lbPair) {
    const info = await this.connection.getAccountInfo(lbPair)
    if (!info) throw new Error(`lb pair ${lbPair.toBase58()} not found`)
    return dlmm.decodeLbPair(info.data)
  }

  // ---- admin ----

  initializeConfigIx({ admin, cluster, treasury, keepers, update = {} }) {
    return this.program.methods
      .initializeConfig(cluster, treasury, keepers, emptyConfigUpdate(update))
      .accountsStrict({ admin, config: configPda(), whitelist: whitelistPda(), systemProgram: SYSTEM_PROGRAM_ID })
      .instruction()
  }

  initTreasuryVaultsIx({ admin }) {
    return this.program.methods
      .initTreasuryVaults()
      .accountsStrict({
        admin,
        config: configPda(),
        buybackVault: buybackVaultPda(),
        prizeVault: prizeVaultPda(),
        systemProgram: SYSTEM_PROGRAM_ID,
      })
      .instruction()
  }

  /** @param {{ authority: PublicKey, add?: { mint: PublicKey, tier: number }[], remove?: PublicKey[] }} p */
  updateWhitelistIx({ authority, add = [], remove = [] }) {
    return this.program.methods
      .updateWhitelist(add, remove)
      .accountsStrict({ authority, config: configPda(), whitelist: whitelistPda(), systemProgram: SYSTEM_PROGRAM_ID })
      .instruction()
  }

  // ---- launch ----

  /**
   * `create_basket` with the first `CREATE_BASKET_POSITIONS` entries of the
   * book. `args` uses camelCase field names (see the IDL `CreateBasketArgs`).
   * The payer's wSOL ATA must exist: precede with
   * `createAtaIdempotentIx(payer, payer, WSOL_MINT)`.
   */
  createBasketIx({ payer, args, chunk, attestation = null }) {
    const shareMint = shareMintPda(payer, args.nonce)
    const basket = basketPda(shareMint)
    const pool = dlmm.poolFor(shareMint, WSOL_MINT)
    return this.program.methods
      .createBasket({ ...args, nonce: bn(args.nonce) }, toPositionArgs(chunk), attestation)
      .accountsStrict({
        payer,
        config: configPda(),
        whitelist: whitelistPda(),
        shareMint,
        shareAuth: shareAuthPda(shareMint),
        basket,
        metadata: metadataPda(shareMint),
        lbPair: pool.lbPair,
        reserveX: pool.reserveX,
        reserveY: pool.reserveY,
        oracle: pool.oracle,
        payerShareAta: ata(payer, shareMint),
        payerWsolAta: ata(payer, WSOL_MINT),
        wsolMint: WSOL_MINT,
        dlmmEventAuthority: dlmm.eventAuthority(),
        dlmmProgram: DLMM_PROGRAM_ID,
        tokenMetadataProgram: METAPLEX_PROGRAM_ID,
        ...tokenPrograms,
        rent: SYSVAR_RENT_ID,
        instructions: SYSVAR_INSTRUCTIONS_ID,
      })
      .remainingAccounts(positionMetas(basket, shareMint, chunk))
      .instruction()
  }

  addPositionsIx({ payer, shareMint, chunk }) {
    const basket = basketPda(shareMint)
    return this.program.methods
      .addPositions(toPositionArgs(chunk))
      .accountsStrict({ payer, config: configPda(), whitelist: whitelistPda(), basket, ...tokenPrograms })
      .remainingAccounts(positionMetas(basket, shareMint, chunk))
      .instruction()
  }

  /**
   * First buy. Opens the tight position `[activeId − halfWidth, activeId + halfWidth]`.
   * `deposits[i]` pairs with `chunk[i]`; amounts are base units.
   */
  seedIx({ payer, shareMint, treasury, chunk, activeId, tightHalfWidth, deposits, initialShares, sleeveLamports }) {
    const basket = basketPda(shareMint)
    const feeVault = feeVaultPda(shareMint)
    const pool = dlmm.poolFor(shareMint, WSOL_MINT)
    const lower = activeId - tightHalfWidth
    const upper = activeId + tightHalfWidth
    return this.program.methods
      .seed({ deposits: deposits.map(bn), initialShares: bn(initialShares), sleeveLamports: bn(sleeveLamports) })
      .accountsStrict({
        payer,
        config: configPda(),
        basket,
        shareMint,
        shareAuth: shareAuthPda(shareMint),
        payerShareAta: ata(payer, shareMint),
        feeVault,
        feeVaultShareAta: ata(feeVault, shareMint),
        treasury,
        wsolMint: WSOL_MINT,
        basketShareAta: ata(basket, shareMint),
        basketWsolAta: ata(basket, WSOL_MINT),
        lbPair: pool.lbPair,
        reserveX: pool.reserveX,
        reserveY: pool.reserveY,
        tightPosition: dlmm.basketPositionPda(pool.lbPair, basket, lower, upper),
        dlmmEventAuthority: dlmm.eventAuthority(),
        dlmmProgram: DLMM_PROGRAM_ID,
        memoProgram: MEMO_PROGRAM_ID,
        ...tokenPrograms,
        rent: SYSVAR_RENT_ID,
      })
      .remainingAccounts([...componentMetas(basket, shareMint, payer, chunk), ...binArrayMetas(pool.lbPair, lower, upper)])
      .instruction()
  }
}

/** `ConfigUpdate` with every field `None` unless set in `fields`. */
export function emptyConfigUpdate(fields = {}) {
  const keys = idl.types.find((t) => t.name === 'ConfigUpdate').type.fields.map((f) => camel(f.name))
  return Object.fromEntries(keys.map((k) => [k, fields[k] ?? null]))
}

const camel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase())
