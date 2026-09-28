//! basketlabs.fun factory program.
//!
//! One deployment. Each launch creates a `Basket`, a classic SPL share mint
//! with Metaplex metadata, one `Position` + vault ATA per component, and (at
//! the first buy) a Meteora DAMM v2 pool whose position NFT the basket owns.
//! Spec: `docs/program-build-confirmation.md`; that file wins over comments
//! here where they disagree.

pub mod constants;
pub mod damm;
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
}
