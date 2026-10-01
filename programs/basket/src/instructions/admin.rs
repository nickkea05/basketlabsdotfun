//! `initialize_config`, `update_config`, `rotate_keepers`, `set_paused`,
//! `update_whitelist` (§5, D17, D18).

use anchor_lang::prelude::*;

use crate::constants::*;
use crate::error::BasketError;
use crate::events::*;
use crate::state::*;

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        init,
        payer = admin,
        space = 8 + Config::INIT_SPACE,
        seeds = [seeds::CONFIG],
        bump,
    )]
    pub config: Box<Account<'info, Config>>,
    #[account(
        init,
        payer = admin,
        space = Whitelist::space_for(0),
        seeds = [seeds::WHITELIST],
        bump,
    )]
    pub whitelist: Account<'info, Whitelist>,
    pub system_program: Program<'info, System>,
}

pub fn handle_initialize_config(
    ctx: Context<InitializeConfig>,
    cluster: u8,
    treasury: Pubkey,
    keepers: Vec<Pubkey>,
    update: ConfigUpdate,
) -> Result<()> {
    require!(cluster <= 2, BasketError::InvalidArgument);
    require!(keepers.len() <= MAX_KEEPERS, BasketError::InvalidArgument);

    let config = &mut ctx.accounts.config;
    config.bump = ctx.bumps.config;
    config.cluster = cluster;
    config.paused = false;
    config.admin = ctx.accounts.admin.key();
    config.treasury = treasury;
    // Devnet placeholder until the Squads vault exists (Q13).
    config.team_wallet = treasury;
    config.whitelist_authority = ctx.accounts.admin.key();
    config.keepers = keepers;
    config.fees = FeeDefaults::default();
    config.sleeve = SleeveDefaults::default();
    config.managed = ManagedRules::default();
    config.rebalance_window_s = DEFAULT_REBALANCE_WINDOW_S;
    config.close_idle_s = DEFAULT_CLOSE_IDLE_S;
    config.pools = PoolDefaults::default();
    config.deposit_lamports = DEFAULT_DEPOSIT_LAMPORTS;
    config.mint_window = MintWindow::default();
    config.allow_open_gate = false;
    config.bskt_mint = None;
    config.prizes = PrizeParams::default();
    config.prizes.epoch_anchor_ts = Clock::get()?.unix_timestamp;
    config.creator_tier_bps = [DEFAULT_CREATOR_SPLIT_BPS; TIER_COUNT];
    config.swap_programs = vec![JUPITER_V6_ID];
    config.rebalance_tolerance_bps = DEFAULT_REBALANCE_TOLERANCE_BPS;
    apply_update(config, update)?;

    let whitelist = &mut ctx.accounts.whitelist;
    whitelist.bump = ctx.bumps.whitelist;
    whitelist.entries = Vec::new();

    emit!(ConfigInitialized {
        admin: config.admin,
        treasury: config.treasury,
        cluster,
    });
    Ok(())
}

fn apply_update(config: &mut Config, u: ConfigUpdate) -> Result<()> {
    if let Some(v) = u.admin {
        config.admin = v;
    }
    if let Some(v) = u.treasury {
        config.treasury = v;
    }
    if let Some(v) = u.team_wallet {
        config.team_wallet = v;
    }
    if let Some(v) = u.whitelist_authority {
        config.whitelist_authority = v;
    }
    if let Some(v) = u.fees {
        require!(v.validate(), BasketError::FeeOutOfBounds);
        config.fees = v;
    }
    if let Some(v) = u.sleeve {
        require!(v.validate(), BasketError::SleeveOutOfBand);
        config.sleeve = v;
    }
    if let Some(v) = u.managed {
        require!(
            v.timelock_s >= 0 && v.turnover_window_s > 0 && v.crystallize_period_s > 0 && v.turnover_cap_bps <= BPS_TOTAL,
            BasketError::InvalidArgument
        );
        config.managed = v;
    }
    if let Some(v) = u.rebalance_window_s {
        require!(v > 0, BasketError::InvalidArgument);
        config.rebalance_window_s = v;
    }
    if let Some(v) = u.close_idle_s {
        require!(v > 0, BasketError::InvalidArgument);
        config.close_idle_s = v;
    }
    if let Some(v) = u.pools {
        require!(v.validate(), BasketError::InvalidArgument);
        config.pools = v;
    }
    if let Some(v) = u.deposit_lamports {
        config.deposit_lamports = v;
    }
    if let Some(v) = u.mint_window {
        require!(v.validate(), BasketError::InvalidGate);
        config.mint_window = v;
    }
    if let Some(v) = u.allow_open_gate {
        config.allow_open_gate = v;
    }
    if let Some(v) = u.prizes {
        require!(v.validate(), BasketError::InvalidArgument);
        config.prizes = v;
    }
    if let Some(v) = u.creator_tier_bps {
        require!(v.iter().all(|b| *b <= BPS_TOTAL), BasketError::FeeOutOfBounds);
        config.creator_tier_bps = v;
    }
    if let Some(v) = u.swap_programs {
        require!(v.len() <= MAX_SWAP_PROGRAMS, BasketError::InvalidArgument);
        config.swap_programs = v;
    }
    if let Some(v) = u.rebalance_tolerance_bps {
        require!(v <= BPS_TOTAL, BasketError::InvalidArgument);
        config.rebalance_tolerance_bps = v;
    }
    Ok(())
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [seeds::CONFIG], bump = config.bump, has_one = admin @ BasketError::Unauthorized)]
    pub config: Box<Account<'info, Config>>,
}

pub fn handle_update_config(ctx: Context<AdminOnly>, update: ConfigUpdate) -> Result<()> {
    apply_update(&mut ctx.accounts.config, update)?;
    emit!(ConfigUpdated { admin: ctx.accounts.config.admin });
    Ok(())
}

pub fn handle_rotate_keepers(ctx: Context<AdminOnly>, keepers: Vec<Pubkey>) -> Result<()> {
    require!(keepers.len() <= MAX_KEEPERS, BasketError::InvalidArgument);
    ctx.accounts.config.keepers = keepers.clone();
    emit!(KeepersRotated { keepers });
    Ok(())
}

pub fn handle_set_paused(ctx: Context<AdminOnly>, paused: bool) -> Result<()> {
    ctx.accounts.config.paused = paused;
    emit!(PauseChanged { paused });
    Ok(())
}

/// Q8: set exactly once; immutable afterwards. The timelock is the Squads
/// multisig that holds `admin`.
pub fn handle_set_bskt_mint(ctx: Context<AdminOnly>, mint: Pubkey) -> Result<()> {
    require!(ctx.accounts.config.bskt_mint.is_none(), BasketError::BsktMintState);
    ctx.accounts.config.bskt_mint = Some(mint);
    emit!(BsktMintSet { mint });
    Ok(())
}

#[derive(Accounts)]
#[instruction(add: Vec<WhitelistEntry>, remove: Vec<Pubkey>)]
pub struct UpdateWhitelist<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        seeds = [seeds::CONFIG],
        bump = config.bump,
        constraint = config.whitelist_authority == authority.key() || config.admin == authority.key() @ BasketError::Unauthorized,
    )]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [seeds::WHITELIST],
        bump = whitelist.bump,
        realloc = Whitelist::space_for(whitelist.entries.len() + add.len()),
        realloc::payer = authority,
        realloc::zero = false,
    )]
    pub whitelist: Account<'info, Whitelist>,
    pub system_program: Program<'info, System>,
}

pub fn handle_update_whitelist(
    ctx: Context<UpdateWhitelist>,
    add: Vec<WhitelistEntry>,
    remove: Vec<Pubkey>,
) -> Result<()> {
    let wl = &mut ctx.accounts.whitelist;
    require!(
        wl.entries.len() + add.len() <= Whitelist::MAX_ENTRIES,
        BasketError::WhitelistFull
    );
    let mut added = 0u32;
    let mut removed = 0u32;
    for e in add {
        if wl.upsert(e.mint, e.tier) {
            added += 1;
        }
    }
    for m in remove {
        if wl.remove(&m) {
            removed += 1;
        }
    }
    emit!(WhitelistUpdated { added, removed, total: wl.entries.len() as u32 });
    Ok(())
}
