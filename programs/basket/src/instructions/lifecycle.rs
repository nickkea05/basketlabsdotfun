//! Phase 7: the creator lock and the close crank. (Holder rewards — Merkle
//! roots, `distribute_rewards`, hold-time indexing — were removed by the
//! 2026-09-30 change order; their fee line goes to the buyback.)
//!
//! Creator lock: shares parked in ATA(CreatorLock, share_mint) until
//! `unlock_at`; top-ups extend, never shorten.
//!
//! Close: once no one outside the FeeVault holds shares and the basket has
//! been idle for `close_idle_s`, the keeper runs `close_positions` (vault
//! dust to the treasury, rent to the basket's payer) in chunks, then
//! `close_basket` (tight position drained and closed, sleeve SOL to the
//! treasury, leftover shares burned, deposit and rent to the payer). The
//! backstop is withdrawn and closed by the keeper instructions first; the
//! FeeVault is closed last by `close_fee_vault` once its components are
//! settled.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;
use anchor_lang::AccountsClose;
use anchor_spl::associated_token::{get_associated_token_address_with_program_id, AssociatedToken};
use anchor_spl::token::{self, Mint, Token, TokenAccount};
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface;

use crate::constants::*;
use crate::dlmm;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::read_token_amount;
use crate::instructions::sleeve::*;
use crate::state::*;

// ------------------------------------------------------ creator lock

#[derive(Accounts)]
pub struct LockCreatorShares<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
    )]
    pub basket: Box<Account<'info, Basket>>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    #[account(
        mut,
        constraint = creator_share_ata.owner == creator.key() @ BasketError::Unauthorized,
        constraint = creator_share_ata.mint == share_mint.key() @ BasketError::ComponentMismatch,
    )]
    pub creator_share_ata: Account<'info, TokenAccount>,
    /// CHECK: `["lock", share_mint]`, created on first use.
    #[account(mut, seeds = [seeds::LOCK, share_mint.key().as_ref()], bump)]
    pub lock: UncheckedAccount<'info>,
    /// CHECK: ATA(lock, share_mint), created if missing.
    #[account(mut)]
    pub lock_share_ata: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_lock_creator_shares(ctx: Context<LockCreatorShares>, amount: u64, duration_s: i64) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(amount > 0, BasketError::ZeroAmount);
    require!((MIN_CREATOR_LOCK_S..=MAX_CREATOR_LOCK_S).contains(&duration_s), BasketError::InvalidArgument);

    let lock_ai = ctx.accounts.lock.to_account_info();
    let creator = ctx.accounts.creator.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let bump = ctx.bumps.lock;
    let mut state = if lock_ai.data_is_empty() {
        let space = 8 + CreatorLock::INIT_SPACE;
        let lamports = Rent::get()?.minimum_balance(space);
        invoke_signed(
            &system_instruction::create_account(creator.key, lock_ai.key, lamports, space as u64, &crate::ID),
            &[creator.clone(), lock_ai.clone(), ctx.accounts.system_program.to_account_info()],
            &[&[seeds::LOCK, ctx.accounts.basket.share_mint.as_ref(), &[bump]]],
        )?;
        CreatorLock {
            bump,
            basket: ctx.accounts.basket.key(),
            creator: creator.key(),
            amount: 0,
            locked_at: now,
            unlock_at: now,
        }
    } else {
        let data = lock_ai.try_borrow_data()?;
        CreatorLock::try_deserialize(&mut &data[..])?
    };

    ensure_ata(
        &creator,
        &ctx.accounts.lock_share_ata,
        &lock_ai,
        &share_mint,
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.key(),
            token::Transfer {
                from: ctx.accounts.creator_share_ata.to_account_info(),
                to: ctx.accounts.lock_share_ata.to_account_info(),
                authority: creator.clone(),
            },
        ),
        amount,
    )?;

    state.amount = state.amount.checked_add(amount).ok_or_else(|| error!(BasketError::MathOverflow))?;
    state.unlock_at = state.unlock_at.max(now.saturating_add(duration_s));
    {
        let mut data = lock_ai.try_borrow_mut_data()?;
        state.try_serialize(&mut &mut data[..])?;
    }
    emit!(CreatorSharesLocked { basket: ctx.accounts.basket.key(), creator: creator.key(), amount, unlock_at: state.unlock_at });
    Ok(())
}

#[derive(Accounts)]
pub struct UnlockCreatorShares<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
    )]
    pub basket: Box<Account<'info, Basket>>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    /// CHECK: ATA(creator, share_mint), created if missing.
    #[account(mut)]
    pub creator_share_ata: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [seeds::LOCK, share_mint.key().as_ref()],
        bump = lock.bump,
        has_one = creator @ BasketError::Unauthorized,
        close = creator,
    )]
    pub lock: Account<'info, CreatorLock>,
    /// CHECK: ATA(lock, share_mint).
    #[account(mut)]
    pub lock_share_ata: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_unlock_creator_shares(ctx: Context<UnlockCreatorShares>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(now >= ctx.accounts.lock.unlock_at, BasketError::LockActive);
    let creator = ctx.accounts.creator.to_account_info();
    let lock_ai = ctx.accounts.lock.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let expected = get_associated_token_address_with_program_id(lock_ai.key, share_mint.key, token_program.key);
    require_keys_eq!(ctx.accounts.lock_share_ata.key(), expected, BasketError::ComponentMismatch);
    ensure_ata(
        &creator,
        &ctx.accounts.creator_share_ata,
        &creator,
        &share_mint,
        &ctx.accounts.system_program.to_account_info(),
        &token_program,
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;
    let amount = read_token_amount(&ctx.accounts.lock_share_ata)?;
    let lock_seeds: [&[u8]; 3] = [seeds::LOCK, ctx.accounts.basket.share_mint.as_ref(), &[ctx.accounts.lock.bump]];
    if amount > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                token_program.key(),
                token::Transfer {
                    from: ctx.accounts.lock_share_ata.to_account_info(),
                    to: ctx.accounts.creator_share_ata.to_account_info(),
                    authority: lock_ai.clone(),
                },
                &[&lock_seeds],
            ),
            amount,
        )?;
    }
    token::close_account(CpiContext::new_with_signer(
        token_program.key(),
        token::CloseAccount { account: ctx.accounts.lock_share_ata.to_account_info(), destination: creator.clone(), authority: lock_ai.clone() },
        &[&lock_seeds],
    ))?;
    emit!(CreatorSharesUnlocked { basket: ctx.accounts.basket.key(), creator: creator.key(), amount });
    Ok(())
}

// ------------------------------------------------------ close crank

/// Shared precondition: idle for `close_idle_s`, nothing in flight, the
/// backstop already withdrawn and closed, and nothing but dust outstanding
/// (`supply − shares in tight − idle shares ≤ CLOSE_DUST_SHARES`; the
/// FeeVault's own fee shares count as outstanding, so `sweep_fees` and the
/// creator's redemption come first).
fn require_closable(
    basket: &Basket,
    config: &Config,
    now: i64,
    supply: u64,
    view: Option<&PoolView>,
    idle_shares: u64,
    pending_book: &AccountInfo,
) -> Result<()> {
    require!(!basket.rebalance.active && basket.pending_redeem_shares == 0, BasketError::NotClosable);
    require!(pending_book.data_is_empty(), BasketError::NotClosable);
    require!(now >= basket.last_activity_at.saturating_add(config.close_idle_s), BasketError::NotClosable);
    require!(matches!(basket.backstop_state, BACKSTOP_UNPLACED | BACKSTOP_CLOSED), BasketError::BackstopState);
    let h = if basket.seeded {
        let view = view.ok_or_else(|| error!(BasketError::PoolMismatch))?;
        view.holder_shares(supply, idle_shares, 0)?
    } else {
        supply
    };
    // Bin-share ↔ amount conversion is only exact to a few base units after
    // partial removals; a stray holder of that much owns nothing.
    require!(h <= CLOSE_DUST_SHARES, BasketError::NotClosable);
    Ok(())
}

#[derive(Accounts)]
pub struct ClosePositions<'info> {
    /// Pays treasury ATAs for dust if missing.
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = payer @ BasketError::Unauthorized,
    )]
    pub basket: Box<Account<'info, Basket>>,
    pub share_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint), may not exist.
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: the basket's DLMM pool (any account when unseeded).
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: the tight position (any account when unseeded).
    pub tight_position: UncheckedAccount<'info>,
    /// CHECK: `["pending", share_mint]`, must be closed.
    #[account(seeds = [seeds::PENDING, share_mint.key().as_ref()], bump)]
    pub pending_book: UncheckedAccount<'info>,
    /// CHECK: `config.treasury`, receives vault dust.
    #[account(address = config.treasury @ BasketError::Unauthorized)]
    pub treasury: UncheckedAccount<'info>,
    /// CHECK: `basket.payer`, receives rent.
    #[account(mut)]
    pub payer: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    // remaining: `[position, vault, mint, treasury_ata]` × `count`, then
    // the tight range's bin arrays.
}

fn load_view_if_seeded<'a, 'info>(
    basket: &Basket,
    basket_key: &Pubkey,
    lb_pair: &'a AccountInfo<'info>,
    tight: &'a AccountInfo<'info>,
    tail: &'a [AccountInfo<'info>],
) -> Result<Option<PoolView<'a, 'info>>> {
    if !basket.seeded {
        return Ok(None);
    }
    Ok(Some(PoolView::load(basket, basket_key, lb_pair, Some(tight), None, tail)?))
}

pub fn handle_close_positions<'info>(ctx: Context<'info, ClosePositions<'info>>, count: u16) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    // Before the pool view: a backstop with liquidity needs its position in
    // the view, and the close crank does not carry it.
    require!(matches!(ctx.accounts.basket.backstop_state, BACKSTOP_UNPLACED | BACKSTOP_CLOSED), BasketError::BackstopState);
    let n = count as usize;
    require!(n >= 1 && ctx.remaining_accounts.len() >= n * 4, BasketError::ComponentCountMismatch);
    let (groups, tail) = ctx.remaining_accounts.split_at(n * 4);
    let lb_pair = ctx.accounts.lb_pair.to_account_info();
    let tight = ctx.accounts.tight_position.to_account_info();
    let view = load_view_if_seeded(&ctx.accounts.basket, &ctx.accounts.basket.key(), &lb_pair, &tight, tail)?;
    let idle_shares = idle_amount(&ctx.accounts.basket_share_ata.to_account_info())?;
    require_closable(
        &ctx.accounts.basket,
        &ctx.accounts.config,
        now,
        ctx.accounts.share_mint.supply,
        view.as_ref(),
        idle_shares,
        &ctx.accounts.pending_book,
    )?;

    let basket_key = ctx.accounts.basket.key();
    let basket_ai = ctx.accounts.basket.to_account_info();
    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_seeds = signer.basket_seeds();
    let keeper = ctx.accounts.keeper.to_account_info();
    let treasury = ctx.accounts.treasury.to_account_info();
    let payer = ctx.accounts.payer.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();

    let mut closing: Vec<Account<Position>> = Vec::with_capacity(n);
    for group in groups.chunks(4) {
        let (pos_ai, vault_ai, mint_ai, treasury_ata) = (&group[0], &group[1], &group[2], &group[3]);
        let position: Account<Position> = Account::try_from(pos_ai)?;
        require_keys_eq!(position.basket, basket_key, BasketError::ComponentMismatch);
        require_keys_eq!(vault_ai.key(), position.vault, BasketError::ComponentMismatch);
        require_keys_eq!(mint_ai.key(), position.mint, BasketError::ComponentMismatch);
        let tp = if position.token_program == token::ID {
            ctx.accounts.token_program.to_account_info()
        } else {
            ctx.accounts.token_2022_program.to_account_info()
        };
        let expected = get_associated_token_address_with_program_id(treasury.key, &position.mint, &position.token_program);
        require_keys_eq!(treasury_ata.key(), expected, BasketError::ComponentMismatch);

        let dust = read_token_amount(vault_ai)?;
        if dust > 0 {
            ensure_ata(&keeper, treasury_ata, &treasury, mint_ai, &system_program, &tp, &ata_program)?;
            token_interface::transfer_checked(
                CpiContext::new_with_signer(
                    tp.key(),
                    token_interface::TransferChecked {
                        from: vault_ai.clone(),
                        mint: mint_ai.clone(),
                        to: treasury_ata.clone(),
                        authority: basket_ai.clone(),
                    },
                    &[&basket_seeds],
                ),
                dust,
                position.decimals,
            )?;
        }
        token_interface::close_account(CpiContext::new_with_signer(
            tp.key(),
            token_interface::CloseAccount { account: vault_ai.clone(), destination: payer.clone(), authority: basket_ai.clone() },
            &[&basket_seeds],
        ))?;
        closing.push(position);
    }
    // Program-owned closes only after the last CPI: the runtime re-checks
    // the caller's lamport changes at every CPI boundary and a half-done
    // close (payer credited, position not yet in the callee's view) reads
    // as unbalanced.
    for position in &closing {
        position.close(payer.clone())?;
    }
    let b = &mut ctx.accounts.basket;
    b.position_count =
        b.position_count.checked_sub(closing.len() as u16).ok_or_else(|| error!(BasketError::MathOverflow))?;
    Ok(())
}

#[derive(Accounts)]
pub struct CloseBasket<'info> {
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
        has_one = payer @ BasketError::Unauthorized,
    )]
    pub basket: Box<Account<'info, Basket>>,
    #[account(mut)]
    pub share_mint: Account<'info, Mint>,
    /// Seeded baskets only. Stays open (see `close_fee_vault`).
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Option<Account<'info, FeeVault>>,
    /// CHECK: ATA(fee_vault, share_mint), may not exist.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// CHECK: `basket.creator`, receives any held-back creator SOL.
    #[account(mut)]
    pub creator: UncheckedAccount<'info>,
    /// CHECK: `config.treasury`.
    #[account(mut, address = config.treasury @ BasketError::Unauthorized)]
    pub treasury: UncheckedAccount<'info>,
    /// CHECK: ATA(treasury, wSOL), created if missing; receives the sleeve.
    #[account(mut)]
    pub treasury_wsol_ata: UncheckedAccount<'info>,
    /// CHECK: `basket.payer`, receives rent and the deposit.
    #[account(mut)]
    pub payer: UncheckedAccount<'info>,
    /// CHECK: `["pending", share_mint]`, must be closed.
    #[account(seeds = [seeds::PENDING, share_mint.key().as_ref()], bump)]
    pub pending_book: UncheckedAccount<'info>,
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint): withdrawn shares land here and are burned.
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL).
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,

    /// CHECK: the basket's DLMM pool (any account when unseeded).
    #[account(mut)]
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: pool reserve for shares.
    #[account(mut)]
    pub reserve_x: UncheckedAccount<'info>,
    /// CHECK: pool reserve for wSOL.
    #[account(mut)]
    pub reserve_y: UncheckedAccount<'info>,
    /// CHECK: the tight position (any account when unseeded).
    #[account(mut)]
    pub tight_position: UncheckedAccount<'info>,
    /// CHECK: lb_clmm event authority.
    pub dlmm_event_authority: UncheckedAccount<'info>,
    /// CHECK: the DLMM program.
    #[account(address = dlmm::LB_CLMM_ID)]
    pub dlmm_program: UncheckedAccount<'info>,
    /// CHECK: SPL Memo.
    #[account(address = dlmm::MEMO_PROGRAM_ID)]
    pub memo_program: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    // remaining: the tight range's bin arrays.
}

pub fn handle_close_basket<'info>(ctx: Context<'info, CloseBasket<'info>>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(matches!(ctx.accounts.basket.backstop_state, BACKSTOP_UNPLACED | BACKSTOP_CLOSED), BasketError::BackstopState);
    let lb_pair = ctx.accounts.lb_pair.to_account_info();
    let tight = ctx.accounts.tight_position.to_account_info();
    let view = load_view_if_seeded(&ctx.accounts.basket, &ctx.accounts.basket.key(), &lb_pair, &tight, ctx.remaining_accounts)?;
    let idle_shares = idle_amount(&ctx.accounts.basket_share_ata.to_account_info())?;
    require_closable(
        &ctx.accounts.basket,
        &ctx.accounts.config,
        now,
        ctx.accounts.share_mint.supply,
        view.as_ref(),
        idle_shares,
        &ctx.accounts.pending_book,
    )?;
    require!(ctx.accounts.basket.position_count == 0, BasketError::PositionsNotClosed);

    let basket_key = ctx.accounts.basket.key();
    let basket_ai = ctx.accounts.basket.to_account_info();
    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_seeds = signer.basket_seeds();
    let keeper = ctx.accounts.keeper.to_account_info();
    let payer = ctx.accounts.payer.to_account_info();
    let treasury = ctx.accounts.treasury.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let wsol_mint = ctx.accounts.wsol_mint.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    let mut sleeve_lamports = 0u64;

    if ctx.accounts.basket.seeded {
        let view = view.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let b = &ctx.accounts.basket;
        let dl = DlmmAccounts {
            program: &ctx.accounts.dlmm_program.to_account_info(),
            lb_pair: &lb_pair,
            reserve_x: &ctx.accounts.reserve_x.to_account_info(),
            reserve_y: &ctx.accounts.reserve_y.to_account_info(),
            share_mint: &share_mint,
            wsol_mint: &wsol_mint,
            basket_share_ata: &ctx.accounts.basket_share_ata.to_account_info(),
            basket_wsol_ata: &ctx.accounts.basket_wsol_ata.to_account_info(),
            basket: &basket_ai,
            token_program: &token_program,
            memo_program: &ctx.accounts.memo_program.to_account_info(),
            event_authority: &ctx.accounts.dlmm_event_authority.to_account_info(),
            system_program: &system_program,
        };
        dl.check(b)?;
        ensure_ata(&keeper, dl.basket_share_ata, &basket_ai, &share_mint, &system_program, &token_program, &ata_program)?;
        ensure_ata(&keeper, dl.basket_wsol_ata, &basket_ai, &wsol_mint, &system_program, &token_program, &ata_program)?;
        ensure_ata(&keeper, &ctx.accounts.treasury_wsol_ata, &treasury, &wsol_mint, &system_program, &token_program, &ata_program)?;

        // Tight out, fees claimed, position closed (rent to the payer).
        let t = b.tight;
        let has_liquidity = view.tight.map(|p| p.amount_x > 0 || p.amount_y > 0).unwrap_or(false);
        let arrays = view.bin_arrays_for(t.lower_bin_id, t.upper_bin_id)?;
        if has_liquidity {
            dl.remove_liquidity(&signer, &tight, arrays.clone(), t.lower_bin_id, t.upper_bin_id, BPS_TOTAL)?;
        }
        dl.claim_fee(&signer, &tight, arrays, t.lower_bin_id, t.upper_bin_id)?;
        dl.close_position(&signer, &tight, &payer)?;

        // Withdrawn treasury shares burned; the SOL side to the treasury.
        let shares_left = read_token_amount(dl.basket_share_ata)?;
        burn_basket_shares(&basket_ai, dl.basket_share_ata, &share_mint, &token_program, &signer, shares_left)?;
        token::close_account(CpiContext::new_with_signer(
            token_program.key(),
            token::CloseAccount { account: dl.basket_share_ata.clone(), destination: payer.clone(), authority: basket_ai.clone() },
            &[&basket_seeds],
        ))?;
        sleeve_lamports = read_token_amount(dl.basket_wsol_ata)?;
        transfer_from_basket(&basket_ai, dl.basket_wsol_ata, &ctx.accounts.treasury_wsol_ata, &token_program, &signer, sleeve_lamports)?;
        token::close_account(CpiContext::new_with_signer(
            token_program.key(),
            token::CloseAccount { account: dl.basket_wsol_ata.clone(), destination: payer.clone(), authority: basket_ai.clone() },
            &[&basket_seeds],
        ))?;

        // FeeVault: burn any fee-share dust and close the ATA; pay the
        // creator's held-back SOL; other free SOL (unrouted pool fees) to
        // the treasury. The FeeVault itself stays for `settle_fees`.
        let vault = ctx.accounts.fee_vault.as_mut().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let vault_ai = vault.to_account_info();
        let fee_bump = [vault.bump];
        let fee_seeds: [&[u8]; 3] = [seeds::FEES, b.share_mint.as_ref(), &fee_bump];
        let fee_ata = &ctx.accounts.fee_vault_share_ata;
        if !fee_ata.data_is_empty() {
            let expected = get_associated_token_address_with_program_id(vault_ai.key, share_mint.key, token_program.key);
            require_keys_eq!(fee_ata.key(), expected, BasketError::ComponentMismatch);
            let left = read_token_amount(fee_ata)?;
            if left > 0 {
                token::burn(
                    CpiContext::new_with_signer(
                        token_program.key(),
                        token::Burn { mint: share_mint.clone(), from: fee_ata.to_account_info(), authority: vault_ai.clone() },
                        &[&fee_seeds],
                    ),
                    left,
                )?;
            }
            token::close_account(CpiContext::new_with_signer(
                token_program.key(),
                token::CloseAccount { account: fee_ata.to_account_info(), destination: payer.clone(), authority: vault_ai.clone() },
                &[&fee_seeds],
            ))?;
        }
        let rent = Rent::get()?;
        let free = vault_ai.lamports().saturating_sub(rent.minimum_balance(vault_ai.data_len()));
        let creator_ai = ctx.accounts.creator.to_account_info();
        let owed = vault.creator_owed_lamports.min(free);
        let creator_paid = if owed > 0 && creator_ai.lamports() + owed >= rent.minimum_balance(creator_ai.data_len()) {
            move_lamports(&vault_ai, &creator_ai, owed)?;
            owed
        } else {
            0
        };
        move_lamports(&vault_ai, &treasury, free - creator_paid)?;
        vault.creator_owed_lamports -= creator_paid;
        vault.unrouted_lamports = 0;

        ctx.accounts.share_mint.reload()?;
        require!(ctx.accounts.share_mint.supply <= CLOSE_DUST_SHARES, BasketError::NotClosable);
    } else {
        require!(ctx.accounts.share_mint.supply == 0, BasketError::NotClosable);
    }

    let lamports = basket_ai.lamports();
    let rent_min = Rent::get()?.minimum_balance(basket_ai.data_len());
    let deposit_refunded = lamports.saturating_sub(rent_min);
    let deposit_spent = ctx.accounts.basket.deposit_spent_lamports;
    let refunded_to = payer.key();
    ctx.accounts.basket.close(payer)?;
    emit!(BasketClosed {
        basket: basket_key,
        share_mint: share_mint.key(),
        refunded_to,
        lamports,
        deposit_refunded,
        deposit_spent,
    });
    let _ = sleeve_lamports;
    Ok(())
}

#[derive(Accounts)]
pub struct CloseFeeVault<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    /// CHECK: the share mint the FeeVault belongs to (seed only).
    pub share_mint: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [seeds::FEES, share_mint.key().as_ref()],
        bump = fee_vault.bump,
        has_one = payer @ BasketError::Unauthorized,
        close = payer,
    )]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: the basket account; must be closed already.
    #[account(seeds = [seeds::BASKET, share_mint.key().as_ref()], bump)]
    pub basket: UncheckedAccount<'info>,
    /// CHECK: ATA(fee_vault, share_mint); must be closed already.
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// CHECK: `fee_vault.payer`, receives rent.
    #[account(mut)]
    pub payer: UncheckedAccount<'info>,
}

/// After `close_basket`, once the keeper has settled every FeeVault
/// component ATA: reclaim the FeeVault's rent for the basket's payer.
pub fn handle_close_fee_vault(ctx: Context<CloseFeeVault>) -> Result<()> {
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(ctx.accounts.basket.data_is_empty(), BasketError::NotClosable);
    require!(ctx.accounts.fee_vault_share_ata.data_is_empty(), BasketError::NotClosable);
    let v = &ctx.accounts.fee_vault;
    require!(v.unrouted_lamports == 0 && v.creator_owed_lamports == 0 && v.unsettled_shares == 0, BasketError::NotClosable);
    Ok(())
}
