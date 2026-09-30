//! Shared LiteSVM harness for the per-instruction test files.
//!
//! Loads the basket program plus the Metaplex token-metadata and Meteora
//! DLMM (`lb_clmm`) binaries dumped into `tests/fixtures/` (see
//! `scripts/fetch-fixtures.ps1`), and provides small helpers for keypairs,
//! SPL mints/ATAs, Anchor instruction building and the ed25519 precompile.

#![allow(dead_code)]

pub mod ed25519_stub;
pub mod books;
pub mod dlmm;
pub mod fees;
pub mod keeper;
pub mod lifecycle;
pub mod managed;
pub mod redeem;
pub mod sleeve;
pub mod treasury;

pub use books::*;
pub use dlmm::*;
pub use fees::*;
pub use keeper::*;
pub use lifecycle::*;
pub use managed::*;
pub use redeem::*;
pub use sleeve::*;
pub use treasury::*;

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
/// Positions the `create_basket` transaction itself can carry: the DLMM
/// pool init (pair, two reserves, oracle), the metadata CPI and the
/// launch-proof token moves use ~25 of the 64 traced instructions.
pub const CREATE_BASKET_POSITIONS: usize = 6;
pub const METAPLEX_ID: Pubkey = anchor_spl::metadata::ID;
pub const WSOL: Pubkey = anchor_spl::token::spl_token::native_mint::ID;
pub const NATIVE_MINT: Pubkey = WSOL;
/// LiteSVM starts at unix time 0; DLMM treats `now == 0` as "pool locked".
/// Every cluster is far past this, so the harness starts here.
pub const GENESIS_TS: i64 = 1_790_000_000;

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
        svm.add_program(dlmm::LB_CLMM_ID, &fixture("lb_clmm.so")).unwrap();
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
        // The treasury and team wallets must stay rent-exempt to receive lamports.
        svm.airdrop(&treasury.pubkey(), LAMPORTS).unwrap();
        let mut env = Env { svm, admin, treasury, keeper };
        env.warp(GENESIS_TS);
        env
    }

    /// Fresh SVM with `initialize_config` (cluster 2 = localnet) and the
    /// BUYBACK / PRIZE vaults done.
    pub fn initialized() -> Self {
        let mut env = Env::new();
        env.initialize_config(ConfigUpdate::default()).unwrap();
        env.init_treasury_vaults();
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

    pub fn slot(&self) -> u64 {
        self.svm.get_sysvar::<Clock>().slot
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

    /// Lamports above the account's own rent exemption (0 if it does not exist).
    pub fn free_lamports(&self, key: &Pubkey) -> u64 {
        match self.account(key) {
            Some(acc) => acc.lamports.saturating_sub(self.svm.minimum_balance_for_rent_exemption(acc.data.len())),
            None => 0,
        }
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

    /// Wrap `lamports` of `owner`'s SOL into their wSOL ATA.
    pub fn wrap_sol(&mut self, owner: &Keypair, lamports: u64) -> Pubkey {
        let ata = Env::ata(&owner.pubkey(), &WSOL);
        let ixs = [
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &owner.pubkey(),
                &owner.pubkey(),
                &WSOL,
                &spl_token::ID,
            ),
            anchor_lang::solana_program::system_instruction::transfer(&owner.pubkey(), &ata, lamports),
            spl_token::instruction::sync_native(&spl_token::ID, &ata).unwrap(),
        ];
        self.send_ok(&ixs, owner, &[]);
        ata
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

    pub fn config(&self) -> Config {
        self.load(&config_pda())
    }

    pub fn update_config(&mut self, update: ConfigUpdate) {
        let ix = self.admin_ix(basket::instruction::UpdateConfig { update }.data());
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
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

    pub fn set_paused(&mut self, paused: bool) {
        let ix = self.admin_ix(basket::instruction::SetPaused { paused }.data());
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
    }
}

// ---- PDAs ----

pub fn config_pda() -> Pubkey {
    Pubkey::find_program_address(&[seeds::CONFIG], &basket::id()).0
}

pub fn whitelist_pda() -> Pubkey {
    Pubkey::find_program_address(&[seeds::WHITELIST], &basket::id()).0
}

/// The share mint is a PDA of the deployer (payer) and the nonce.
pub fn share_mint_pda(payer: &Pubkey, nonce: u64) -> Pubkey {
    Pubkey::find_program_address(&[seeds::SHARE_MINT, payer.as_ref(), &nonce.to_le_bytes()], &basket::id()).0
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

pub fn buyback_vault_pda() -> Pubkey {
    Pubkey::find_program_address(&[seeds::BUYBACK], &basket::id()).0
}

pub fn prize_vault_pda() -> Pubkey {
    Pubkey::find_program_address(&[seeds::PRIZE], &basket::id()).0
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

/// Launch bin every harness basket opens at: 1 lamport per base share unit
/// (0.001 SOL per whole share).
pub const LAUNCH_ACTIVE_ID: i32 = 0;

/// A default Fixed/index payload over `book`: 48 h mint window from `now`.
pub fn fixed_args(nonce: u64, book: &[PositionArg], now: i64) -> CreateBasketArgs {
    CreateBasketArgs {
        nonce,
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
        gate: MintGate::WindowUntil { close_ts: now + DEFAULT_MINT_WINDOW_S },
        schedule: None,
        launch_active_id: LAUNCH_ACTIVE_ID,
    }
}

pub fn serialize<T: AnchorSerialize>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.serialize(&mut out).unwrap();
    out
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

/// The DLMM pool `create_basket` opens for a share mint.
pub fn pool_for(share_mint: &Pubkey, bin_step: u16) -> DlmmPool {
    let lb_pair = basket::dlmm::customizable_lb_pair(share_mint, &WSOL);
    DlmmPool {
        lb_pair,
        mint_x: *share_mint,
        mint_y: WSOL,
        reserve_x: basket::dlmm::reserve(&lb_pair, share_mint),
        reserve_y: basket::dlmm::reserve(&lb_pair, &WSOL),
        oracle: basket::dlmm::oracle(&lb_pair),
        bin_step,
    }
}

pub fn create_basket_ix(
    payer: &Pubkey,
    args: &CreateBasketArgs,
    chunk: &[PositionArg],
    attestation: Option<TierAttestation>,
) -> Instruction {
    let share_mint = share_mint_pda(payer, args.nonce);
    let basket_key = basket_pda(&share_mint);
    let pool = pool_for(&share_mint, 0);
    let mut accounts = basket::accounts::CreateBasket {
        payer: *payer,
        config: config_pda(),
        whitelist: whitelist_pda(),
        share_mint,
        share_auth: share_auth_pda(&share_mint),
        basket: basket_key,
        metadata: metadata_pda(&share_mint),
        lb_pair: pool.lb_pair,
        reserve_x: pool.reserve_x,
        reserve_y: pool.reserve_y,
        oracle: pool.oracle,
        payer_share_ata: Env::ata(payer, &share_mint),
        payer_wsol_ata: Env::ata(payer, &WSOL),
        wsol_mint: WSOL,
        dlmm_event_authority: basket::dlmm::event_authority(),
        dlmm_program: LB_CLMM_ID,
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
        data: basket::instruction::CreateBasket { args: args.clone(), positions: chunk.to_vec(), attestation }.data(),
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
/// all positions created in one `create_basket`. The deployer is both
/// `creator` and `payer` (the same keypair, cloned).
pub struct Launched {
    pub creator: Keypair,
    pub payer: Keypair,
    pub mints: Vec<Pubkey>,
    pub book: Vec<PositionArg>,
    pub args: CreateBasketArgs,
    pub share_mint: Pubkey,
    pub basket: Pubkey,
    pub pool: DlmmPool,
}

impl Launched {
    pub fn tight_position(&self, env: &Env) -> Pubkey {
        let b: Basket = env.load(&self.basket);
        b.tight.key
    }

    /// The account to pass as `backstop_position` when the basket has none:
    /// an inert, unowned PDA (the share authority).
    pub fn placeholder(&self) -> Pubkey {
        share_auth_pda(&self.share_mint)
    }

    pub fn basket_state(&self, env: &Env) -> Basket {
        env.load(&self.basket)
    }
}

/// Keeper-signed tier attestation: the precompile instruction to put right
/// before `create_basket`, and the argument to pass with it.
pub fn tier_attestation_for(
    keeper: &Keypair,
    cluster: u8,
    creator: &Pubkey,
    tier: u8,
    expiry_slot: u64,
) -> (Instruction, TierAttestation) {
    let message = serialize(&TierMessage { program_id: basket::id(), cluster, creator: *creator, tier, expiry_slot });
    let sig = keeper.sign_message(&message);
    let sig_bytes: [u8; 64] = sig.as_ref().try_into().unwrap();
    let ix = basket::ed25519::ed25519_instruction_embedded(&keeper.pubkey().to_bytes(), &sig_bytes, &message);
    (ix, TierAttestation { keeper: keeper.pubkey(), tier, expiry_slot })
}

impl Env {
    pub fn tier_attestation(&self, creator: &Pubkey, tier: u8) -> (Instruction, TierAttestation) {
        tier_attestation_for(&self.keeper, 2, creator, tier, self.slot() + 1_000)
    }

    pub fn launch_fixed(&mut self, n: usize, nonce: u64) -> Launched {
        self.launch_with(n, nonce, |_| {})
    }

    /// Like `launch_fixed`, with a hook to edit the payload before sending.
    pub fn launch_with(&mut self, n: usize, nonce: u64, edit: impl FnOnce(&mut CreateBasketArgs)) -> Launched {
        self.launch_partial_with(n, nonce, n, false, None, edit)
    }

    /// Creates the basket with only the first `first` positions delivered.
    pub fn launch_partial(&mut self, n: usize, nonce: u64, first: usize) -> Launched {
        self.launch_partial_with(n, nonce, n.min(first), false, None, |_| {})
    }

    /// Component mints carry a freeze authority (the admin), xStocks-style.
    pub fn launch_fixed_freezable(&mut self, n: usize, nonce: u64) -> Launched {
        self.launch_partial_with(n, nonce, n, true, None, |_| {})
    }

    /// Launch with a keeper-attested creator tier.
    pub fn launch_with_tier(&mut self, n: usize, nonce: u64, tier: u8) -> Launched {
        self.launch_partial_with(n, nonce, n, false, Some(tier), |_| {})
    }

    /// Everything a deployer needs before `create_basket`: SOL and a wSOL ATA.
    pub fn new_deployer(&mut self) -> Keypair {
        let payer = self.fund(50 * LAMPORTS);
        self.create_ata(&payer.pubkey(), &WSOL);
        payer
    }

    fn launch_partial_with(
        &mut self,
        n: usize,
        nonce: u64,
        first: usize,
        freezable: bool,
        tier: Option<u8>,
        edit: impl FnOnce(&mut CreateBasketArgs),
    ) -> Launched {
        let payer = self.new_deployer();
        let admin_key = self.admin.pubkey();
        let freeze = if freezable { Some(admin_key) } else { None };
        let mints: Vec<Pubkey> =
            (0..n).map(|_| self.create_mint_with_freeze(6, &admin_key, freeze.as_ref())).collect();
        self.whitelist(&mints);
        let book = book(&mints, &equal_weights(n));
        let mut args = fixed_args(nonce, &book, self.now());
        edit(&mut args);
        let share_mint = share_mint_pda(&payer.pubkey(), nonce);
        // Each position costs ~5 inner instructions (account + ATA), and a
        // transaction may trace at most 64: the launch tx carries fewer
        // positions than an `add_positions` chunk (§6).
        let head = first.min(CREATE_BASKET_POSITIONS);
        let mut ixs = Vec::new();
        let attestation = tier.map(|t| {
            let (ix, att) = self.tier_attestation(&payer.pubkey(), t);
            ixs.push(ix);
            att
        });
        ixs.push(create_basket_ix(&payer.pubkey(), &args, &book[..head], attestation));
        let m = self.send_ok(&ixs, &payer, &[]);
        eprintln!("create_basket({head} positions) CU {}", m.compute_units_consumed);
        for chunk in book[head..first].chunks(POSITIONS_PER_TX) {
            self.send_ok(&[add_positions_ix(&payer.pubkey(), &share_mint, chunk)], &payer, &[]);
        }
        let b: Basket = self.load(&basket_pda(&share_mint));
        Launched {
            creator: payer.insecure_clone(),
            payer,
            mints,
            book,
            args,
            share_mint,
            basket: basket_pda(&share_mint),
            pool: pool_for(&share_mint, b.pool.preset.bin_step),
        }
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
