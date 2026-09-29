//! Book changes (§5, D17, D20): `submit_book`, `open_position`,
//! `apply_book`, `execute_swap`, `close_position`, `finalize_rebalance`.
//!
//! A rebalance is a small state machine on `Basket.rebalance`:
//!
//! 1. `submit_book` stores the target list in `PendingBook`. Mirror and
//!    Strategy keepers open the trading window right away; a Managed
//!    creator's book waits for the timelock and `apply_book` (turnover cap,
//!    management-fee accrual). New mints get a `Position` + vault now (or
//!    later through `open_position`) so `redeem` pays out everything the
//!    basket holds while trades are in flight.
//! 2. `execute_swap` — keeper, inside the window — re-issues an inner
//!    instruction to an allow-listed venue with the basket PDA signing.
//!    Guards: only the two named vaults may be touched, no more than the
//!    declared amount leaves, at least the minimum arrives, output goes to a
//!    target position, and each position's cumulative sales stay within
//!    the weight change plus a tolerance.
//! 3. `close_position` removes emptied positions that left the book.
//! 4. `finalize_rebalance` walks the target in chunks, re-weights and
//!    re-orders positions; on the last chunk the chain hash must equal the
//!    target, weights must sum to 10_000 and no stray position may remain.
//!    `book_hash` flips, `mint` reopens, `PendingBook` rent goes back.
//!
//! `mint` is refused while `rebalance.active` (vault ratios are in flux);
//! `redeem` is not.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;
use anchor_lang::AccountsClose;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::Token;
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::{self, TokenAccount};

use crate::constants::*;
use crate::damm::cp_amm;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::crystallize::accrue_mgmt_fee;
use crate::instructions::positions::*;
use crate::instructions::sleeve::*;
use crate::state::*;

// ---------------------------------------------------------------- helpers

fn validate_book(book: &[PositionArg], whitelist: &Whitelist) -> Result<[u8; 32]> {
    require!(!book.is_empty() && book.len() <= PendingBook::MAX_BOOK, BasketError::InvalidArgument);
    let mut acc = [0u8; 32];
    let mut total: u16 = 0;
    for (i, e) in book.iter().enumerate() {
        require!(e.weight_bps > 0, BasketError::InvalidArgument);
        require!(book[..i].iter().all(|o| o.mint != e.mint), BasketError::DuplicateMint);
        require!(whitelist.contains(&e.mint), BasketError::MintNotWhitelisted);
        total = total.checked_add(e.weight_bps).ok_or_else(|| error!(BasketError::WeightsMustSumToTotal))?;
        acc = Basket::chain_hash(&acc, &e.mint, e.weight_bps);
    }
    require!(total == BPS_TOTAL, BasketError::WeightsMustSumToTotal);
    Ok(acc)
}

/// Who may write this basket's book: the creator for Managed, a keeper for
/// Mirror and Strategy. Fixed books never change.
fn check_book_authority(basket: &Basket, config: &Config, signer: &Pubkey) -> Result<()> {
    require!(basket.book_is_mutable(), BasketError::ImmutableBasket);
    let ok = if basket.is_managed() { *signer == basket.creator } else { config.is_keeper(signer) };
    require!(ok, BasketError::Unauthorized);
    Ok(())
}

/// Open the trading window on `pending`.
fn start_rebalance(basket: &mut Basket, pending: &PendingBook, config: &Config, now: i64) {
    let r = &mut basket.rebalance;
    r.active = true;
    r.seq = r.seq.wrapping_add(1);
    r.target_hash = pending.book_hash;
    r.target_count = pending.book.len() as u16;
    r.window_end_ts = now.saturating_add(config.rebalance_window_s);
    r.acc_count = 0;
    r.acc_hash = [0u8; 32];
    r.acc_weight = 0;
}

/// Σ|Δw| / 2 between two books, in bps.
fn turnover_bps(old: &[PositionArg], new: &[PositionArg]) -> u16 {
    let w = |book: &[PositionArg], m: &Pubkey| book.iter().find(|p| p.mint == *m).map(|p| p.weight_bps as i32).unwrap_or(0);
    let mut sum: i32 = 0;
    for e in old {
        sum += (w(new, &e.mint) - e.weight_bps as i32).abs();
    }
    for e in new {
        if !old.iter().any(|o| o.mint == e.mint) {
            sum += e.weight_bps as i32;
        }
    }
    (sum / 2) as u16
}

// ------------------------------------------------------------ submit_book

#[derive(Accounts)]
pub struct SubmitBook<'info> {
    /// Managed: the creator. Others: a keeper. Pays PendingBook rent and
    /// any new Position + vault.
    #[account(mut)]
    pub signer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(seeds = [seeds::WHITELIST], bump = whitelist.bump)]
    pub whitelist: Account<'info, Whitelist>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    /// CHECK: `["pending", share_mint]`, created here on first use.
    #[account(mut, seeds = [seeds::PENDING, share_mint.key().as_ref()], bump)]
    pub pending_book: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_submit_book<'info>(ctx: Context<'info, SubmitBook<'info>>, book: Vec<PositionArg>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    check_book_authority(&ctx.accounts.basket, config, ctx.accounts.signer.key)?;
    require!(ctx.accounts.basket.seeded, BasketError::NotSeeded);
    let book_hash = validate_book(&book, &ctx.accounts.whitelist)?;

    // An open window locks the target. An expired one is abandoned: the
    // positions it created stay until a keeper closes them.
    {
        let r = &ctx.accounts.basket.rebalance;
        require!(!r.active || now >= r.window_end_ts, BasketError::RebalanceActive);
    }
    ctx.accounts.basket.rebalance.active = false;

    // New positions: `[mint, position, vault]` per entering mint.
    let remaining = ctx.remaining_accounts;
    require!(remaining.len() % 3 == 0, BasketError::ComponentCountMismatch);
    if !remaining.is_empty() {
        let programs = PositionPrograms {
            payer: &ctx.accounts.signer.to_account_info(),
            token_program: &ctx.accounts.token_program.to_account_info(),
            token_2022_program: &ctx.accounts.token_2022_program.to_account_info(),
            associated_token_program: &ctx.accounts.associated_token_program.to_account_info(),
            system_program: &ctx.accounts.system_program.to_account_info(),
        };
        for group in remaining.chunks(3) {
            let mint = group[0].key();
            let arg = book.iter().find(|e| e.mint == mint).ok_or_else(|| error!(BasketError::OffBook))?;
            let index = ctx.accounts.basket.position_count;
            create_position(&ctx.accounts.basket, &ctx.accounts.whitelist, &programs, group, arg, index)?;
            ctx.accounts.basket.position_count =
                index.checked_add(1).ok_or_else(|| error!(BasketError::TooManyAssets))?;
        }
    }

    // PendingBook: create on first use, otherwise overwrite (rent stays
    // with whoever paid it).
    let pb_ai = ctx.accounts.pending_book.to_account_info();
    let share_mint = ctx.accounts.basket.share_mint;
    let bump = ctx.bumps.pending_book;
    let payer = if pb_ai.data_is_empty() {
        let space = 8 + PendingBook::INIT_SPACE;
        let lamports = Rent::get()?.minimum_balance(space);
        invoke_signed(
            &system_instruction::create_account(ctx.accounts.signer.key, pb_ai.key, lamports, space as u64, &crate::ID),
            &[ctx.accounts.signer.to_account_info(), pb_ai.clone(), ctx.accounts.system_program.to_account_info()],
            &[&[seeds::PENDING, share_mint.as_ref(), &[bump]]],
        )?;
        ctx.accounts.signer.key()
    } else {
        let data = pb_ai.try_borrow_data()?;
        PendingBook::try_deserialize(&mut &data[..])?.payer
    };
    let managed = ctx.accounts.basket.is_managed();
    let ready_at = if managed { now.saturating_add(ctx.accounts.basket.managed.timelock_s) } else { now };
    let pending = PendingBook {
        bump,
        basket: ctx.accounts.basket.key(),
        payer,
        submitted_at: now,
        ready_at,
        book_hash,
        book,
    };
    {
        let mut data = pb_ai.try_borrow_mut_data()?;
        pending.try_serialize(&mut &mut data[..])?;
    }

    if !managed {
        start_rebalance(&mut ctx.accounts.basket, &pending, config, now);
    }
    ctx.accounts.basket.touch(now);

    emit!(BookSubmitted {
        basket: ctx.accounts.basket.key(),
        by: ctx.accounts.signer.key(),
        book_hash,
        count: pending.book.len() as u16,
        ready_at,
    });
    Ok(())
}

// ---------------------------------------------------------- open_position

/// Create the Position + vault for a pending-book mint that did not come
/// with its accounts at `submit_book`. Permissionless while the book is
/// pending or the rebalance is active; the payer funds the rent.
#[derive(Accounts)]
pub struct OpenPosition<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(seeds = [seeds::WHITELIST], bump = whitelist.bump)]
    pub whitelist: Account<'info, Whitelist>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    #[account(seeds = [seeds::PENDING, share_mint.key().as_ref()], bump = pending_book.bump)]
    pub pending_book: Account<'info, PendingBook>,
    /// CHECK: must be in the pending book; token program read from the owner.
    pub mint: UncheckedAccount<'info>,
    /// CHECK: `["position", share_mint, mint]`, must not exist.
    #[account(mut)]
    pub position: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, mint).
    #[account(mut)]
    pub vault: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_open_position<'info>(ctx: Context<'info, OpenPosition<'info>>) -> Result<()> {
    require!(!ctx.accounts.config.paused, BasketError::Paused);
    let mint = ctx.accounts.mint.key();
    let weight = ctx.accounts.pending_book.weight_of(&mint).ok_or_else(|| error!(BasketError::OffBook))?;
    let arg = PositionArg { mint, weight_bps: weight };
    let programs = PositionPrograms {
        payer: &ctx.accounts.payer.to_account_info(),
        token_program: &ctx.accounts.token_program.to_account_info(),
        token_2022_program: &ctx.accounts.token_2022_program.to_account_info(),
        associated_token_program: &ctx.accounts.associated_token_program.to_account_info(),
        system_program: &ctx.accounts.system_program.to_account_info(),
    };
    let group = [
        ctx.accounts.mint.to_account_info(),
        ctx.accounts.position.to_account_info(),
        ctx.accounts.vault.to_account_info(),
    ];
    let index = ctx.accounts.basket.position_count;
    create_position(&ctx.accounts.basket, &ctx.accounts.whitelist, &programs, &group, &arg, index)?;
    ctx.accounts.basket.position_count = index.checked_add(1).ok_or_else(|| error!(BasketError::TooManyAssets))?;
    Ok(())
}

// ------------------------------------------------------------- apply_book

#[derive(Accounts)]
pub struct ApplyBook<'info> {
    /// Anyone; pays the creator share ATA if missing.
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
        has_one = pool @ BasketError::PoolMismatch,
        has_one = pool_position @ BasketError::PoolMismatch,
    )]
    pub basket: Account<'info, Basket>,
    #[account(mut)]
    pub share_mint: Account<'info, anchor_spl::token::Mint>,
    /// CHECK: mint authority PDA, seeds checked.
    #[account(seeds = [seeds::SHARE_AUTH, share_mint.key().as_ref()], bump = basket.share_auth_bump)]
    pub share_auth: UncheckedAccount<'info>,
    /// CHECK: `basket.creator`.
    pub creator: UncheckedAccount<'info>,
    /// CHECK: ATA(creator, share_mint), created if missing.
    #[account(mut)]
    pub creator_share_ata: UncheckedAccount<'info>,
    #[account(seeds = [seeds::PENDING, share_mint.key().as_ref()], bump = pending_book.bump)]
    pub pending_book: Account<'info, PendingBook>,
    pub pool: AccountLoader<'info, cp_amm::accounts::Pool>,
    pub pool_position: AccountLoader<'info, cp_amm::accounts::Position>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_apply_book(ctx: Context<ApplyBook>, current_book: Vec<PositionArg>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    {
        let b = &ctx.accounts.basket;
        require!(b.is_managed(), BasketError::InvalidBasketType);
        require!(!b.rebalance.active || now >= b.rebalance.window_end_ts, BasketError::RebalanceActive);
        require!(now >= ctx.accounts.pending_book.ready_at, BasketError::TimelockActive);
        // The caller supplies the live book; it must hash to the commitment.
        let mut acc = [0u8; 32];
        for e in &current_book {
            acc = Basket::chain_hash(&acc, &e.mint, e.weight_bps);
        }
        require!(acc == b.book_hash, BasketError::BookHashMismatch);
    }

    // Turnover budget per window.
    let turnover = turnover_bps(&current_book, &ctx.accounts.pending_book.book);
    {
        let b = &mut ctx.accounts.basket;
        let rules = b.managed;
        let r = &mut b.rebalance;
        if r.turnover_window_start_ts == 0 || now >= r.turnover_window_start_ts.saturating_add(rules.turnover_window_s) {
            r.turnover_window_start_ts = now;
            r.turnover_used_bps = 0;
        }
        let used = r.turnover_used_bps.checked_add(turnover).ok_or_else(|| error!(BasketError::MathOverflow))?;
        require!(used <= rules.turnover_cap_bps, BasketError::TurnoverExceeded);
        r.turnover_used_bps = used;
    }

    // Management fee up to now, so the fee base is the pre-trade book.
    let h = {
        let pool = ctx.accounts.pool.load()?;
        let position = ctx.accounts.pool_position.load()?;
        holder_shares(ctx.accounts.share_mint.supply, &pool, &position, ctx.accounts.basket.pending_redeem_shares)?
    };
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    ensure_ata(
        &ctx.accounts.caller.to_account_info(),
        &ctx.accounts.creator_share_ata,
        &ctx.accounts.creator,
        &share_mint,
        &ctx.accounts.system_program.to_account_info(),
        &token_program,
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;
    let mgmt_shares = accrue_mgmt_fee(
        &mut ctx.accounts.basket,
        h,
        now,
        &share_mint,
        &ctx.accounts.creator_share_ata,
        &ctx.accounts.share_auth.to_account_info(),
        &token_program,
    )?;

    start_rebalance(&mut ctx.accounts.basket, &ctx.accounts.pending_book, config, now);
    ctx.accounts.basket.touch(now);

    emit!(BookApplied {
        basket: ctx.accounts.basket.key(),
        book_hash: ctx.accounts.pending_book.book_hash,
        window_end_ts: ctx.accounts.basket.rebalance.window_end_ts,
        mgmt_fee_shares: mgmt_shares,
    });
    Ok(())
}

// ----------------------------------------------------------- execute_swap

#[derive(Accounts)]
pub struct ExecuteSwap<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    /// CHECK: `["pending", share_mint]`; read by hand so a missing account
    /// reports `NoRebalance` rather than a deserialization error.
    #[account(seeds = [seeds::PENDING, share_mint.key().as_ref()], bump)]
    pub pending_book: UncheckedAccount<'info>,
    #[account(mut, constraint = in_position.basket == basket.key() @ BasketError::ComponentMismatch)]
    pub in_position: Account<'info, Position>,
    #[account(mut, address = in_position.vault @ BasketError::ComponentMismatch)]
    pub in_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(constraint = out_position.basket == basket.key() @ BasketError::ComponentMismatch)]
    pub out_position: Account<'info, Position>,
    #[account(mut, address = out_position.vault @ BasketError::ComponentMismatch)]
    pub out_vault: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: must be in `config.swap_programs`.
    pub swap_program: UncheckedAccount<'info>,
    // remaining: the inner instruction's accounts, in order. The basket PDA
    // is passed unsigned; the program signs for it.
}

/// Is `ai` a token account owned by the basket PDA?
fn is_basket_token_account(ai: &AccountInfo, basket: &Pubkey) -> bool {
    if *ai.owner != anchor_spl::token::ID && *ai.owner != anchor_spl::token_2022::ID {
        return false;
    }
    let Ok(data) = ai.try_borrow_data() else { return false };
    TokenAccount::try_deserialize(&mut &data[..]).map(|t| t.owner == *basket).unwrap_or(false)
}

pub fn handle_execute_swap<'info>(
    ctx: Context<'info, ExecuteSwap<'info>>,
    amount_in: u64,
    min_amount_out: u64,
    data: Vec<u8>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    require!(config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    let basket_key = ctx.accounts.basket.key();
    let seq = {
        let r = &ctx.accounts.basket.rebalance;
        require!(r.active, BasketError::NoRebalance);
        require!(now < r.window_end_ts, BasketError::RebalanceWindowClosed);
        r.seq
    };
    require!(config.allows_swap_program(ctx.accounts.swap_program.key), BasketError::SwapProgramNotAllowed);
    require!(amount_in > 0, BasketError::ZeroAmount);
    let pending = {
        let ai = &ctx.accounts.pending_book;
        require!(!ai.data_is_empty(), BasketError::NoRebalance);
        let data = ai.try_borrow_data()?;
        PendingBook::try_deserialize(&mut &data[..])?
    };
    let in_mint = ctx.accounts.in_position.mint;
    let out_mint = ctx.accounts.out_position.mint;
    require!(in_mint != out_mint, BasketError::InvalidArgument);
    require!(pending.weight_of(&out_mint).is_some(), BasketError::OffBook);

    // The inner instruction may touch the two named vaults and nothing
    // else the basket owns.
    let in_vault_key = ctx.accounts.in_vault.key();
    let out_vault_key = ctx.accounts.out_vault.key();
    for ai in ctx.remaining_accounts {
        if *ai.key == in_vault_key || *ai.key == out_vault_key {
            continue;
        }
        require!(!is_basket_token_account(ai, &basket_key), BasketError::UnexpectedAccountInSwap);
    }

    let in_before = ctx.accounts.in_vault.amount;
    let out_before = ctx.accounts.out_vault.amount;
    let basket_lamports_before = ctx.accounts.basket.to_account_info().lamports();

    let metas: Vec<AccountMeta> = ctx
        .remaining_accounts
        .iter()
        .map(|ai| AccountMeta {
            pubkey: *ai.key,
            is_signer: ai.is_signer || *ai.key == basket_key,
            is_writable: ai.is_writable,
        })
        .collect();
    let ix = Instruction { program_id: ctx.accounts.swap_program.key(), accounts: metas, data };
    let mut infos: Vec<AccountInfo<'info>> = ctx.remaining_accounts.to_vec();
    infos.push(ctx.accounts.swap_program.to_account_info());
    infos.push(ctx.accounts.basket.to_account_info());
    let signer = BasketSigner::new(&ctx.accounts.basket);
    invoke_signed(&ix, &infos, &[&signer.basket_seeds()])?;

    ctx.accounts.in_vault.reload()?;
    ctx.accounts.out_vault.reload()?;
    require!(ctx.accounts.basket.to_account_info().lamports() >= basket_lamports_before, BasketError::UnexpectedAccountInSwap);
    let in_delta = in_before.checked_sub(ctx.accounts.in_vault.amount).ok_or_else(|| error!(BasketError::InvalidArgument))?;
    require!(in_delta <= amount_in, BasketError::InvalidArgument);
    let out_delta = ctx.accounts.out_vault.amount.checked_sub(out_before).ok_or_else(|| error!(BasketError::SlippageExceeded))?;
    require!(out_delta >= min_amount_out, BasketError::SlippageExceeded);

    // Cumulative sell cap from the weight change (+ tolerance) on the vault
    // as it stood when this rebalance started (= now + already sold).
    let pos = &mut ctx.accounts.in_position;
    if pos.rebalance_seq != seq {
        pos.rebalance_seq = seq;
        pos.sold = 0;
    }
    let w_old = pos.weight_bps as u128;
    let w_new = pending.weight_of(&in_mint).unwrap_or(0) as u128;
    let tol = config.rebalance_tolerance_bps as u128;
    let allowed_bps = if w_new >= w_old || w_old == 0 {
        tol
    } else {
        ((w_old - w_new) * BPS_TOTAL as u128 / w_old + tol).min(BPS_TOTAL as u128)
    };
    let base = in_before as u128 + pos.sold as u128;
    let cap = base * allowed_bps / BPS_TOTAL as u128;
    let sold = pos.sold.checked_add(in_delta).ok_or_else(|| error!(BasketError::MathOverflow))?;
    require!(sold as u128 <= cap, BasketError::SellCapExceeded);
    pos.sold = sold;

    emit!(SwapExecuted {
        basket: basket_key,
        keeper: ctx.accounts.keeper.key(),
        in_mint,
        out_mint,
        amount_in: in_delta,
        amount_out: out_delta,
    });
    Ok(())
}

// ---------------------------------------------------------- close_position

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    #[account(seeds = [seeds::PENDING, share_mint.key().as_ref()], bump = pending_book.bump)]
    pub pending_book: Account<'info, PendingBook>,
    #[account(
        mut,
        close = rent_payer,
        constraint = position.basket == basket.key() @ BasketError::ComponentMismatch,
        has_one = mint @ BasketError::ComponentMismatch,
        has_one = vault @ BasketError::ComponentMismatch,
    )]
    pub position: Account<'info, Position>,
    #[account(mut)]
    pub vault: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: `position.mint`.
    pub mint: UncheckedAccount<'info>,
    /// CHECK: `basket.payer` — funded the Position rent at creation.
    #[account(mut, address = basket.payer @ BasketError::Unauthorized)]
    pub rent_payer: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
}

pub fn handle_close_position(ctx: Context<ClosePosition>) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(ctx.accounts.basket.rebalance.active, BasketError::NoRebalance);
    let mint = ctx.accounts.position.mint;
    require!(ctx.accounts.pending_book.weight_of(&mint).is_none(), BasketError::OffBook);
    require!(ctx.accounts.vault.amount == 0 && ctx.accounts.position.owed == 0, BasketError::PositionNotEmpty);

    let signer = BasketSigner::new(&ctx.accounts.basket);
    let seeds = signer.basket_seeds();
    token_interface::close_account(CpiContext::new_with_signer(
        ctx.accounts.position.token_program,
        token_interface::CloseAccount {
            account: ctx.accounts.vault.to_account_info(),
            destination: ctx.accounts.rent_payer.to_account_info(),
            authority: ctx.accounts.basket.to_account_info(),
        },
        &[&seeds],
    ))?;
    let basket = &mut ctx.accounts.basket;
    basket.position_count = basket.position_count.checked_sub(1).ok_or_else(|| error!(BasketError::MathOverflow))?;
    Ok(())
}

// ------------------------------------------------------ finalize_rebalance

#[derive(Accounts)]
pub struct FinalizeRebalance<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::PENDING, share_mint.key().as_ref()], bump = pending_book.bump)]
    pub pending_book: Account<'info, PendingBook>,
    /// CHECK: `pending_book.payer`, refunded on completion.
    #[account(mut, address = pending_book.payer @ BasketError::Unauthorized)]
    pub pending_payer: UncheckedAccount<'info>,
    // remaining: one Position per entry, in order.
}

pub fn handle_finalize_rebalance<'info>(
    ctx: Context<'info, FinalizeRebalance<'info>>,
    entries: Vec<PositionArg>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(ctx.accounts.basket.rebalance.active, BasketError::NoRebalance);
    require!(!entries.is_empty(), BasketError::InvalidArgument);
    require!(ctx.remaining_accounts.len() == entries.len(), BasketError::ComponentCountMismatch);
    let basket_key = ctx.accounts.basket.key();

    let basket = &mut ctx.accounts.basket;
    let target_count = basket.rebalance.target_count;
    require!(
        basket.rebalance.acc_count as usize + entries.len() <= target_count as usize,
        BasketError::BookHashMismatch
    );
    for (ai, e) in ctx.remaining_accounts.iter().zip(&entries) {
        let mut position: Account<Position> = Account::try_from(ai)?;
        require_keys_eq!(position.basket, basket_key, BasketError::ComponentMismatch);
        require_keys_eq!(position.mint, e.mint, BasketError::ComponentMismatch);
        require!(e.weight_bps > 0, BasketError::InvalidArgument);
        let r = &mut basket.rebalance;
        position.weight_bps = e.weight_bps;
        position.index = r.acc_count;
        {
            let mut data = ai.try_borrow_mut_data()?;
            position.try_serialize(&mut &mut data[..])?;
        }
        r.acc_hash = Basket::chain_hash(&r.acc_hash, &e.mint, e.weight_bps);
        r.acc_weight = r.acc_weight.checked_add(e.weight_bps).ok_or_else(|| error!(BasketError::WeightsMustSumToTotal))?;
        r.acc_count += 1;
    }

    if basket.rebalance.acc_count < target_count {
        basket.touch(now);
        return Ok(());
    }

    // Last chunk: everything must line up.
    require!(basket.position_count == target_count, BasketError::PositionsNotClosed);
    require!(basket.rebalance.acc_hash == basket.rebalance.target_hash, BasketError::BookHashMismatch);
    require!(basket.rebalance.acc_weight == BPS_TOTAL, BasketError::WeightsMustSumToTotal);
    basket.book_hash = basket.rebalance.target_hash;
    basket.book_acc = basket.rebalance.target_hash;
    basket.weight_acc = BPS_TOTAL;
    basket.asset_count = target_count;
    basket.rebalance.active = false;
    basket.touch(now);

    ctx.accounts.pending_book.close(ctx.accounts.pending_payer.to_account_info())?;

    emit!(RebalanceFinalized { basket: basket_key, book_hash: basket.book_hash, position_count: target_count });
    Ok(())
}
