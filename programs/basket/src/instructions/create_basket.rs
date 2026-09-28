//! `create_basket` — the first buyer submits the creator-signed payload.
//! Creates `Basket`, the share mint (classic SPL, PDA), Metaplex metadata,
//! and the first chunk of `Position`s + vault ATAs. Rent from the payer
//! (D8, D19, D20).
//!
//! Remaining accounts: `[mint, position, vault]` per entry in `positions`.

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::{self, mpl_token_metadata, Metadata};
use anchor_spl::token::{Mint, Token};
use anchor_spl::token_2022::Token2022;

use crate::constants::*;
use crate::ed25519::verify_creator_signature;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::{append_positions, try_complete, PositionPrograms};
use crate::state::*;

#[derive(Accounts)]
#[instruction(args: CreateBasketArgs)]
pub struct CreateBasket<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(seeds = [seeds::WHITELIST], bump = whitelist.bump)]
    pub whitelist: Account<'info, Whitelist>,

    /// Share mint. PDA of (creator, nonce): the replay guard (D19).
    #[account(
        init,
        payer = payer,
        seeds = [seeds::SHARE_MINT, args.creator.as_ref(), &args.domain.nonce.to_le_bytes()],
        bump,
        mint::decimals = SHARE_DECIMALS,
        mint::authority = share_auth,
        mint::token_program = token_program,
    )]
    pub share_mint: Account<'info, Mint>,
    /// CHECK: PDA, mint authority and portfolio marker (D2).
    #[account(seeds = [seeds::SHARE_AUTH, share_mint.key().as_ref()], bump)]
    pub share_auth: UncheckedAccount<'info>,
    #[account(
        init,
        payer = payer,
        space = 8 + Basket::INIT_SPACE,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump,
    )]
    pub basket: Account<'info, Basket>,
    /// CHECK: Metaplex metadata PDA, created by CPI.
    #[account(
        mut,
        seeds = [b"metadata", Metadata::id().as_ref(), share_mint.key().as_ref()],
        seeds::program = Metadata::id(),
        bump,
    )]
    pub metadata: UncheckedAccount<'info>,

    pub token_metadata_program: Program<'info, Metadata>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
    /// CHECK: instructions sysvar, for ed25519 introspection.
    #[account(address = solana_sdk_ids::sysvar::instructions::ID)]
    pub instructions: UncheckedAccount<'info>,
}

pub fn validate_args(config: &Config, args: &CreateBasketArgs, now: i64) -> Result<u8> {
    require!(
        args.domain.program_id == crate::ID && args.domain.cluster == config.cluster,
        BasketError::BadDomain
    );
    let profile = profile_for(args.basket_type, args.fixed_kind)?;
    if args.basket_type != BASKET_TYPE_FIXED {
        require!(args.fixed_kind == 0, BasketError::InvalidBasketType);
    }
    require!(args.name.len() <= NAME_MAX && !args.name.is_empty(), BasketError::NameTooLong);
    require!(args.symbol.len() <= SYMBOL_MAX && !args.symbol.is_empty(), BasketError::SymbolTooLong);
    require!(args.uri.len() <= URI_MAX, BasketError::UriTooLong);
    require!(args.asset_count >= 1, BasketError::InvalidArgument);
    if args.basket_type == BASKET_TYPE_FIXED {
        require!(args.asset_count <= MAX_ASSETS_FIXED, BasketError::TooManyAssets);
    }

    let zero = [0u8; 32];
    if args.basket_type == BASKET_TYPE_MIRROR {
        require!(args.hosts_hash != zero, BasketError::InvalidArgument);
        require!(
            args.host_weighting == HOST_WEIGHTING_EQUAL || args.host_weighting == HOST_WEIGHTING_VALUE,
            BasketError::InvalidArgument
        );
    } else {
        require!(args.hosts_hash == zero && args.host_weighting == 0, BasketError::InvalidArgument);
    }
    if args.basket_type == BASKET_TYPE_STRATEGY {
        require!(args.strategy_hash != zero, BasketError::InvalidArgument);
    } else {
        require!(args.strategy_hash == zero, BasketError::InvalidArgument);
    }

    let band = &config.sleeve.profiles[profile as usize];
    require!(
        args.sleeve_r_bps >= band.min_r_bps && args.sleeve_r_bps <= band.max_r_bps,
        BasketError::SleeveOutOfBand
    );
    if args.basket_type == BASKET_TYPE_MANAGED {
        require!(args.perf_fee_bps <= config.fees.perf_fee_max_bps, BasketError::FeeOutOfBounds);
    } else {
        require!(args.perf_fee_bps == 0, BasketError::FeeOutOfBounds);
    }

    if let MintGate::WindowUntil { close_ts } = args.gate {
        require!(close_ts > now, BasketError::InvalidGate);
    }
    if let Some(s) = &args.schedule {
        require!(s.validate(), BasketError::InvalidGate);
        require!(args.gate != MintGate::Open, BasketError::InvalidGate);
    }
    Ok(profile)
}

pub fn handle_create_basket<'info>(
    ctx: Context<'info, CreateBasket<'info>>,
    args: CreateBasketArgs,
    positions: Vec<PositionArg>,
) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let profile = validate_args(config, &args, now)?;

    // D19: the creator signed exactly these bytes.
    let mut message = Vec::with_capacity(256);
    args.serialize(&mut message)?;
    verify_creator_signature(&ctx.accounts.instructions, &args.creator, &message)?;

    let share_mint = ctx.accounts.share_mint.key();
    let band = config.sleeve.profiles[profile as usize];
    let basket = &mut ctx.accounts.basket;
    basket.bump = ctx.bumps.basket;
    basket.share_auth_bump = ctx.bumps.share_auth;
    basket.basket_type = args.basket_type;
    basket.profile = profile;
    basket.host_weighting = args.host_weighting;
    basket.complete = false;
    basket.seeded = false;
    basket.creator = args.creator;
    basket.payer = ctx.accounts.payer.key();
    basket.share_mint = share_mint;
    basket.nonce = args.domain.nonce;
    basket.name = args.name.clone();
    basket.symbol = args.symbol.clone();
    basket.uri = args.uri.clone();
    basket.asset_count = args.asset_count;
    basket.position_count = 0;
    basket.book_hash = args.book_hash;
    basket.book_acc = [0u8; 32];
    basket.weight_acc = 0;
    basket.hosts_hash = args.hosts_hash;
    basket.strategy_hash = args.strategy_hash;
    let managed = args.basket_type == BASKET_TYPE_MANAGED;
    basket.fees = FeeSchedule {
        mint_fee_bps: config.fees.mint_fee_bps,
        redeem_fee_bps: config.fees.redeem_fee_bps,
        holder_split_bps: config.fees.holder_split_bps,
        creator_split_bps: config.fees.creator_split_bps,
        protocol_split_bps: config.fees.protocol_split_bps,
        mgmt_fee_bps_per_year: if managed { config.fees.mgmt_fee_bps_per_year } else { 0 },
        perf_fee_bps: if managed { args.perf_fee_bps } else { 0 },
        pool_fee_bps: band.pool_fee_bps,
    };
    basket.managed = config.managed;
    basket.sleeve = SleeveParams {
        step_r_bps: config.sleeve.step_r_bps,
        step_threshold_lamports: config.sleeve.step_threshold_lamports,
        r_bps: args.sleeve_r_bps,
        price_range: band.price_range,
    };
    basket.gate = args.gate;
    basket.schedule = args.schedule;
    basket.pool = Pubkey::default();
    basket.position_nft_mint = Pubkey::default();
    basket.pool_position = Pubkey::default();
    basket.pending_redeem_shares = 0;
    basket.hwm_nav_lamports = 0;
    basket.last_crystallized_ts = now;
    basket.last_mgmt_accrual_ts = now;
    basket.rebalance = Rebalance::default();
    basket.created_at = now;
    basket.last_activity_at = now;

    // Metaplex metadata (D1). Immutable; the fee line lives in the URI JSON.
    let share_auth_seeds: &[&[u8]] = &[seeds::SHARE_AUTH, share_mint.as_ref(), &[ctx.bumps.share_auth]];
    metadata::create_metadata_accounts_v3(
        CpiContext::new_with_signer(
            ctx.accounts.token_metadata_program.key(),
            metadata::CreateMetadataAccountsV3 {
                metadata: ctx.accounts.metadata.to_account_info(),
                mint: ctx.accounts.share_mint.to_account_info(),
                mint_authority: ctx.accounts.share_auth.to_account_info(),
                payer: ctx.accounts.payer.to_account_info(),
                update_authority: ctx.accounts.share_auth.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
            },
            &[share_auth_seeds],
        ),
        mpl_token_metadata::types::DataV2 {
            name: args.name.clone(),
            symbol: args.symbol.clone(),
            uri: args.uri.clone(),
            seller_fee_basis_points: 0,
            creators: None,
            collection: None,
            uses: None,
        },
        false,
        true,
        None,
    )?;

    // First chunk of the book.
    let programs = PositionPrograms {
        payer: &ctx.accounts.payer.to_account_info(),
        token_program: &ctx.accounts.token_program.to_account_info(),
        token_2022_program: &ctx.accounts.token_2022_program.to_account_info(),
        associated_token_program: &ctx.accounts.associated_token_program.to_account_info(),
        system_program: &ctx.accounts.system_program.to_account_info(),
    };
    let added = append_positions(
        &mut ctx.accounts.basket,
        &ctx.accounts.whitelist,
        &programs,
        ctx.remaining_accounts,
        &positions,
    )?;
    let complete = try_complete(&mut ctx.accounts.basket)?;

    let basket = &ctx.accounts.basket;
    emit!(BasketCreated {
        basket: basket.key(),
        share_mint,
        creator: basket.creator,
        payer: basket.payer,
        basket_type: basket.basket_type,
        profile,
        asset_count: basket.asset_count,
        book_hash: basket.book_hash,
        nonce: basket.nonce,
    });
    if added > 0 {
        emit!(PositionsAdded { basket: basket.key(), from_index: 0, count: added, complete });
    }
    Ok(())
}

#[derive(Accounts)]
pub struct AddPositions<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(seeds = [seeds::WHITELIST], bump = whitelist.bump)]
    pub whitelist: Account<'info, Whitelist>,
    #[account(
        mut,
        seeds = [seeds::BASKET, basket.share_mint.as_ref()],
        bump = basket.bump,
        constraint = !basket.complete @ BasketError::ImmutableBasket,
    )]
    pub basket: Account<'info, Basket>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_add_positions<'info>(
    ctx: Context<'info, AddPositions<'info>>,
    positions: Vec<PositionArg>,
) -> Result<()> {
    require!(!ctx.accounts.config.paused, BasketError::Paused);
    require!(!positions.is_empty(), BasketError::InvalidArgument);
    let from_index = ctx.accounts.basket.position_count;
    let programs = PositionPrograms {
        payer: &ctx.accounts.payer.to_account_info(),
        token_program: &ctx.accounts.token_program.to_account_info(),
        token_2022_program: &ctx.accounts.token_2022_program.to_account_info(),
        associated_token_program: &ctx.accounts.associated_token_program.to_account_info(),
        system_program: &ctx.accounts.system_program.to_account_info(),
    };
    let added = append_positions(
        &mut ctx.accounts.basket,
        &ctx.accounts.whitelist,
        &programs,
        ctx.remaining_accounts,
        &positions,
    )?;
    let complete = try_complete(&mut ctx.accounts.basket)?;
    emit!(PositionsAdded { basket: ctx.accounts.basket.key(), from_index, count: added, complete });
    Ok(())
}
