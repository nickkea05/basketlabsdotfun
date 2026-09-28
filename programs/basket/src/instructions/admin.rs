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
    pub config: Account<'info, Config>,
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
    config.whitelist_authority = ctx.accounts.admin.key();
    config.keepers = keepers;
    config.fees = FeeDefaults::default();
    config.sleeve = SleeveDefaults::default();
    config.managed = ManagedRules::default();
    config.rebalance_window_s = DEFAULT_REBALANCE_WINDOW_S;
    config.close_idle_s = DEFAULT_CLOSE_IDLE_S;
    config.graduation_lamports = DEFAULT_GRADUATION_LAMPORTS;
    config.scheduler = LaunchScheduler::default();
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
    if let Some(v) = u.graduation_lamports {
        config.graduation_lamports = v;
    }
    if let Some(v) = u.scheduler {
        require!(v.cliff_bps <= 9_900 && v.period_s > 0, BasketError::InvalidArgument);
        config.scheduler = v;
    }
    Ok(())
}

#[derive(Accounts)]
pub struct AdminOnly<'info> {
    pub admin: Signer<'info>,
    #[account(mut, seeds = [seeds::CONFIG], bump = config.bump, has_one = admin @ BasketError::Unauthorized)]
    pub config: Account<'info, Config>,
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
    pub config: Account<'info, Config>,
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
