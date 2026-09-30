//! basketlabs.fun factory program.
//!
//! One deployment. Each launch creates a `Basket`, a classic SPL share mint
//! with Metaplex metadata, a Meteora DLMM pool (shares / wSOL) the basket
//! holds two positions in, and one `Position` + vault ATA per component.
//! Spec: `docs/program-build-confirmation.md` as amended by
//! `docs/change-order-liquidity-and-fees.md` (the change order wins where
//! they disagree) and the decisions in `docs/program-progress.md` §6.4a.

pub mod constants;
pub mod dlmm;
pub mod ed25519;
pub mod error;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;

use anchor_lang::prelude::*;

pub use constants::*;
pub use instructions::*;
pub use state::*;

// Localnet/dev id. The mainnet program id is assigned at first deploy from a
// keypair kept outside this repository; `anchor keys sync` updates this line.
declare_id!("79SdBiQYuH2xU5pj8iQq3p7PMMTiaQ2PeWQ6vV7e4fnX");

#[program]
pub mod basket {
    use super::*;

    // ---- admin (D17, D18) ----

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        cluster: u8,
        treasury: Pubkey,
        keepers: Vec<Pubkey>,
        update: ConfigUpdate,
    ) -> Result<()> {
        handle_initialize_config(ctx, cluster, treasury, keepers, update)
    }

    pub fn update_config(ctx: Context<AdminOnly>, update: ConfigUpdate) -> Result<()> {
        handle_update_config(ctx, update)
    }

    pub fn rotate_keepers(ctx: Context<AdminOnly>, keepers: Vec<Pubkey>) -> Result<()> {
        handle_rotate_keepers(ctx, keepers)
    }

    pub fn set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
        handle_set_paused(ctx, paused)
    }

    /// Admin, once: the $BSKT mint `execute_buyback` burns into (Q8).
    pub fn set_bskt_mint(ctx: Context<AdminOnly>, mint: Pubkey) -> Result<()> {
        handle_set_bskt_mint(ctx, mint)
    }

    pub fn update_whitelist(
        ctx: Context<UpdateWhitelist>,
        add: Vec<WhitelistEntry>,
        remove: Vec<Pubkey>,
    ) -> Result<()> {
        handle_update_whitelist(ctx, add, remove)
    }

    /// Admin: create the BUYBACK and PRIZE PDAs.
    pub fn init_treasury_vaults(ctx: Context<InitTreasuryVaults>) -> Result<()> {
        handle_init_treasury_vaults(ctx)
    }

    // ---- creation (D8, D20; change order §2.2) ----

    /// The deployer launches: Basket, share mint, metadata, DLMM pool, first
    /// chunk of positions; posts the deposit. `attestation` is the optional
    /// keeper-signed tier (ed25519 instruction right before this one).
    pub fn create_basket<'info>(
        ctx: Context<'info, CreateBasket<'info>>,
        args: CreateBasketArgs,
        positions: Vec<PositionArg>,
        attestation: Option<TierAttestation>,
    ) -> Result<()> {
        handle_create_basket(ctx, args, positions, attestation)
    }

    pub fn add_positions<'info>(
        ctx: Context<'info, AddPositions<'info>>,
        positions: Vec<PositionArg>,
    ) -> Result<()> {
        handle_add_positions(ctx, positions)
    }

    // ---- buys (D4, D5, D6, D8) ----

    pub fn seed<'info>(ctx: Context<'info, Seed<'info>>, args: SeedArgs) -> Result<()> {
        handle_seed(ctx, args)
    }

    pub fn mint<'info>(ctx: Context<'info, MintShares<'info>>, args: MintArgs) -> Result<()> {
        handle_mint(ctx, args)
    }

    /// Redeem `shares` (gross) for pro-rata components and the SOL leg in one tx.
    pub fn redeem<'info>(ctx: Context<'info, Redeem<'info>>, shares: u64) -> Result<()> {
        handle_redeem(ctx, shares)
    }

    /// Large books: burn + pool leg now, components in later `redeem_components` chunks.
    pub fn redeem_begin<'info>(ctx: Context<'info, RedeemBegin<'info>>, shares: u64) -> Result<()> {
        handle_redeem_begin(ctx, shares)
    }

    pub fn redeem_components<'info>(ctx: Context<'info, RedeemComponents<'info>>, count: u16) -> Result<()> {
        handle_redeem_components(ctx, count)
    }

    pub fn claim_frozen(ctx: Context<ClaimFrozen>) -> Result<()> {
        handle_claim_frozen(ctx)
    }

    // ---- keeper: the two positions (change order §2.4) ----

    /// Keeper: open the backstop position around the launch bin (or the
    /// current one after a reset). Remaining accounts: its bin arrays.
    pub fn place_backstop<'info>(ctx: Context<'info, KeeperPool<'info>>) -> Result<()> {
        handle_place_backstop(ctx)
    }

    /// Keeper: deposit the idle sleeve's share for one bin array of the backstop.
    pub fn fund_backstop<'info>(ctx: Context<'info, KeeperPool<'info>>, array_index: i64) -> Result<()> {
        handle_fund_backstop(ctx, array_index)
    }

    /// Keeper: withdraw one bin array of the backstop (reset or wind-down).
    pub fn withdraw_backstop<'info>(ctx: Context<'info, KeeperPool<'info>>, array_index: i64) -> Result<()> {
        handle_withdraw_backstop(ctx, array_index)
    }

    /// Keeper: close the emptied backstop position.
    pub fn close_backstop<'info>(ctx: Context<'info, KeeperPool<'info>>) -> Result<()> {
        handle_close_backstop(ctx)
    }

    /// Keeper: move the tight position around the active bin. Remaining
    /// accounts: old range's bin arrays, then the new range's.
    pub fn recenter_tight<'info>(ctx: Context<'info, KeeperPool<'info>>) -> Result<()> {
        handle_recenter_tight(ctx)
    }

    // ---- fees (change order §2.5) ----

    /// Claim one position's swap fees over `[min, max]` and route them. Anyone.
    pub fn claim_pool_fees<'info>(ctx: Context<'info, ClaimPoolFees<'info>>, min_bin_id: i32, max_bin_id: i32) -> Result<()> {
        handle_claim_pool_fees(ctx, min_bin_id, max_bin_id)
    }

    /// Creator's line in shares; the rest burned and redeemed in kind
    /// (tight SOL leg routed now, components via `sweep_fees_components`). Anyone.
    pub fn sweep_fees<'info>(ctx: Context<'info, SweepFees<'info>>) -> Result<()> {
        handle_sweep_fees(ctx)
    }

    pub fn sweep_fees_components<'info>(ctx: Context<'info, SweepFeesComponents<'info>>, count: u16) -> Result<()> {
        handle_sweep_fees_components(ctx, count)
    }

    /// Keeper: swap one FeeVault component ATA to SOL through an allow-listed
    /// venue and route it. Remaining accounts: the inner instruction's.
    pub fn settle_fees<'info>(
        ctx: Context<'info, SettleFees<'info>>,
        amount_in: u64,
        min_amount_out: u64,
        data: Vec<u8>,
    ) -> Result<()> {
        handle_settle_fees(ctx, amount_in, min_amount_out, data)
    }

    /// Keeper: move BUYBACK SOL onto its wSOL account for `execute_buyback`.
    pub fn stage_buyback(ctx: Context<StageBuyback>, lamports: u64) -> Result<()> {
        handle_stage_buyback(ctx, lamports)
    }

    /// Keeper: swap staged wSOL for $BSKT and burn it.
    pub fn execute_buyback<'info>(
        ctx: Context<'info, ExecuteBuyback<'info>>,
        amount_in: u64,
        min_amount_out: u64,
        data: Vec<u8>,
    ) -> Result<()> {
        handle_execute_buyback(ctx, amount_in, min_amount_out, data)
    }

    /// Keeper: pay one epoch's prizes. Remaining accounts:
    /// `[creator_wallet, creator_lock, basket]` per recipient in rank order.
    pub fn post_prize_payout<'info>(ctx: Context<'info, PostPrizePayout<'info>>, epoch: u64, total_lamports: u64) -> Result<()> {
        handle_post_prize_payout(ctx, epoch, total_lamports)
    }

    /// Managed only, once per period: management fee accrual plus the
    /// performance fee above the HWM on a keeper-attested NAV.
    pub fn crystallize<'info>(ctx: Context<'info, Crystallize<'info>>, nav_lamports_per_share: u64) -> Result<()> {
        handle_crystallize(ctx, nav_lamports_per_share)
    }

    // ---- books / rebalance (D17, D20) ----

    /// Propose a new book. Mirror/Strategy: keeper, window opens now.
    /// Managed: creator, timelocked until `apply_book`. Remaining accounts:
    /// `[mint, position, vault]` per entering mint.
    pub fn submit_book<'info>(ctx: Context<'info, SubmitBook<'info>>, book: Vec<PositionArg>) -> Result<()> {
        handle_submit_book(ctx, book)
    }

    /// Create the Position + vault for a pending-book mint. Anyone.
    pub fn open_position<'info>(ctx: Context<'info, OpenPosition<'info>>) -> Result<()> {
        handle_open_position(ctx)
    }

    /// Managed: open the window on the pending book once the timelock has
    /// passed and the turnover fits the cap. Anyone.
    pub fn apply_book<'info>(ctx: Context<'info, ApplyBook<'info>>, current_book: Vec<PositionArg>) -> Result<()> {
        handle_apply_book(ctx, current_book)
    }

    /// Keeper: route one swap through an allow-listed venue with the
    /// basket as authority. Remaining accounts: the inner instruction's.
    pub fn execute_swap<'info>(
        ctx: Context<'info, ExecuteSwap<'info>>,
        amount_in: u64,
        min_amount_out: u64,
        data: Vec<u8>,
    ) -> Result<()> {
        handle_execute_swap(ctx, amount_in, min_amount_out, data)
    }

    /// Keeper: remove an emptied position that left the book.
    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        handle_close_position(ctx)
    }

    /// Keeper: walk the target book (chunked); the last chunk flips the book.
    pub fn finalize_rebalance<'info>(
        ctx: Context<'info, FinalizeRebalance<'info>>,
        entries: Vec<PositionArg>,
    ) -> Result<()> {
        handle_finalize_rebalance(ctx, entries)
    }

    // ---- creator lock ----

    pub fn lock_creator_shares(ctx: Context<LockCreatorShares>, amount: u64, duration_s: i64) -> Result<()> {
        handle_lock_creator_shares(ctx, amount, duration_s)
    }

    pub fn unlock_creator_shares(ctx: Context<UnlockCreatorShares>) -> Result<()> {
        handle_unlock_creator_shares(ctx)
    }

    // ---- close crank ----

    /// Keeper: close positions of a closable basket (dust to treasury, rent
    /// to payer). Remaining accounts: `[position, vault, mint, treasury_ata]`
    /// × `count`, then the tight range's bin arrays.
    pub fn close_positions<'info>(ctx: Context<'info, ClosePositions<'info>>, count: u16) -> Result<()> {
        handle_close_positions(ctx, count)
    }

    /// Keeper: drain and close the tight position, burn leftovers, refund
    /// the deposit and rent to the payer, close the Basket.
    pub fn close_basket<'info>(ctx: Context<'info, CloseBasket<'info>>) -> Result<()> {
        handle_close_basket(ctx)
    }

    /// Keeper: reclaim the FeeVault's rent once everything is settled.
    pub fn close_fee_vault(ctx: Context<CloseFeeVault>) -> Result<()> {
        handle_close_fee_vault(ctx)
    }
}
