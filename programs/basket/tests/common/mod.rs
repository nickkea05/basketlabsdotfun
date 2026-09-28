//! Shared LiteSVM harness for the per-instruction test files.
//!
//! Loads the basket program plus the Metaplex token-metadata and Meteora
//! DAMM v2 (`cp-amm`) binaries dumped into `tests/fixtures/` (see
//! `scripts/fetch-fixtures.ps1`), and provides small helpers for keypairs,
//! SPL mints/ATAs, Anchor instruction building and the ed25519 precompile.

#![allow(dead_code)]

pub mod ed25519_stub;
pub mod fees;
pub mod managed;
pub mod redeem;
pub mod sleeve;

pub use managed::*;
pub use redeem::*;
pub use sleeve::*;

use anchor_lang::prelude::*;
use ed25519_stub::BuiltinFunctionDefinition as _;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::associated_token::{
    get_associated_token_address_with_program_id, spl_associated_token_account,
};
use anchor_spl::token::spl_token;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use litesvm::LiteSVM;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program_pack::Pack;
use solana_signer::Signer;
use solana_transaction::Transaction;

use basket::constants::*;
use basket::state::*;

pub const LAMPORTS: u64 = 1_000_000_000;
/// Positions per `create_basket` / `add_positions` transaction (instruction
/// trace limit of 64; each position is ~5 inner instructions).
pub const POSITIONS_PER_TX: usize = 8;
pub const METAPLEX_ID: Pubkey = anchor_spl::metadata::ID;
pub const CP_AMM_ID: Pubkey = basket::damm::CP_AMM_ID;
pub const WSOL: Pubkey = anchor_spl::token::spl_token::native_mint::ID;
pub const NATIVE_MINT: Pubkey = WSOL;

pub fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!("missing fixture {path}: {e}. Run scripts/fetch-fixtures.ps1")
    })
}

pub fn program_bytes() -> Vec<u8> {
    let path = format!("{}/../../target/deploy/basket.so", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("run `anchor build` first ({path}): {e}"))
}

pub struct Env {
    pub svm: LiteSVM,
    pub admin: Keypair,
    pub treasury: Keypair,
    pub keeper: Keypair,
}

impl Env {
    /// Fresh SVM with all programs loaded; config not initialized.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        svm.add_builtin(ed25519_stub::ED25519_ID, ed25519_stub::Ed25519Stub::register);
        svm.add_program(basket::id(), &program_bytes()).unwrap();
        svm.add_program(METAPLEX_ID, &fixture("mpl_token_metadata.so")).unwrap();
        svm.add_program(CP_AMM_ID, &fixture("cp_amm.so")).unwrap();
        // The wSOL native mint exists on every cluster; LiteSVM starts empty.
        let mut wsol_data = vec![0u8; spl_token::state::Mint::LEN];
        spl_token::state::Mint {
            mint_authority: None.into(),
            supply: 0,
            decimals: 9,
            is_initialized: true,
            freeze_authority: None.into(),
        }
        .pack_into_slice(&mut wsol_data);
        svm.set_account(
            WSOL,
            solana_account::Account {
                lamports: svm.minimum_balance_for_rent_exemption(wsol_data.len()),
                data: wsol_data,
                owner: spl_token::ID,
                executable: false,
                rent_epoch: 0,
            },
        )
        .unwrap();
        let admin = Keypair::new();
        let treasury = Keypair::new();
        let keeper = Keypair::new();
        svm.airdrop(&admin.pubkey(), 100 * LAMPORTS).unwrap();
        svm.airdrop(&keeper.pubkey(), 10 * LAMPORTS).unwrap();
        Env { svm, admin, treasury, keeper }
    }

    /// Fresh SVM with `initialize_config` done (cluster 2 = localnet).
    pub fn initialized() -> Self {
        let mut env = Env::new();
        env.initialize_config(ConfigUpdate::default()).unwrap();
        env
    }

    pub fn fund(&mut self, lamports: u64) -> Keypair {
        let kp = Keypair::new();
        self.svm.airdrop(&kp.pubkey(), lamports).unwrap();
        kp
    }

    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    pub fn warp(&mut self, seconds: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp += seconds;
        clock.slot += (seconds.max(1) as u64) * 5 / 2;
        self.svm.set_sysvar(&clock);
    }

    pub fn send(
        &mut self,
        ixs: &[Instruction],
        payer: &Keypair,
        signers: &[&Keypair],
    ) -> std::result::Result<TransactionMetadata, FailedTransactionMetadata> {
        let mut all: Vec<&Keypair> = vec![payer];
        for s in signers {
            if s.pubkey() != payer.pubkey() {
                all.push(s);
            }
        }
        // Every real client attaches a compute budget; batch instructions
        // (20-asset books, N-component mint/redeem) need more than the 200k
        // default. Appended last so ed25519 instruction indices stay stable.
        let mut ixs = ixs.to_vec();
        ixs.push(set_compute_unit_limit(1_400_000));
        let msg = Message::new_with_blockhash(&ixs, Some(&payer.pubkey()), &self.svm.latest_blockhash());
        let tx = Transaction::new(&all, msg, self.svm.latest_blockhash());
        let r = self.svm.send_transaction(tx);
        // Avoid duplicate-signature rejections for identical back-to-back txs.
        self.svm.expire_blockhash();
        r
    }

    pub fn send_ok(&mut self, ixs: &[Instruction], payer: &Keypair, signers: &[&Keypair]) -> TransactionMetadata {
        // (compute budget appended inside `send`)
        match self.send(ixs, payer, signers) {
            Ok(m) => m,
            Err(e) => panic!("tx failed: {:?}\nlogs:\n{}", e.err, e.meta.logs.join("\n")),
        }
    }

    /// Send and require failure with the given custom program error.
    pub fn send_expect_err(&mut self, ixs: &[Instruction], payer: &Keypair, signers: &[&Keypair], code: u32) {
        match self.send(ixs, payer, signers) {
            Ok(m) => panic!("expected error {code}, tx succeeded:\n{}", m.logs.join("\n")),
            Err(e) => {
                let s = format!("{:?}", e.err);
                assert!(
                    s.contains(&format!("Custom({code})")),
                    "expected Custom({code}), got {s}\nlogs:\n{}",
                    e.meta.logs.join("\n")
                );
            }
        }
    }

    pub fn account(&self, key: &Pubkey) -> Option<solana_account::Account> {
        self.svm.get_account(key)
    }

    pub fn load<T: AccountDeserialize>(&self, key: &Pubkey) -> T {
        let acc = self.account(key).unwrap_or_else(|| panic!("account {key} missing"));
        T::try_deserialize(&mut &acc.data[..]).unwrap()
    }

    pub fn lamports(&self, key: &Pubkey) -> u64 {
        self.svm.get_balance(key).unwrap_or(0)
    }

    // ---- SPL helpers ----

    pub fn create_mint(&mut self, decimals: u8, authority: &Pubkey) -> Pubkey {
        self.create_mint_with_freeze(decimals, authority, None)
    }

    pub fn create_mint_with_freeze(&mut self, decimals: u8, authority: &Pubkey, freeze: Option<&Pubkey>) -> Pubkey {
        let mint = Keypair::new();
        let rent = self.svm.minimum_balance_for_rent_exemption(spl_token::state::Mint::LEN);
        let ixs = [
            anchor_lang::solana_program::system_instruction::create_account(
                &self.admin.pubkey(),
                &mint.pubkey(),
                rent,
                spl_token::state::Mint::LEN as u64,
                &spl_token::ID,
            ),
            spl_token::instruction::initialize_mint2(&spl_token::ID, &mint.pubkey(), authority, freeze, decimals).unwrap(),
        ];
        let admin = self.admin.insecure_clone();
        self.send_ok(&ixs, &admin, &[&mint]);
        mint.pubkey()
    }

    pub fn ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        get_associated_token_address_with_program_id(owner, mint, &spl_token::ID)
    }

    pub fn create_ata(&mut self, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
        let ix = spl_associated_token_account::instruction::create_associated_token_account_idempotent(
            &self.admin.pubkey(),
            owner,
            mint,
            &spl_token::ID,
        );
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
        Self::ata(owner, mint)
    }

    /// Mint `amount` of `mint` (authority = admin) into `owner`'s ATA.
    pub fn mint_to(&mut self, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Pubkey {
        let ata = self.create_ata(owner, mint);
        let ix = spl_token::instruction::mint_to(&spl_token::ID, mint, &ata, &self.admin.pubkey(), &[], amount).unwrap();
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
        ata
    }

    pub fn token_amount(&self, ata: &Pubkey) -> u64 {
        match self.account(ata) {
            Some(acc) if acc.data.len() >= 72 => spl_token::state::Account::unpack(&acc.data[..165]).map(|a| a.amount).unwrap_or(0),
            _ => 0,
        }
    }

    pub fn mint_supply(&self, mint: &Pubkey) -> u64 {
        let acc = self.account(mint).unwrap();
        spl_token::state::Mint::unpack(&acc.data[..82]).unwrap().supply
    }

    // ---- program instructions ----

    pub fn initialize_config(
        &mut self,
        update: ConfigUpdate,
    ) -> std::result::Result<TransactionMetadata, FailedTransactionMetadata> {
        let ix = Instruction {
            program_id: basket::id(),
            accounts: basket::accounts::InitializeConfig {
                admin: self.admin.pubkey(),
                config: config_pda(),
                whitelist: whitelist_pda(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: basket::instruction::InitializeConfig {
                cluster: 2,
                treasury: self.treasury.pubkey(),
                keepers: vec![self.keeper.pubkey()],
                update,
            }
            .data(),
        };
        let admin = self.admin.insecure_clone();
        self.send(&[ix], &admin, &[])
    }

    pub fn whitelist(&mut self, mints: &[Pubkey]) {
        let ix = Instruction {
            program_id: basket::id(),
            accounts: basket::accounts::UpdateWhitelist {
                authority: self.admin.pubkey(),
                config: config_pda(),
                whitelist: whitelist_pda(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: basket::instruction::UpdateWhitelist {
                add: mints.iter().map(|m| WhitelistEntry { mint: *m, tier: 0 }).collect(),
                remove: vec![],
            }
            .data(),
        };
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
    }

    pub fn admin_ix(&self, data: Vec<u8>) -> Instruction {
        Instruction {
            program_id: basket::id(),
            accounts: basket::accounts::AdminOnly { admin: self.admin.pubkey(), config: config_pda() }
                .to_account_metas(None),
            data,
        }
    }
}

// ---- PDAs ----

pub fn config_pda() -> Pubkey {
    Pubkey::find_program_address(&[seeds::CONFIG], &basket::id()).0
}

pub fn whitelist_pda() -> Pubkey {
    Pubkey::find_program_address(&[seeds::WHITELIST], &basket::id()).0
}

pub fn share_mint_pda(creator: &Pubkey, nonce: u64) -> Pubkey {
    Pubkey::find_program_address(&[seeds::SHARE_MINT, creator.as_ref(), &nonce.to_le_bytes()], &basket::id()).0
}

pub fn basket_pda(share_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::BASKET, share_mint.as_ref()], &basket::id()).0
}

pub fn share_auth_pda(share_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::SHARE_AUTH, share_mint.as_ref()], &basket::id()).0
}

pub fn position_pda(share_mint: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::POSITION, share_mint.as_ref(), mint.as_ref()], &basket::id()).0
}

pub fn fee_vault_pda(share_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::FEES, share_mint.as_ref()], &basket::id()).0
}

pub fn metadata_pda(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"metadata", METAPLEX_ID.as_ref(), mint.as_ref()], &METAPLEX_ID).0
}

// ---- book helpers ----

pub fn chain_hash(book: &[PositionArg]) -> [u8; 32] {
    let mut acc = [0u8; 32];
    for p in book {
        acc = Basket::chain_hash(&acc, &p.mint, p.weight_bps);
    }
    acc
}

pub fn book(mints: &[Pubkey], weights: &[u16]) -> Vec<PositionArg> {
    mints.iter().zip(weights).map(|(m, w)| PositionArg { mint: *m, weight_bps: *w }).collect()
}

/// Equal weights summing to exactly 10_000 (remainder to the first).
pub fn equal_weights(n: usize) -> Vec<u16> {
    let each = BPS_TOTAL / n as u16;
    let mut w = vec![each; n];
    w[0] += BPS_TOTAL - each * n as u16;
    w
}

/// A default Fixed/index payload for `creator` over `book`.
pub fn fixed_args(creator: &Pubkey, nonce: u64, book: &[PositionArg]) -> CreateBasketArgs {
    CreateBasketArgs {
        domain: Domain { program_id: basket::id(), cluster: 2, nonce },
        creator: *creator,
        basket_type: BASKET_TYPE_FIXED,
        fixed_kind: FIXED_KIND_INDEX,
        name: "Solana Blue".into(),
        symbol: "BLUE".into(),
        uri: "https://basketlabs.fun/m/blue.json".into(),
        asset_count: book.len() as u16,
        book_hash: chain_hash(book),
        hosts_hash: [0u8; 32],
        host_weighting: 0,
        strategy_hash: [0u8; 32],
        sleeve_r_bps: 700,
        perf_fee_bps: 0,
        gate: MintGate::Open,
        schedule: None,
    }
}

pub fn serialize<T: AnchorSerialize>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.serialize(&mut out).unwrap();
    out
}

/// Ed25519 precompile instruction signing `borsh(args)`, referencing the
/// message from instruction `ix_index` (the `create_basket` instruction) at
/// offset 8.
pub fn creator_signature_ix(creator: &Keypair, args: &CreateBasketArgs, ix_index: u16) -> Instruction {
    let message = serialize(args);
    let sig = creator.sign_message(&message);
    let sig_bytes: [u8; 64] = sig.as_ref().try_into().unwrap();
    basket::ed25519::ed25519_instruction_referencing(
        &creator.pubkey().to_bytes(),
        &sig_bytes,
        ix_index,
        8,
        message.len() as u16,
    )
}

/// `[mint, position, vault]` remaining accounts for a chunk of the book.
pub fn position_metas(basket_key: &Pubkey, share_mint: &Pubkey, chunk: &[PositionArg]) -> Vec<AccountMeta> {
    let mut metas = Vec::new();
    for p in chunk {
        metas.push(AccountMeta::new_readonly(p.mint, false));
        metas.push(AccountMeta::new(position_pda(share_mint, &p.mint), false));
        metas.push(AccountMeta::new(Env::ata(basket_key, &p.mint), false));
    }
    metas
}

pub fn create_basket_ix(
    payer: &Pubkey,
    args: &CreateBasketArgs,
    chunk: &[PositionArg],
) -> Instruction {
    let share_mint = share_mint_pda(&args.creator, args.domain.nonce);
    let basket_key = basket_pda(&share_mint);
    let mut accounts = basket::accounts::CreateBasket {
        payer: *payer,
        config: config_pda(),
        whitelist: whitelist_pda(),
        share_mint,
        share_auth: share_auth_pda(&share_mint),
        basket: basket_key,
        metadata: metadata_pda(&share_mint),
        token_metadata_program: METAPLEX_ID,
        token_program: spl_token::ID,
        token_2022_program: anchor_spl::token_2022::ID,
        associated_token_program: anchor_spl::associated_token::ID,
        system_program: anchor_lang::system_program::ID,
        rent: solana_sdk_ids::sysvar::rent::ID,
        instructions: solana_sdk_ids::sysvar::instructions::ID,
    }
    .to_account_metas(None);
    accounts.extend(position_metas(&basket_key, &share_mint, chunk));
    Instruction {
        program_id: basket::id(),
        accounts,
        data: basket::instruction::CreateBasket { args: args.clone(), positions: chunk.to_vec() }.data(),
    }
}

pub fn add_positions_ix(payer: &Pubkey, share_mint: &Pubkey, chunk: &[PositionArg]) -> Instruction {
    let basket_key = basket_pda(share_mint);
    let mut accounts = basket::accounts::AddPositions {
        payer: *payer,
        config: config_pda(),
        whitelist: whitelist_pda(),
        basket: basket_key,
        token_program: spl_token::ID,
        token_2022_program: anchor_spl::token_2022::ID,
        associated_token_program: anchor_spl::associated_token::ID,
        system_program: anchor_lang::system_program::ID,
    }
    .to_account_metas(None);
    accounts.extend(position_metas(&basket_key, share_mint, chunk));
    Instruction {
        program_id: basket::id(),
        accounts,
        data: basket::instruction::AddPositions { positions: chunk.to_vec() }.data(),
    }
}

/// A launched-but-unseeded basket: `n` whitelisted 6-dp mints, Fixed/index,
/// all positions created in one `create_basket`.
pub struct Launched {
    pub creator: Keypair,
    pub payer: Keypair,
    pub mints: Vec<Pubkey>,
    pub book: Vec<PositionArg>,
    pub args: CreateBasketArgs,
    pub share_mint: Pubkey,
    pub basket: Pubkey,
}

impl Env {
    pub fn launch_fixed(&mut self, n: usize, nonce: u64) -> Launched {
        self.launch_with(n, nonce, |_| {})
    }

    /// Like `launch_fixed`, with a hook to edit the payload before signing.
    pub fn launch_with(&mut self, n: usize, nonce: u64, edit: impl FnOnce(&mut CreateBasketArgs)) -> Launched {
        self.launch_partial_with(n, nonce, n, false, edit)
    }

    /// Creates the basket with only the first `first` positions delivered.
    pub fn launch_partial(&mut self, n: usize, nonce: u64, first: usize) -> Launched {
        self.launch_partial_with(n, nonce, first, false, |_| {})
    }

    /// Component mints carry a freeze authority (the admin), xStocks-style.
    pub fn launch_fixed_freezable(&mut self, n: usize, nonce: u64) -> Launched {
        self.launch_partial_with(n, nonce, n, true, |_| {})
    }

    fn launch_partial_with(
        &mut self,
        n: usize,
        nonce: u64,
        first: usize,
        freezable: bool,
        edit: impl FnOnce(&mut CreateBasketArgs),
    ) -> Launched {
        let creator = Keypair::new();
        let payer = self.fund(50 * LAMPORTS);
        let admin_key = self.admin.pubkey();
        let freeze = if freezable { Some(admin_key) } else { None };
        let mints: Vec<Pubkey> =
            (0..n).map(|_| self.create_mint_with_freeze(6, &admin_key, freeze.as_ref())).collect();
        self.whitelist(&mints);
        let book = book(&mints, &equal_weights(n));
        let mut args = fixed_args(&creator.pubkey(), nonce, &book);
        edit(&mut args);
        let share_mint = share_mint_pda(&creator.pubkey(), nonce);
        // Each position costs ~5 inner instructions (account + ATA), and a
        // transaction may trace at most 64, so books go in chunks of 8 (§6).
        let head = first.min(POSITIONS_PER_TX);
        let ixs = [creator_signature_ix(&creator, &args, 1), create_basket_ix(&payer.pubkey(), &args, &book[..head])];
        self.send_ok(&ixs, &payer, &[]);
        for chunk in book[head..first].chunks(POSITIONS_PER_TX) {
            self.send_ok(&[add_positions_ix(&payer.pubkey(), &share_mint, chunk)], &payer, &[]);
        }
        Launched { creator, payer, mints, book, args, share_mint, basket: basket_pda(&share_mint) }
    }

    pub fn set_paused(&mut self, paused: bool) {
        let ix = self.admin_ix(basket::instruction::SetPaused { paused }.data());
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
    }
}

pub const fn err(e: basket::error::BasketError) -> u32 {
    6000 + e as u32
}

/// `ComputeBudgetInstruction::SetComputeUnitLimit` (discriminant 2).
pub fn set_compute_unit_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction { program_id: solana_sdk_ids::compute_budget::ID, accounts: vec![], data }
}
