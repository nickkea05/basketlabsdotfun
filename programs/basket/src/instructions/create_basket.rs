//! `create_basket` — the deployer launches (change order §2.2): creates
//! `Basket`, the share mint (classic SPL, PDA), Metaplex metadata, the DLMM
//! pool (`collect_fee_mode = OnlyY`, flat base fee from the type's preset),
//! and the first chunk of `Position`s + vault ATAs. The deployer pays rent
//! and posts `config.deposit_lamports` into the Basket account; the pool's
//! non-refundable rent is taken out of that deposit right away
//! (`deposit_spent_lamports`), everything else comes back at `close_basket`.
//!
//! Creator tier (Q10): an optional keeper-signed `TierAttestation` in an
//! ed25519 precompile instruction right before this one; missing, expired
//! or unsigned by a Config keeper means tier 0.
//!
//! Remaining accounts: `[mint, position, vault]` per entry in `positions`.

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::{self, mpl_token_metadata, Metadata};
use anchor_spl::token::{self, Mint, Token, TokenAccount};
use anchor_spl::token_2022::Token2022;

use crate::constants::*;
use crate::dlmm::{self, lb_clmm};
use crate::ed25519::verify_ed25519_signature;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::{append_positions, try_complete, PositionPrograms};
use crate::instructions::sleeve::{ensure_ata, spend_deposit, SPEND_POOL};
use crate::state::*;

#[derive(Accounts)]
#[instruction(args: CreateBasketArgs)]
pub struct CreateBasket<'info> {
    /// The deployer: creator, payer of rent, and funder of the deposit.
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(seeds = [seeds::WHITELIST], bump = whitelist.bump)]
    pub whitelist: Account<'info, Whitelist>,

    /// Share mint. PDA of (payer, nonce).
    #[account(
        init,
        payer = payer,
        seeds = [seeds::SHARE_MINT, payer.key().as_ref(), &args.nonce.to_le_bytes()],
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
    pub basket: Box<Account<'info, Basket>>,
    /// CHECK: Metaplex metadata PDA, created by CPI.
    #[account(
        mut,
        seeds = [b"metadata", Metadata::id().as_ref(), share_mint.key().as_ref()],
        seeds::program = Metadata::id(),
        bump,
    )]
    pub metadata: UncheckedAccount<'info>,

    // --- DLMM pool, created by CPI (all address-checked in the handler) ---
    /// CHECK: customizable permissionless LbPair PDA.
    #[account(mut)]
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: reserve for the share mint.
    #[account(mut)]
    pub reserve_x: UncheckedAccount<'info>,
    /// CHECK: reserve for wSOL.
    #[account(mut)]
    pub reserve_y: UncheckedAccount<'info>,
    /// CHECK: oracle PDA.
    #[account(mut)]
    pub oracle: UncheckedAccount<'info>,
    /// CHECK: the deployer's share ATA (created here; DLMM wants it to exist).
    #[account(mut)]
    pub payer_share_ata: UncheckedAccount<'info>,
    /// The deployer's wSOL ATA (must exist; DLMM reads it and wants a
    /// non-zero balance, topped up here if empty).
    #[account(
        mut,
        token::mint = wsol_mint,
        token::authority = payer,
        token::token_program = token_program,
    )]
    pub payer_wsol_ata: Account<'info, TokenAccount>,
    #[account(address = token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: lb_clmm event authority PDA.
    pub dlmm_event_authority: UncheckedAccount<'info>,
    /// CHECK: the DLMM program.
    #[account(address = dlmm::LB_CLMM_ID)]
    pub dlmm_program: UncheckedAccount<'info>,

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

    // Q12 / change order §2.3: the window is bounded; Open only if allowed.
    match args.gate {
        MintGate::WindowUntil { close_ts } => {
            let open_for = close_ts.saturating_sub(now);
            require!(
                open_for >= config.mint_window.min_s && open_for <= config.mint_window.max_s,
                BasketError::InvalidGate
            );
        }
        MintGate::Open => require!(config.allow_open_gate, BasketError::InvalidGate),
        MintGate::Closed => {}
    }
    if let Some(s) = &args.schedule {
        require!(s.validate(), BasketError::InvalidGate);
        require!(args.gate != MintGate::Open, BasketError::InvalidGate);
    }
    Ok(profile)
}

/// Tier from an optional attestation: 0 unless a Config keeper signed
/// `(program, cluster, creator, tier, expiry_slot)` and it has not expired.
fn resolve_tier(
    config: &Config,
    instructions: &AccountInfo,
    creator: &Pubkey,
    attestation: Option<TierAttestation>,
) -> Result<u8> {
    let Some(att) = attestation else { return Ok(0) };
    require!(config.is_keeper(&att.keeper), BasketError::Unauthorized);
    require!((att.tier as usize) < TIER_COUNT, BasketError::InvalidArgument);
    if Clock::get()?.slot > att.expiry_slot {
        return Ok(0);
    }
    let message = TierMessage {
        program_id: crate::ID,
        cluster: config.cluster,
        creator: *creator,
        tier: att.tier,
        expiry_slot: att.expiry_slot,
    };
    let mut bytes = Vec::with_capacity(80);
    message.serialize(&mut bytes)?;
    verify_ed25519_signature(instructions, &att.keeper, &bytes)?;
    Ok(att.tier)
}

pub fn handle_create_basket<'info>(
    ctx: Context<'info, CreateBasket<'info>>,
    args: CreateBasketArgs,
    positions: Vec<PositionArg>,
    attestation: Option<TierAttestation>,
) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let profile = validate_args(config, &args, now)?;
    let creator = ctx.accounts.payer.key();
    let tier = resolve_tier(config, &ctx.accounts.instructions, &creator, attestation)?;

    let share_mint = ctx.accounts.share_mint.key();
    let preset = config.pools.presets[profile as usize];
    let wsol = token::spl_token::native_mint::ID;

    // Pool addresses.
    let lb_pair = dlmm::customizable_lb_pair(&share_mint, &wsol);
    require_keys_eq!(ctx.accounts.lb_pair.key(), lb_pair, BasketError::PoolMismatch);
    require_keys_eq!(ctx.accounts.reserve_x.key(), dlmm::reserve(&lb_pair, &share_mint), BasketError::PoolMismatch);
    require_keys_eq!(ctx.accounts.reserve_y.key(), dlmm::reserve(&lb_pair, &wsol), BasketError::PoolMismatch);
    require_keys_eq!(ctx.accounts.oracle.key(), dlmm::oracle(&lb_pair), BasketError::PoolMismatch);
    require_keys_eq!(ctx.accounts.dlmm_event_authority.key(), dlmm::event_authority(), BasketError::PoolMismatch);
    require!(ctx.accounts.lb_pair.data_is_empty(), BasketError::PoolMismatch);

    let basket = &mut ctx.accounts.basket;
    basket.bump = ctx.bumps.basket;
    basket.share_auth_bump = ctx.bumps.share_auth;
    basket.basket_type = args.basket_type;
    basket.profile = profile;
    basket.host_weighting = args.host_weighting;
    basket.complete = false;
    basket.seeded = false;
    basket.creator_tier = tier;
    basket.creator = creator;
    basket.payer = creator;
    basket.share_mint = share_mint;
    basket.nonce = args.nonce;
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
    // Creator line by tier; the other three lines share the remainder in
    // their configured proportions (FeeSchedule::split).
    basket.fees = FeeSchedule {
        mint_fee_bps: config.fees.mint_fee_bps,
        redeem_fee_bps: config.fees.redeem_fee_bps,
        creator_split_bps: config.creator_bps_for_tier(tier),
        buyback_split_bps: config.fees.buyback_split_bps,
        team_split_bps: config.fees.team_split_bps,
        prize_split_bps: config.fees.prize_split_bps,
        mgmt_fee_bps_per_year: if managed { config.fees.mgmt_fee_bps_per_year } else { 0 },
        perf_fee_bps: if managed { args.perf_fee_bps } else { 0 },
    };
    basket.managed = config.managed;
    basket.sleeve = SleeveParams {
        step_r_bps: config.sleeve.step_r_bps,
        step_threshold_lamports: config.sleeve.step_threshold_lamports,
        r_bps: args.sleeve_r_bps,
    };
    basket.gate = args.gate;
    basket.schedule = args.schedule;
    basket.pool = PoolParams { lb_pair, preset, launch_active_id: args.launch_active_id };
    basket.tight = PositionRef::default();
    basket.backstop = PositionRef::default();
    basket.backstop_state = BACKSTOP_UNPLACED;
    basket.backstop_mask = 0;
    basket.last_recenter_ts = 0;
    basket.recenter_count = 0;
    basket.deposit_lamports = config.deposit_lamports;
    basket.deposit_spent_lamports = 0;
    basket.fee_epoch = 0;
    basket.fee_epoch_lamports = 0;
    basket.fee_prev_epoch_lamports = 0;
    basket.pending_redeem_shares = 0;
    basket.hwm_nav_lamports = 0;
    basket.last_crystallized_ts = now;
    basket.last_mgmt_accrual_ts = now;
    basket.rebalance = Rebalance::default();
    basket.created_at = now;
    basket.last_activity_at = now;

    // The deposit: 1 SOL (Config) held on the Basket account (D11: refunded
    // at close, minus non-refundable rent spent).
    system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.key(),
            system_program::Transfer { from: ctx.accounts.payer.to_account_info(), to: ctx.accounts.basket.to_account_info() },
        ),
        config.deposit_lamports,
    )?;

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

    // The pool. DLMM needs the funder's token accounts for both mints to
    // exist; the share ATA cannot exist before this instruction, so create it.
    ensure_ata(
        &ctx.accounts.payer.to_account_info(),
        &ctx.accounts.payer_share_ata.to_account_info(),
        &ctx.accounts.payer.to_account_info(),
        &ctx.accounts.share_mint.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;
    let (base_factor, base_fee_power_factor) = dlmm::base_factor_for_fee_bps(preset.bin_step, preset.base_fee_bps as u64)
        .ok_or_else(|| error!(BasketError::InvalidArgument))?;
    // DLMM's customizable pair wants the funder to hold some of both tokens
    // as proof they launched it. Mint one base unit of shares to the deployer
    // for the duration of the CPI and burn it right after (supply goes back
    // to 0); make sure the deployer's wSOL ATA holds at least one lamport.
    if ctx.accounts.payer_wsol_ata.amount == 0 {
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.key(),
                system_program::Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.payer_wsol_ata.to_account_info(),
                },
            ),
            1,
        )?;
        token::sync_native(CpiContext::new(
            ctx.accounts.token_program.key(),
            token::SyncNative { account: ctx.accounts.payer_wsol_ata.to_account_info() },
        ))?;
    }
    token::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.key(),
            token::MintTo {
                mint: ctx.accounts.share_mint.to_account_info(),
                to: ctx.accounts.payer_share_ata.to_account_info(),
                authority: ctx.accounts.share_auth.to_account_info(),
            },
            &[share_auth_seeds],
        ),
        1,
    )?;
    // X = shares (asks), Y = wSOL (bids); the pool PDA sorts the mints, the
    // instruction takes X/Y as given (verified in the §6.1b spike).
    let payer_before = ctx.accounts.payer.lamports();
    lb_clmm::cpi::initialize_customizable_permissionless_lb_pair2(
        CpiContext::new(
            ctx.accounts.dlmm_program.key(),
            lb_clmm::cpi::accounts::InitializeCustomizablePermissionlessLbPair2 {
                lb_pair: ctx.accounts.lb_pair.to_account_info(),
                bin_array_bitmap_extension: None,
                token_mint_x: ctx.accounts.share_mint.to_account_info(),
                token_mint_y: ctx.accounts.wsol_mint.to_account_info(),
                reserve_x: ctx.accounts.reserve_x.to_account_info(),
                reserve_y: ctx.accounts.reserve_y.to_account_info(),
                oracle: ctx.accounts.oracle.to_account_info(),
                user_token_x: ctx.accounts.payer_share_ata.to_account_info(),
                funder: ctx.accounts.payer.to_account_info(),
                token_badge_x: None,
                token_badge_y: None,
                token_program_x: ctx.accounts.token_program.to_account_info(),
                token_program_y: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
                user_token_y: ctx.accounts.payer_wsol_ata.to_account_info(),
                event_authority: ctx.accounts.dlmm_event_authority.to_account_info(),
                program: ctx.accounts.dlmm_program.to_account_info(),
            },
        ),
        lb_clmm::types::CustomizableParams {
            active_id: args.launch_active_id,
            bin_step: preset.bin_step,
            base_factor,
            activation_type: dlmm::ACTIVATION_TYPE_TIMESTAMP,
            has_alpha_vault: false,
            activation_point: None,
            creator_pool_on_off_control: false,
            base_fee_power_factor,
            concrete_function_type: 0,
            collect_fee_mode: dlmm::COLLECT_FEE_MODE_ONLY_Y,
            padding: [0u8; 60],
        },
    )?;
    let pool_rent = payer_before.saturating_sub(ctx.accounts.payer.lamports());
    token::burn(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token::Burn {
                mint: ctx.accounts.share_mint.to_account_info(),
                from: ctx.accounts.payer_share_ata.to_account_info(),
                authority: ctx.accounts.payer.to_account_info(),
            },
        ),
        1,
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

    // Pool rent out of the deposit (after the last CPI).
    spend_deposit(&mut ctx.accounts.basket, &ctx.accounts.payer.to_account_info(), pool_rent, SPEND_POOL)?;

    let basket = &ctx.accounts.basket;
    emit!(BasketCreated {
        basket: basket.key(),
        share_mint,
        creator: basket.creator,
        payer: basket.payer,
        basket_type: basket.basket_type,
        profile,
        creator_tier: tier,
        asset_count: basket.asset_count,
        book_hash: basket.book_hash,
        nonce: basket.nonce,
        lb_pair,
        bin_step: preset.bin_step,
        launch_active_id: args.launch_active_id,
        deposit_lamports: basket.deposit_lamports,
        pool_rent_lamports: pool_rent,
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
    pub config: Box<Account<'info, Config>>,
    #[account(seeds = [seeds::WHITELIST], bump = whitelist.bump)]
    pub whitelist: Account<'info, Whitelist>,
    #[account(
        mut,
        seeds = [seeds::BASKET, basket.share_mint.as_ref()],
        bump = basket.bump,
        constraint = !basket.complete @ BasketError::ImmutableBasket,
    )]
    pub basket: Box<Account<'info, Basket>>,
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
