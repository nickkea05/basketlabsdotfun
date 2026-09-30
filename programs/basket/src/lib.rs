//! basketlabs.fun factory program.
//!
//! One deployment. Each launch creates a `Basket`, a classic SPL share mint
//! with Metaplex metadata, one `Position` + vault ATA per component, and (at
//! the first buy) a Meteora DAMM v2 pool whose position NFT the basket owns.
//! Spec: `docs/program-build-confirmation.md`; that file wins over comments
//! here where they disagree.

pub mod constants;
pub mod damm;
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

    pub fn update_whitelist(
        ctx: Context<UpdateWhitelist>,
        add: Vec<WhitelistEntry>,
        remove: Vec<Pubkey>,
    ) -> Result<()> {
        handle_update_whitelist(ctx, add, remove)
    }

    // ---- creation (D8, D19, D20) ----

    pub fn create_basket<'info>(
        ctx: Context<'info, CreateBasket<'info>>,
        args: CreateBasketArgs,
        positions: Vec<PositionArg>,
    ) -> Result<()> {
        handle_create_basket(ctx, args, positions)
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

    /// Crank the position's pending swap fees (SOL) into the FeeVault. Anyone.
    pub fn claim_pool_fees(ctx: Context<ClaimPoolFees>) -> Result<()> {
        handle_claim_pool_fees(ctx)
    }

    /// Split accumulated fees: holder line reserved, creator and protocol lines out. Anyone.
    pub fn sweep_fees(ctx: Context<SweepFees>) -> Result<()> {
        handle_sweep_fees(ctx)
    }

    /// Managed only, once per period: management fee accrual plus the
    /// performance fee above the HWM on a keeper-attested NAV.
    pub fn crystallize(ctx: Context<Crystallize>, nav_lamports_per_share: u64) -> Result<()> {
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
    pub fn apply_book(ctx: Context<ApplyBook>, current_book: Vec<PositionArg>) -> Result<()> {
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

    // ---- rewards (D15) ----

    /// Keeper: commit an epoch's Merkle root, funded from the holder reserve
    /// (`reward_mint` = share mint or native mint).
    pub fn post_rewards_root(
        ctx: Context<PostRewardsRoot>,
        epoch: u64,
        root: [u8; 32],
        reward_mint: Pubkey,
        total_amount: u64,
        leaf_count: u32,
    ) -> Result<()> {
        handle_post_rewards_root(ctx, epoch, root, reward_mint, total_amount, leaf_count)
    }

    /// Anyone: pay one leaf to its wallet.
    pub fn distribute_rewards(ctx: Context<DistributeRewards>, index: u32, amount: u64, proof: Vec<[u8; 32]>) -> Result<()> {
        handle_distribute_rewards(ctx, index, amount, proof)
    }

    /// Keeper: retire a paid or stale root; the unpaid remainder returns to the reserve.
    pub fn close_rewards_root(ctx: Context<CloseRewardsRoot>) -> Result<()> {
        handle_close_rewards_root(ctx)
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
    /// to payer). Remaining accounts: `[position, vault, mint, treasury_ata]`.
    pub fn close_positions<'info>(ctx: Context<'info, ClosePositions<'info>>) -> Result<()> {
        handle_close_positions(ctx)
    }

    /// Keeper: drain the sleeve, burn leftovers, close FeeVault + Basket.
    pub fn close_basket(ctx: Context<CloseBasket>) -> Result<()> {
        handle_close_basket(ctx)
    }
}
