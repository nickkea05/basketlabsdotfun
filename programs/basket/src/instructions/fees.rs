//! Fee path (§4, §5; change order §2.5, Q1): `claim_pool_fees`,
//! `sweep_fees` / `sweep_fees_components`, `settle_fees`.
//!
//! Two fee sources, one FeeVault (`["fees", share_mint]`) per basket:
//!
//! * **Pool swap fees** arrive as SOL (`collect_fee_mode = OnlyY`).
//!   `claim_pool_fees` claims one position over a bin range into the
//!   basket's wSOL ATA, unwraps the delta onto the FeeVault and routes it
//!   straight away by the schedule frozen into the basket: creator (by tier)
//!   / BUYBACK PDA / team wallet / PRIZE PDA. The claimed total is recorded
//!   on the basket per prize epoch.
//! * **Mint / redeem fees** are withheld as shares in the FeeVault's share
//!   ATA. `sweep_fees` hands the creator their line in shares and redeems
//!   the rest in kind: components land in the FeeVault's own ATAs, the tight
//!   SOL leg is routed to the three protocol lines. Books that do not fit one
//!   transaction continue with `sweep_fees_components` chunks (a
//!   `Redemption` whose holder is the FeeVault). `settle_fees` then swaps one
//!   component ATA to SOL through an allow-listed venue and routes it.
//!
//! Nothing here prices anything; swap output is whatever the venue returns.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::program::invoke_signed;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token};
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface::TokenAccount;

use crate::constants::*;
use crate::dlmm;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::{load_components, read_token_amount, ComponentAccounts};
use crate::instructions::redeem::{pay_components, remove_sleeve, take_shares, Programs, Redeemer};
use crate::instructions::sleeve::*;
use crate::math;
use crate::state::*;

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// The four destinations of a fee split.
pub struct Routes<'a, 'info> {
    pub creator: &'a AccountInfo<'info>,
    pub buyback_vault: &'a AccountInfo<'info>,
    pub team_wallet: &'a AccountInfo<'info>,
    pub prize_vault: &'a AccountInfo<'info>,
}

/// Move `split` out of the FeeVault. The creator line goes out only if it
/// leaves the creator's wallet rent-exempt; otherwise it accrues in
/// `creator_owed_lamports` and rides along with the next payout. Call after
/// the instruction's last CPI.
pub fn route_from_fee_vault<'info>(
    fee_vault: &mut Account<'info, FeeVault>,
    buyback: &mut Account<'info, BuybackVault>,
    prize: &mut Account<'info, PrizeVault>,
    routes: &Routes<'_, 'info>,
    split: &FeeSplit,
) -> Result<u64> {
    let fee_vault_ai = fee_vault.to_account_info();
    let rent = Rent::get()?;
    let creator_due = fee_vault.creator_owed_lamports.checked_add(split.creator).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let creator_paid = if creator_due == 0 {
        0
    } else if routes.creator.lamports() + creator_due >= rent.minimum_balance(routes.creator.data_len()) {
        move_lamports(&fee_vault_ai, routes.creator, creator_due)?;
        creator_due
    } else {
        0
    };
    move_lamports(&fee_vault_ai, routes.buyback_vault, split.buyback)?;
    move_lamports(&fee_vault_ai, routes.team_wallet, split.team)?;
    move_lamports(&fee_vault_ai, routes.prize_vault, split.prize)?;
    fee_vault.creator_owed_lamports = creator_due - creator_paid;
    fee_vault.swept_creator_lamports += creator_paid;
    fee_vault.routed_buyback_lamports += split.buyback;
    fee_vault.routed_team_lamports += split.team;
    fee_vault.routed_prize_lamports += split.prize;
    buyback.received_lamports += split.buyback;
    prize.received_lamports += split.prize;
    Ok(creator_paid)
}

/// The three protocol lines' share of `amount` (the creator was already
/// paid in shares): buyback / team / prize in their configured proportions.
pub fn split_protocol_lines(fees: &FeeSchedule, amount: u64) -> Result<FeeSplit> {
    let total = fees.buyback_split_bps as u64 + fees.team_split_bps as u64 + fees.prize_split_bps as u64;
    if total == 0 {
        return Ok(FeeSplit { creator: 0, buyback: amount, team: 0, prize: 0 });
    }
    let buyback = math::mul_div_u64(amount, fees.buyback_split_bps as u64, total, math::Rounding::Down)?;
    let team = math::mul_div_u64(amount, fees.team_split_bps as u64, total, math::Rounding::Down)?;
    Ok(FeeSplit { creator: 0, buyback, team, prize: amount - buyback - team })
}

/// Unwrap the FeeVault's temporary wSOL ATA onto the FeeVault PDA; returns
/// `(lamports unwrapped, rent to hand back to `rent_to`)`.
fn unwrap_fee_vault_wsol<'info>(
    fee_vault: &Account<'info, FeeVault>,
    fee_vault_wsol_ata: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    share_mint: &Pubkey,
) -> Result<u64> {
    let amount = read_token_amount(fee_vault_wsol_ata)?;
    let fee_vault_ai = fee_vault.to_account_info();
    let fee_seeds: [&[u8]; 3] = [seeds::FEES, share_mint.as_ref(), &[fee_vault.bump]];
    token::close_account(CpiContext::new_with_signer(
        token_program.key(),
        token::CloseAccount { account: fee_vault_wsol_ata.clone(), destination: fee_vault_ai.clone(), authority: fee_vault_ai.clone() },
        &[&fee_seeds],
    ))?;
    Ok(amount)
}

// ---------------------------------------------------------------------------
// claim_pool_fees
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct ClaimPoolFees<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
    )]
    pub basket: Box<Account<'info, Basket>>,
    pub share_mint: Account<'info, Mint>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, wSOL); created, filled, closed onto the FeeVault within this ix.
    #[account(mut)]
    pub fee_vault_wsol_ata: UncheckedAccount<'info>,
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint).
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL): the claim lands here first.
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,
    /// CHECK: tight or backstop position, checked against the basket.
    #[account(mut)]
    pub position: UncheckedAccount<'info>,
    /// CHECK: the basket's DLMM pool.
    #[account(mut)]
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: pool reserve for shares.
    #[account(mut)]
    pub reserve_x: UncheckedAccount<'info>,
    /// CHECK: pool reserve for wSOL.
    #[account(mut)]
    pub reserve_y: UncheckedAccount<'info>,
    /// CHECK: lb_clmm event authority.
    pub dlmm_event_authority: UncheckedAccount<'info>,
    /// CHECK: the DLMM program.
    #[account(address = dlmm::LB_CLMM_ID)]
    pub dlmm_program: UncheckedAccount<'info>,
    /// CHECK: SPL Memo.
    #[account(address = dlmm::MEMO_PROGRAM_ID)]
    pub memo_program: UncheckedAccount<'info>,

    /// CHECK: `basket.creator`.
    #[account(mut)]
    pub creator: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::BUYBACK], bump = buyback_vault.bump)]
    pub buyback_vault: Account<'info, BuybackVault>,
    /// CHECK: `config.team_wallet`.
    #[account(mut, address = config.team_wallet @ BasketError::Unauthorized)]
    pub team_wallet: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::PRIZE], bump = prize_vault.bump)]
    pub prize_vault: Account<'info, PrizeVault>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Claim `[min_bin_id, max_bin_id]` of `position`'s fees (a bin array at a
/// time keeps the wide backstop within budget). Remaining accounts: the bin
/// arrays covering the range.
pub fn handle_claim_pool_fees<'info>(ctx: Context<'info, ClaimPoolFees<'info>>, min_bin_id: i32, max_bin_id: i32) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let basket = &ctx.accounts.basket;
    require!(basket.seeded, BasketError::NotSeeded);
    let position_key = ctx.accounts.position.key();
    let range = if basket.tight.is_set() && position_key == basket.tight.key {
        basket.tight
    } else if basket.backstop.is_set() && position_key == basket.backstop.key {
        basket.backstop
    } else {
        return err!(BasketError::PositionMismatch);
    };
    require!(
        min_bin_id <= max_bin_id && min_bin_id >= range.lower_bin_id && max_bin_id <= range.upper_bin_id,
        BasketError::PositionMismatch
    );

    let caller = ctx.accounts.caller.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let wsol_mint = ctx.accounts.wsol_mint.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();

    let dl = DlmmAccounts {
        program: &ctx.accounts.dlmm_program.to_account_info(),
        lb_pair: &ctx.accounts.lb_pair.to_account_info(),
        reserve_x: &ctx.accounts.reserve_x.to_account_info(),
        reserve_y: &ctx.accounts.reserve_y.to_account_info(),
        share_mint: &share_mint,
        wsol_mint: &wsol_mint,
        basket_share_ata: &ctx.accounts.basket_share_ata.to_account_info(),
        basket_wsol_ata: &ctx.accounts.basket_wsol_ata.to_account_info(),
        basket: &ctx.accounts.basket.to_account_info(),
        token_program: &token_program,
        memo_program: &ctx.accounts.memo_program.to_account_info(),
        event_authority: &ctx.accounts.dlmm_event_authority.to_account_info(),
        system_program: &system_program,
    };
    dl.check(basket)?;
    let signer = BasketSigner::new(basket);
    let arrays = bin_arrays_from_tail(ctx.remaining_accounts, &basket.pool.lb_pair, min_bin_id, max_bin_id)?;
    let claimed = dl.claim_fee(&signer, &ctx.accounts.position.to_account_info(), arrays, min_bin_id, max_bin_id)?;

    // Through the FeeVault's temporary wSOL account onto the PDA.
    let created = ctx.accounts.fee_vault_wsol_ata.data_is_empty();
    ensure_ata(&caller, &ctx.accounts.fee_vault_wsol_ata, &fee_vault_ai, &wsol_mint, &system_program, &token_program, &ata_program)?;
    let wsol_rent = if created { ctx.accounts.fee_vault_wsol_ata.lamports() } else { 0 };
    transfer_from_basket(dl.basket, dl.basket_wsol_ata, &ctx.accounts.fee_vault_wsol_ata, &token_program, &signer, claimed)?;
    let unwrapped = unwrap_fee_vault_wsol(&ctx.accounts.fee_vault, &ctx.accounts.fee_vault_wsol_ata, &token_program, &signer.share_mint)?;
    require!(unwrapped >= claimed, BasketError::MathOverflow);

    // Route (after the last CPI), including what the keeper parked.
    let parked = ctx.accounts.fee_vault.unrouted_lamports;
    let split = basket.fees.split(unwrapped + parked)?;
    let routes = Routes {
        creator: &ctx.accounts.creator.to_account_info(),
        buyback_vault: &ctx.accounts.buyback_vault.to_account_info(),
        team_wallet: &ctx.accounts.team_wallet.to_account_info(),
        prize_vault: &ctx.accounts.prize_vault.to_account_info(),
    };
    let creator_paid = route_from_fee_vault(&mut ctx.accounts.fee_vault, &mut ctx.accounts.buyback_vault, &mut ctx.accounts.prize_vault, &routes, &split)?;
    ctx.accounts.fee_vault.unrouted_lamports = 0;
    move_lamports(&fee_vault_ai, &caller, wsol_rent)?;

    let epoch = ctx.accounts.config.prizes.epoch_at(now);
    ctx.accounts.basket.record_pool_fees(epoch, unwrapped);
    emit!(PoolFeesClaimed {
        basket: ctx.accounts.basket.key(),
        position: position_key,
        lamports: unwrapped,
        creator_lamports: creator_paid,
        buyback_lamports: split.buyback,
        team_lamports: split.team,
        prize_lamports: split.prize,
        epoch,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// sweep_fees / sweep_fees_components
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct SweepFees<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
    )]
    pub basket: Box<Account<'info, Basket>>,
    #[account(mut)]
    pub share_mint: Account<'info, Mint>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint), address-checked in the handler.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(fee_vault, wSOL); created, filled, closed onto the FeeVault within this ix.
    #[account(mut)]
    pub fee_vault_wsol_ata: UncheckedAccount<'info>,
    /// CHECK: `basket.creator`.
    #[account(mut)]
    pub creator: UncheckedAccount<'info>,
    /// CHECK: ATA(creator, share_mint), created if missing (caller pays).
    #[account(mut)]
    pub creator_share_ata: UncheckedAccount<'info>,
    /// The in-kind redemption of the protocol lines, paid out by
    /// `sweep_fees_components`. One at a time per basket.
    #[account(
        init,
        payer = caller,
        space = 8 + Redemption::INIT_SPACE,
        seeds = [seeds::REDEMPTION, share_mint.key().as_ref(), fee_vault.key().as_ref()],
        bump,
    )]
    pub redemption: Account<'info, Redemption>,

    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint).
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL).
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,
    /// CHECK: the basket's DLMM pool.
    #[account(mut)]
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: pool reserve for shares.
    #[account(mut)]
    pub reserve_x: UncheckedAccount<'info>,
    /// CHECK: pool reserve for wSOL.
    #[account(mut)]
    pub reserve_y: UncheckedAccount<'info>,
    /// CHECK: the tight position.
    #[account(mut)]
    pub tight_position: UncheckedAccount<'info>,
    /// CHECK: the backstop position (any account when the basket has none).
    pub backstop_position: UncheckedAccount<'info>,
    /// CHECK: lb_clmm event authority.
    pub dlmm_event_authority: UncheckedAccount<'info>,
    /// CHECK: the DLMM program.
    #[account(address = dlmm::LB_CLMM_ID)]
    pub dlmm_program: UncheckedAccount<'info>,
    /// CHECK: SPL Memo.
    #[account(address = dlmm::MEMO_PROGRAM_ID)]
    pub memo_program: UncheckedAccount<'info>,

    #[account(mut, seeds = [seeds::BUYBACK], bump = buyback_vault.bump)]
    pub buyback_vault: Account<'info, BuybackVault>,
    /// CHECK: `config.team_wallet`.
    #[account(mut, address = config.team_wallet @ BasketError::Unauthorized)]
    pub team_wallet: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::PRIZE], bump = prize_vault.bump)]
    pub prize_vault: Account<'info, PrizeVault>,

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Remaining accounts: the pool's bin arrays (tight range; backstop bins
/// with liquidity).
pub fn handle_sweep_fees<'info>(ctx: Context<'info, SweepFees<'info>>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let basket = &ctx.accounts.basket;
    require!(basket.seeded, BasketError::NotSeeded);
    let basket_key = basket.key();
    let caller = ctx.accounts.caller.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let wsol_mint = ctx.accounts.wsol_mint.to_account_info();
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let fees = basket.fees;
    let fee_bump = [ctx.accounts.fee_vault.bump];
    let fee_seeds: [&[u8]; 3] = [seeds::FEES, basket.share_mint.as_ref(), &fee_bump];

    ensure_ata(&caller, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, programs.system, programs.token, programs.ata)?;
    let shares_total = read_token_amount(&ctx.accounts.fee_vault_share_ata)?;
    require!(shares_total > 0, BasketError::ZeroAmount);

    // Creator line in shares.
    let creator_shares = math::bps(shares_total, fees.creator_split_bps)?;
    if creator_shares > 0 {
        ensure_ata(&caller, &ctx.accounts.creator_share_ata, &ctx.accounts.creator, &share_mint, programs.system, programs.token, programs.ata)?;
        token::transfer(
            CpiContext::new_with_signer(
                programs.token.key(),
                token::Transfer {
                    from: ctx.accounts.fee_vault_share_ata.to_account_info(),
                    to: ctx.accounts.creator_share_ata.to_account_info(),
                    authority: fee_vault_ai.clone(),
                },
                &[&fee_seeds],
            ),
            creator_shares,
        )?;
    }
    let settle = shares_total - creator_shares;
    require!(settle > 0, BasketError::ZeroAmount);

    // The protocol lines: burn, tight SOL leg now, components in chunks.
    let view = PoolView::load(
        basket,
        &basket_key,
        &ctx.accounts.lb_pair.to_account_info(),
        Some(&ctx.accounts.tight_position.to_account_info()),
        Some(&ctx.accounts.backstop_position.to_account_info()),
        ctx.remaining_accounts,
    )?;
    let idle_shares = idle_amount(&ctx.accounts.basket_share_ata.to_account_info())?;
    let h = view.holder_shares(ctx.accounts.share_mint.supply, idle_shares, basket.pending_redeem_shares)?;
    let redeemer = Redeemer { ai: &fee_vault_ai, seeds: Some(fee_seeds), payer: &caller, record_frozen: false };
    let (net, _) = take_shares(&redeemer, &ctx.accounts.fee_vault_share_ata, &ctx.accounts.fee_vault_share_ata, &share_mint, programs.token, settle, 0)?;
    let h = h.max(net); // see handle_redeem

    let signer = BasketSigner::new(basket);
    let basket_ai = ctx.accounts.basket.to_account_info();
    let dl = DlmmAccounts {
        program: &ctx.accounts.dlmm_program.to_account_info(),
        lb_pair: &ctx.accounts.lb_pair.to_account_info(),
        reserve_x: &ctx.accounts.reserve_x.to_account_info(),
        reserve_y: &ctx.accounts.reserve_y.to_account_info(),
        share_mint: &share_mint,
        wsol_mint: &wsol_mint,
        basket_share_ata: &ctx.accounts.basket_share_ata.to_account_info(),
        basket_wsol_ata: &ctx.accounts.basket_wsol_ata.to_account_info(),
        basket: &basket_ai,
        token_program: programs.token,
        memo_program: &ctx.accounts.memo_program.to_account_info(),
        event_authority: &ctx.accounts.dlmm_event_authority.to_account_info(),
        system_program: programs.system,
    };
    dl.check(basket)?;
    let created = ctx.accounts.fee_vault_wsol_ata.data_is_empty();
    ensure_ata(&caller, &ctx.accounts.fee_vault_wsol_ata, &fee_vault_ai, &wsol_mint, programs.system, programs.token, programs.ata)?;
    let wsol_rent = if created { ctx.accounts.fee_vault_wsol_ata.lamports() } else { 0 };
    let sleeve = remove_sleeve(&view, net, h, &signer, &dl, &ctx.accounts.tight_position.to_account_info(), &ctx.accounts.fee_vault_wsol_ata)?;
    let unwrapped = unwrap_fee_vault_wsol(&ctx.accounts.fee_vault, &ctx.accounts.fee_vault_wsol_ata, programs.token, &signer.share_mint)?;

    // Route the SOL leg (after the last CPI), open the component redemption.
    let split = split_protocol_lines(&fees, unwrapped)?;
    let routes = Routes {
        creator: &ctx.accounts.creator.to_account_info(),
        buyback_vault: &ctx.accounts.buyback_vault.to_account_info(),
        team_wallet: &ctx.accounts.team_wallet.to_account_info(),
        prize_vault: &ctx.accounts.prize_vault.to_account_info(),
    };
    route_from_fee_vault(&mut ctx.accounts.fee_vault, &mut ctx.accounts.buyback_vault, &mut ctx.accounts.prize_vault, &routes, &split)?;
    move_lamports(&fee_vault_ai, &caller, wsol_rent)?;

    let basket = &mut ctx.accounts.basket;
    basket.pending_redeem_shares = basket.pending_redeem_shares.checked_add(net).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let red = &mut ctx.accounts.redemption;
    red.bump = ctx.bumps.redemption;
    red.basket = basket_key;
    red.holder = fee_vault_ai.key();
    red.shares = net;
    red.position_count = basket.position_count;
    red.paid_count = 0;
    red.paid_mints = Vec::new();
    red.created_at = now;
    let v = &mut ctx.accounts.fee_vault;
    v.swept_creator_shares += creator_shares;
    v.unsettled_shares = net;

    emit!(FeesSwept {
        basket: basket_key,
        creator_shares,
        creator_lamports: 0,
        settled_shares: net,
        pool_lamports: unwrapped,
        buyback_lamports: split.buyback,
        team_lamports: split.team,
        prize_lamports: split.prize,
        components_paid: 0,
        complete: false,
    });
    let _ = sleeve;
    Ok(())
}

#[derive(Accounts)]
pub struct SweepFeesComponents<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Box<Account<'info, Basket>>,
    pub share_mint: Account<'info, Mint>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    #[account(
        mut,
        seeds = [seeds::REDEMPTION, share_mint.key().as_ref(), fee_vault.key().as_ref()],
        bump = redemption.bump,
        constraint = redemption.holder == fee_vault.key() @ BasketError::Unauthorized,
    )]
    pub redemption: Account<'info, Redemption>,
    /// CHECK: ATA(basket, share_mint) — idle shares for the denominator.
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: the basket's DLMM pool.
    pub lb_pair: UncheckedAccount<'info>,
    /// CHECK: the tight position.
    pub tight_position: UncheckedAccount<'info>,
    /// CHECK: the backstop position (any account when the basket has none).
    pub backstop_position: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Remaining accounts: `[position, vault, fee_vault_ata, mint]` × `count`,
/// then the pool's bin arrays.
pub fn handle_sweep_fees_components<'info>(ctx: Context<'info, SweepFeesComponents<'info>>, count: u16) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    let n = count as usize;
    let components = load_components(&basket_key, ctx.remaining_accounts, n)?;
    let tail = ComponentAccounts::trailing(ctx.remaining_accounts, n);
    for c in &components {
        require!(!ctx.accounts.redemption.paid_mints.contains(&c.position.mint), BasketError::DuplicateMint);
    }
    let basket = &ctx.accounts.basket;
    let view = PoolView::load(
        basket,
        &basket_key,
        &ctx.accounts.lb_pair.to_account_info(),
        Some(&ctx.accounts.tight_position.to_account_info()),
        Some(&ctx.accounts.backstop_position.to_account_info()),
        tail,
    )?;
    let idle_shares = idle_amount(&ctx.accounts.basket_share_ata.to_account_info())?;
    let h = view.holder_shares(ctx.accounts.share_mint.supply, idle_shares, basket.pending_redeem_shares)?;
    let net = ctx.accounts.redemption.shares;
    let h = h.max(net); // see handle_redeem

    let caller = ctx.accounts.caller.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let fee_bump = [ctx.accounts.fee_vault.bump];
    let fee_seeds: [&[u8]; 3] = [seeds::FEES, basket.share_mint.as_ref(), &fee_bump];
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let redeemer = Redeemer { ai: &fee_vault_ai, seeds: Some(fee_seeds), payer: &caller, record_frozen: false };
    let signer = BasketSigner::new(basket);
    let basket_ai = ctx.accounts.basket.to_account_info();
    let legs = pay_components(&components, tail, net, h, &signer, &basket_ai, &basket_key, &redeemer, &programs, now)?;

    let red = &mut ctx.accounts.redemption;
    for c in &components {
        red.paid_mints.push(c.position.mint);
    }
    red.paid_count = red.paid_count.checked_add(legs.paid).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let complete = red.paid_count >= red.position_count;
    if complete {
        let basket = &mut ctx.accounts.basket;
        basket.pending_redeem_shares = basket.pending_redeem_shares.saturating_sub(net);
        ctx.accounts.fee_vault.unsettled_shares = 0;
        ctx.accounts.redemption.close(caller.clone())?;
    }
    emit!(FeesSwept {
        basket: basket_key,
        creator_shares: 0,
        creator_lamports: 0,
        settled_shares: net,
        pool_lamports: 0,
        buyback_lamports: 0,
        team_lamports: 0,
        prize_lamports: 0,
        components_paid: legs.paid,
        complete,
    });
    Ok(())
}

// ---------------------------------------------------------------------------
// settle_fees
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct SettleFees<'info> {
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Box<Account<'info, Config>>,
    /// CHECK: the share mint the FeeVault belongs to (seed only; the basket
    /// may already be closed).
    pub share_mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump, has_one = creator @ BasketError::Unauthorized)]
    pub fee_vault: Account<'info, FeeVault>,
    /// The FeeVault's component token account being sold.
    #[account(mut, constraint = in_ata.owner == fee_vault.key() @ BasketError::ComponentMismatch)]
    pub in_ata: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: ATA(fee_vault, wSOL); the swap must pay out here. Created if
    /// missing, closed onto the FeeVault afterwards.
    #[account(mut)]
    pub fee_vault_wsol_ata: UncheckedAccount<'info>,
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: must be in `config.swap_programs`.
    pub swap_program: UncheckedAccount<'info>,
    /// CHECK: `fee_vault.creator`.
    #[account(mut)]
    pub creator: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::BUYBACK], bump = buyback_vault.bump)]
    pub buyback_vault: Account<'info, BuybackVault>,
    /// CHECK: `config.team_wallet`.
    #[account(mut, address = config.team_wallet @ BasketError::Unauthorized)]
    pub team_wallet: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::PRIZE], bump = prize_vault.bump)]
    pub prize_vault: Account<'info, PrizeVault>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    // remaining: the inner instruction's accounts, in order. The FeeVault
    // PDA is passed unsigned; the program signs for it.
}

/// Is `ai` a token account owned by `owner`?
pub fn is_token_account_of(ai: &AccountInfo, owner: &Pubkey) -> bool {
    if *ai.owner != anchor_spl::token::ID && *ai.owner != anchor_spl::token_2022::ID {
        return false;
    }
    let Ok(data) = ai.try_borrow_data() else { return false };
    TokenAccount::try_deserialize(&mut &data[..]).map(|t| t.owner == *owner).unwrap_or(false)
}

/// Run one allow-listed swap for `authority` (a program PDA) over the
/// remaining accounts, letting it touch only `allowed` token accounts of the
/// authority's. Returns `(in_delta, out_delta)`.
#[allow(clippy::too_many_arguments)]
pub fn run_allowlisted_swap<'info>(
    swap_program: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    authority_seeds: &[&[u8]],
    allowed: &[Pubkey],
    in_ata: &AccountInfo<'info>,
    out_ata: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
    amount_in: u64,
    min_amount_out: u64,
    data: Vec<u8>,
) -> Result<(u64, u64)> {
    require!(amount_in > 0, BasketError::ZeroAmount);
    for ai in remaining {
        if allowed.contains(ai.key) {
            continue;
        }
        require!(!is_token_account_of(ai, authority.key), BasketError::UnexpectedAccountInSwap);
    }
    let in_before = read_token_amount(in_ata)?;
    let out_before = read_token_amount(out_ata)?;
    let lamports_before = authority.lamports();
    let metas: Vec<AccountMeta> = remaining
        .iter()
        .map(|ai| AccountMeta { pubkey: *ai.key, is_signer: ai.is_signer || ai.key == authority.key, is_writable: ai.is_writable })
        .collect();
    let ix = Instruction { program_id: swap_program.key(), accounts: metas, data };
    let mut infos: Vec<AccountInfo<'info>> = remaining.to_vec();
    infos.push(swap_program.clone());
    infos.push(authority.clone());
    invoke_signed(&ix, &infos, &[authority_seeds])?;
    require!(authority.lamports() >= lamports_before, BasketError::UnexpectedAccountInSwap);
    let in_delta = in_before.checked_sub(read_token_amount(in_ata)?).ok_or_else(|| error!(BasketError::InvalidArgument))?;
    require!(in_delta <= amount_in, BasketError::InvalidArgument);
    let out_delta = read_token_amount(out_ata)?.checked_sub(out_before).ok_or_else(|| error!(BasketError::SlippageExceeded))?;
    require!(out_delta >= min_amount_out, BasketError::SlippageExceeded);
    Ok((in_delta, out_delta))
}

pub fn handle_settle_fees<'info>(ctx: Context<'info, SettleFees<'info>>, amount_in: u64, min_amount_out: u64, data: Vec<u8>) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    // Keeper-only: a permissionless caller could pick the slippage and take
    // the difference through the venue (raised with the change order).
    require!(config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(config.allows_swap_program(ctx.accounts.swap_program.key), BasketError::SwapProgramNotAllowed);
    require!(ctx.accounts.in_ata.mint != ctx.accounts.wsol_mint.key(), BasketError::InvalidArgument);

    let keeper = ctx.accounts.keeper.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let created = ctx.accounts.fee_vault_wsol_ata.data_is_empty();
    ensure_ata(
        &keeper,
        &ctx.accounts.fee_vault_wsol_ata,
        &fee_vault_ai,
        &ctx.accounts.wsol_mint.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &token_program,
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;
    let wsol_rent = if created { ctx.accounts.fee_vault_wsol_ata.lamports() } else { 0 };

    let share_mint = ctx.accounts.share_mint.key();
    let fee_bump = [ctx.accounts.fee_vault.bump];
    let fee_seeds: [&[u8]; 3] = [seeds::FEES, share_mint.as_ref(), &fee_bump];
    let in_ata = ctx.accounts.in_ata.to_account_info();
    let allowed = [in_ata.key(), ctx.accounts.fee_vault_wsol_ata.key()];
    let (in_delta, out_delta) = run_allowlisted_swap(
        &ctx.accounts.swap_program.to_account_info(),
        &fee_vault_ai,
        &fee_seeds,
        &allowed,
        &in_ata,
        &ctx.accounts.fee_vault_wsol_ata,
        ctx.remaining_accounts,
        amount_in,
        min_amount_out,
        data,
    )?;
    let unwrapped = unwrap_fee_vault_wsol(&ctx.accounts.fee_vault, &ctx.accounts.fee_vault_wsol_ata, &token_program, &share_mint)?;

    let fees = ctx.accounts.fee_vault.fees;
    let split = split_protocol_lines(&fees, unwrapped)?;
    let routes = Routes {
        creator: &ctx.accounts.creator.to_account_info(),
        buyback_vault: &ctx.accounts.buyback_vault.to_account_info(),
        team_wallet: &ctx.accounts.team_wallet.to_account_info(),
        prize_vault: &ctx.accounts.prize_vault.to_account_info(),
    };
    route_from_fee_vault(&mut ctx.accounts.fee_vault, &mut ctx.accounts.buyback_vault, &mut ctx.accounts.prize_vault, &routes, &split)?;
    move_lamports(&fee_vault_ai, &keeper, wsol_rent)?;

    emit!(FeesSettled {
        basket: ctx.accounts.fee_vault.basket,
        mint: ctx.accounts.in_ata.mint,
        amount_in: in_delta,
        lamports_out: out_delta,
        buyback_lamports: split.buyback,
        team_lamports: split.team,
        prize_lamports: split.prize,
    });
    Ok(())
}
