//! `redeem`, `redeem_begin` / `redeem_components`, `claim_frozen` (D9, D10,
//! §4, §5; change order Q14).
//!
//! A redemption of `shares` (gross) withholds the redeem fee as shares into
//! the FeeVault, burns the net amount, and pays `net / H` of everything the
//! holders own: each component vault (less amounts already owed to frozen
//! claimants), the SOL in the **tight** position plus the idle sleeve, and
//! burns the treasury shares that come out with it. `H` is the holder-share
//! count before the burn. The backstop is not touched (Q14: it is a
//! protocol-level guarantee, counted in NAV and settled at close). Redeem
//! has no gate and ignores the pause flag.
//!
//! The tight leg is `remove_liquidity_by_range2(bps)` with
//! `bps = ⌊net·10⁴/H⌋`; the rounding shortfall against the exact pro-rata is
//! made up from idle SOL when there is any.
//!
//! If a component's vault or the holder's ATA is frozen (D10) the leg is
//! recorded in a `FrozenClaim` and `Position.owed` instead of reverting; the
//! holder passes the claim PDAs after the component groups. Books that do
//! not fit one transaction use `redeem_begin` (burn + pool leg, opens a
//! `Redemption`) followed by `redeem_components` chunks; the burned shares
//! stay in the pro-rata denominator via `Basket.pending_redeem_shares` until
//! every component is paid.
//!
//! Remaining accounts: `[position, vault, holder_ata, mint]` per component
//! in book order, then the pool's bin arrays (tight range; backstop bins
//! with liquidity) and any FrozenClaim PDAs, in any order.

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token};
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface;

use crate::constants::*;
use crate::dlmm;
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::{load_components, read_token_amount, ComponentAccounts};
use crate::instructions::sleeve::*;
use crate::math::{self, Rounding};
use crate::state::*;

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub holder: Signer<'info>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Box<Account<'info, Basket>>,
    #[account(mut)]
    pub share_mint: Account<'info, Mint>,
    /// CHECK: holder's share ATA (any share token account the holder owns works).
    #[account(mut)]
    pub holder_share_ata: UncheckedAccount<'info>,
    #[account(seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint), address-checked.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,

    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint): withdrawn treasury shares land here and are burned.
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL).
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(holder, wSOL): receives the SOL leg, closed if created here.
    #[account(mut)]
    pub holder_wsol_ata: UncheckedAccount<'info>,

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

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RedeemBegin<'info> {
    #[account(mut)]
    pub holder: Signer<'info>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Box<Account<'info, Basket>>,
    #[account(mut)]
    pub share_mint: Account<'info, Mint>,
    /// CHECK: holder's share token account.
    #[account(mut)]
    pub holder_share_ata: UncheckedAccount<'info>,
    #[account(seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint), address-checked.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// One open redemption per holder per basket.
    #[account(
        init,
        payer = holder,
        space = 8 + Redemption::INIT_SPACE,
        seeds = [seeds::REDEMPTION, share_mint.key().as_ref(), holder.key().as_ref()],
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
    /// CHECK: ATA(holder, wSOL).
    #[account(mut)]
    pub holder_wsol_ata: UncheckedAccount<'info>,

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

    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RedeemComponents<'info> {
    #[account(mut)]
    pub holder: Signer<'info>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
    )]
    pub basket: Box<Account<'info, Basket>>,
    pub share_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [seeds::REDEMPTION, share_mint.key().as_ref(), holder.key().as_ref()],
        bump = redemption.bump,
        has_one = holder,
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

#[derive(Accounts)]
pub struct ClaimFrozen<'info> {
    #[account(mut)]
    pub wallet: Signer<'info>,
    #[account(seeds = [seeds::BASKET, share_mint.key().as_ref()], bump = basket.bump, has_one = share_mint)]
    pub basket: Box<Account<'info, Basket>>,
    pub share_mint: Account<'info, Mint>,
    #[account(
        mut,
        close = wallet,
        seeds = [seeds::CLAIM, share_mint.key().as_ref(), wallet.key().as_ref(), mint.key().as_ref()],
        bump = claim.bump,
        has_one = wallet,
        has_one = mint,
    )]
    pub claim: Account<'info, FrozenClaim>,
    #[account(
        mut,
        seeds = [seeds::POSITION, share_mint.key().as_ref(), mint.key().as_ref()],
        bump = position.bump,
        has_one = mint,
        has_one = vault,
    )]
    pub position: Account<'info, Position>,
    /// CHECK: basket vault ATA, checked via `position.vault`.
    #[account(mut)]
    pub vault: UncheckedAccount<'info>,
    /// CHECK: ATA(wallet, mint), created if missing.
    #[account(mut)]
    pub wallet_ata: UncheckedAccount<'info>,
    /// CHECK: component mint, checked via `position.mint`.
    pub mint: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

// ---------------------------------------------------------------------------
// Shared pieces (also used by `sweep_fees`, where the FeeVault is the holder)
// ---------------------------------------------------------------------------

pub struct Programs<'a, 'info> {
    pub token: &'a AccountInfo<'info>,
    pub token_2022: &'a AccountInfo<'info>,
    pub ata: &'a AccountInfo<'info>,
    pub system: &'a AccountInfo<'info>,
}

/// Whose shares are being redeemed: a wallet signing the transaction, or a
/// program PDA (the FeeVault) signing through `seeds`.
pub struct Redeemer<'a, 'info> {
    pub ai: &'a AccountInfo<'info>,
    pub seeds: Option<[&'a [u8]; 3]>,
    /// Pays rent for ATAs / claims created along the way.
    pub payer: &'a AccountInfo<'info>,
    /// Frozen legs: record a `FrozenClaim` (wallets) or forfeit (FeeVault).
    pub record_frozen: bool,
}

impl<'a, 'info> Redeemer<'a, 'info> {
    fn signer_seeds(&self) -> Vec<&'a [u8]> {
        self.seeds.map(|s| s.to_vec()).unwrap_or_default()
    }
}

/// Fee to the FeeVault (as shares), net burned. Returns `(net, fee)`.
pub fn take_shares<'info>(
    redeemer: &Redeemer<'_, 'info>,
    holder_share_ata: &AccountInfo<'info>,
    fee_vault_share_ata: &AccountInfo<'info>,
    share_mint: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    gross: u64,
    fee_bps: u16,
) -> Result<(u64, u64)> {
    require!(gross > 0, BasketError::ZeroAmount);
    let fee = math::bps(gross, fee_bps)?;
    let net = gross - fee;
    require!(net > 0, BasketError::ZeroAmount);
    let seeds = redeemer.signer_seeds();
    let seed_set = [seeds.as_slice()];
    let signers: &[&[&[u8]]] = if seeds.is_empty() { &[] } else { &seed_set };
    if fee > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                token_program.key(),
                token::Transfer { from: holder_share_ata.clone(), to: fee_vault_share_ata.clone(), authority: redeemer.ai.clone() },
                signers,
            ),
            fee,
        )?;
    }
    token::burn(
        CpiContext::new_with_signer(
            token_program.key(),
            token::Burn { mint: share_mint.clone(), from: holder_share_ata.clone(), authority: redeemer.ai.clone() },
            signers,
        ),
        net,
    )?;
    Ok((net, fee))
}

pub struct SleeveOut {
    pub treasury_shares_burned: u64,
    pub lamports_out: u64,
}

/// Remove `net / h` of the tight position (`bps` granularity, shortfall
/// covered from idle SOL) plus `net / h` of the idle sleeve: SOL to
/// `dest_wsol_ata`, treasury shares burned.
#[allow(clippy::too_many_arguments)]
pub fn remove_sleeve<'info>(
    view: &PoolView<'_, 'info>,
    net: u64,
    h: u64,
    signer: &BasketSigner,
    dl: &DlmmAccounts<'_, 'info>,
    tight_position: &AccountInfo<'info>,
    dest_wsol_ata: &AccountInfo<'info>,
) -> Result<SleeveOut> {
    require!(h > 0, BasketError::ZeroAmount);
    let idle_x = idle_amount(dl.basket_share_ata)?;
    let idle_y = idle_amount(dl.basket_wsol_ata)?;
    let tight = view.tight.unwrap_or_default();

    let bps = math::mul_div_u64(net, BPS_TOTAL as u64, h, Rounding::Down)?.min(BPS_TOTAL as u64) as u16;
    let mut x_out = 0u64;
    let mut y_out = 0u64;
    if bps > 0 && (tight.amount_x > 0 || tight.amount_y > 0) {
        let arrays = view.bin_arrays_for(tight.lower_bin_id, tight.upper_bin_id)?;
        dl.remove_liquidity(signer, tight_position, arrays, tight.lower_bin_id, tight.upper_bin_id, bps)?;
        x_out = read_token_amount(dl.basket_share_ata)?.saturating_sub(idle_x);
        y_out = read_token_amount(dl.basket_wsol_ata)?.saturating_sub(idle_y);
    }
    // Exact pro-rata of tight SOL + idle SOL; whatever the bps removal fell
    // short by comes from idle, as far as idle goes.
    let pool_sol = tight.amount_y.checked_add(idle_y).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let expected = math::component_for_shares(net, pool_sol, h, Rounding::Down)?;
    let from_idle = expected.saturating_sub(y_out).min(idle_y);
    let lamports_out = y_out + from_idle;
    let idle_x_share = math::component_for_shares(net, idle_x, h, Rounding::Down)?;
    let treasury_shares_burned = x_out + idle_x_share;

    transfer_from_basket(dl.basket, dl.basket_wsol_ata, dest_wsol_ata, dl.token_program, signer, lamports_out)?;
    burn_basket_shares(dl.basket, dl.basket_share_ata, dl.share_mint, dl.token_program, signer, treasury_shares_burned)?;
    Ok(SleeveOut { treasury_shares_burned, lamports_out })
}

pub struct PaidLegs {
    pub paid: u16,
    pub claims: u16,
}

/// Pay `net / h` of each component to the redeemer, or into a FrozenClaim
/// when the leg is frozen. `claims` are the trailing remaining accounts.
#[allow(clippy::too_many_arguments)]
pub fn pay_components<'info>(
    components: &[ComponentAccounts<'info>],
    claims: &[AccountInfo<'info>],
    net: u64,
    h: u64,
    signer: &BasketSigner,
    basket: &AccountInfo<'info>,
    basket_key: &Pubkey,
    redeemer: &Redeemer<'_, 'info>,
    programs: &Programs<'_, 'info>,
    now: i64,
) -> Result<PaidLegs> {
    let mut out = PaidLegs { paid: 0, claims: 0 };
    let basket_seeds = signer.basket_seeds();
    let holder = redeemer.ai;
    for c in components {
        let amount = math::component_for_shares(net, c.available, h, Rounding::Down)?;
        out.paid += 1;
        if amount == 0 {
            continue;
        }
        let tp = if c.position.token_program == token::ID { programs.token } else { programs.token_2022 };
        if c.vault_frozen || c.user_frozen {
            if !redeemer.record_frozen {
                continue;
            }
            let (claim_key, bump) = frozen_claim_address(&signer.share_mint, holder.key, &c.position.mint);
            let claim = claims
                .iter()
                .find(|ai| ai.key() == claim_key)
                .ok_or_else(|| error!(BasketError::ComponentFrozen))?;
            record_claim(claim, bump, &signer.share_mint, basket_key, holder, &c.position.mint, amount, now, programs.system)?;
            let mut position: Position = (*c.position).clone();
            position.owed = position.owed.checked_add(amount).ok_or_else(|| error!(BasketError::MathOverflow))?;
            let mut data = c.position_ai.try_borrow_mut_data()?;
            position.try_serialize(&mut &mut data[..])?;
            out.claims += 1;
            emit!(FrozenClaimCreated { basket: *basket_key, wallet: holder.key(), mint: c.position.mint, amount });
            continue;
        }
        ensure_ata(redeemer.payer, c.user_ata, holder, c.mint, programs.system, tp, programs.ata)?;
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                tp.key(),
                token_interface::TransferChecked {
                    from: c.vault.clone(),
                    mint: c.mint.clone(),
                    to: c.user_ata.clone(),
                    authority: basket.clone(),
                },
                &[&basket_seeds],
            ),
            amount,
            c.position.decimals,
        )?;
    }
    Ok(out)
}

pub fn frozen_claim_address(share_mint: &Pubkey, wallet: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[seeds::CLAIM, share_mint.as_ref(), wallet.as_ref(), mint.as_ref()], &crate::ID)
}

/// Create the claim PDA (rent from the holder) or top up an existing one.
#[allow(clippy::too_many_arguments)]
fn record_claim<'info>(
    claim: &AccountInfo<'info>,
    bump: u8,
    share_mint: &Pubkey,
    basket_key: &Pubkey,
    holder: &AccountInfo<'info>,
    mint: &Pubkey,
    amount: u64,
    now: i64,
    system_program: &AccountInfo<'info>,
) -> Result<()> {
    let mut state = if claim.data_is_empty() {
        let space = 8 + FrozenClaim::INIT_SPACE;
        let lamports = Rent::get()?.minimum_balance(space);
        let seeds: &[&[u8]] = &[seeds::CLAIM, share_mint.as_ref(), holder.key.as_ref(), mint.as_ref(), &[bump]];
        system_program::create_account(
            CpiContext::new_with_signer(
                system_program.key(),
                system_program::CreateAccount { from: holder.clone(), to: claim.clone() },
                &[seeds],
            ),
            lamports,
            space as u64,
            &crate::ID,
        )?;
        FrozenClaim { bump, basket: *basket_key, wallet: holder.key(), mint: *mint, amount: 0, created_at: now }
    } else {
        require_keys_eq!(*claim.owner, crate::ID, BasketError::ComponentMismatch);
        let data = claim.try_borrow_data()?;
        let existing = FrozenClaim::try_deserialize(&mut &data[..])?;
        require!(existing.basket == *basket_key && existing.wallet == holder.key(), BasketError::ComponentMismatch);
        existing
    };
    state.amount = state.amount.checked_add(amount).ok_or_else(|| error!(BasketError::MathOverflow))?;
    let mut data = claim.try_borrow_mut_data()?;
    state.try_serialize(&mut &mut data[..])?;
    Ok(())
}

/// Send the SOL leg to the holder: through their wSOL ATA, unwrapped when
/// this instruction created it.
#[allow(clippy::too_many_arguments)]
fn holder_sol_leg<'info>(
    view: &PoolView<'_, 'info>,
    net: u64,
    h: u64,
    signer: &BasketSigner,
    dl: &DlmmAccounts<'_, 'info>,
    tight_position: &AccountInfo<'info>,
    holder: &AccountInfo<'info>,
    holder_wsol_ata: &AccountInfo<'info>,
    programs: &Programs<'_, 'info>,
) -> Result<SleeveOut> {
    let created = holder_wsol_ata.data_is_empty();
    ensure_ata(holder, holder_wsol_ata, holder, dl.wsol_mint, programs.system, programs.token, programs.ata)?;
    let out = remove_sleeve(view, net, h, signer, dl, tight_position, holder_wsol_ata)?;
    if created {
        token::close_account(CpiContext::new(
            programs.token.key(),
            token::CloseAccount { account: holder_wsol_ata.clone(), destination: holder.clone(), authority: holder.clone() },
        ))?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub fn handle_redeem<'info>(ctx: Context<'info, Redeem<'info>>, shares: u64) -> Result<()> {
    require!(shares > 0, BasketError::ZeroAmount);
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    require!(ctx.accounts.basket.seeded, BasketError::NotSeeded);

    let n = ctx.accounts.basket.position_count as usize;
    let components = load_components(&basket_key, ctx.remaining_accounts, n)?;
    let tail = ComponentAccounts::trailing(ctx.remaining_accounts, n);

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

    let holder = ctx.accounts.holder.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let redeemer = Redeemer { ai: &holder, seeds: None, payer: &holder, record_frozen: true };
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    ensure_ata(&holder, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, programs.system, programs.token, programs.ata)?;
    let (net, fee) = take_shares(
        &redeemer,
        &ctx.accounts.holder_share_ata,
        &ctx.accounts.fee_vault_share_ata,
        &share_mint,
        programs.token,
        shares,
        basket.fees.redeem_fee_bps,
    )?;
    // Position amounts are only exact to a few base units of their bin shares,
    // so `h` can land just under the last holder's balance: never pay more
    // than everything.
    let h = h.max(net);

    let signer = BasketSigner::new(basket);
    let basket_ai = ctx.accounts.basket.to_account_info();
    let dl = DlmmAccounts {
        program: &ctx.accounts.dlmm_program.to_account_info(),
        lb_pair: &ctx.accounts.lb_pair.to_account_info(),
        reserve_x: &ctx.accounts.reserve_x.to_account_info(),
        reserve_y: &ctx.accounts.reserve_y.to_account_info(),
        share_mint: &share_mint,
        wsol_mint: &ctx.accounts.wsol_mint.to_account_info(),
        basket_share_ata: &ctx.accounts.basket_share_ata.to_account_info(),
        basket_wsol_ata: &ctx.accounts.basket_wsol_ata.to_account_info(),
        basket: &basket_ai,
        token_program: programs.token,
        memo_program: &ctx.accounts.memo_program.to_account_info(),
        event_authority: &ctx.accounts.dlmm_event_authority.to_account_info(),
        system_program: programs.system,
    };
    dl.check(basket)?;
    let sleeve = holder_sol_leg(
        &view,
        net,
        h,
        &signer,
        &dl,
        &ctx.accounts.tight_position.to_account_info(),
        &holder,
        &ctx.accounts.holder_wsol_ata.to_account_info(),
        &programs,
    )?;

    let legs = pay_components(&components, tail, net, h, &signer, &basket_ai, &basket_key, &redeemer, &programs, now)?;

    ctx.accounts.basket.touch(now);
    emit!(Redeemed {
        basket: basket_key,
        holder: holder.key(),
        shares_burned: net,
        fee_shares: fee,
        treasury_shares_burned: sleeve.treasury_shares_burned,
        lamports_out: sleeve.lamports_out,
        components_paid: legs.paid,
        claims_created: legs.claims,
        complete: true,
    });
    Ok(())
}

pub fn handle_redeem_begin<'info>(ctx: Context<'info, RedeemBegin<'info>>, shares: u64) -> Result<()> {
    require!(shares > 0, BasketError::ZeroAmount);
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    require!(ctx.accounts.basket.seeded, BasketError::NotSeeded);

    let basket = &ctx.accounts.basket;
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

    let holder = ctx.accounts.holder.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let redeemer = Redeemer { ai: &holder, seeds: None, payer: &holder, record_frozen: true };
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    ensure_ata(&holder, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, programs.system, programs.token, programs.ata)?;
    let (net, fee) = take_shares(
        &redeemer,
        &ctx.accounts.holder_share_ata,
        &ctx.accounts.fee_vault_share_ata,
        &share_mint,
        programs.token,
        shares,
        basket.fees.redeem_fee_bps,
    )?;
    // Position amounts are only exact to a few base units of their bin shares,
    // so `h` can land just under the last holder's balance: never pay more
    // than everything.
    let h = h.max(net);

    let signer = BasketSigner::new(basket);
    let basket_ai = ctx.accounts.basket.to_account_info();
    let dl = DlmmAccounts {
        program: &ctx.accounts.dlmm_program.to_account_info(),
        lb_pair: &ctx.accounts.lb_pair.to_account_info(),
        reserve_x: &ctx.accounts.reserve_x.to_account_info(),
        reserve_y: &ctx.accounts.reserve_y.to_account_info(),
        share_mint: &share_mint,
        wsol_mint: &ctx.accounts.wsol_mint.to_account_info(),
        basket_share_ata: &ctx.accounts.basket_share_ata.to_account_info(),
        basket_wsol_ata: &ctx.accounts.basket_wsol_ata.to_account_info(),
        basket: &basket_ai,
        token_program: programs.token,
        memo_program: &ctx.accounts.memo_program.to_account_info(),
        event_authority: &ctx.accounts.dlmm_event_authority.to_account_info(),
        system_program: programs.system,
    };
    dl.check(basket)?;
    let sleeve = holder_sol_leg(
        &view,
        net,
        h,
        &signer,
        &dl,
        &ctx.accounts.tight_position.to_account_info(),
        &holder,
        &ctx.accounts.holder_wsol_ata.to_account_info(),
        &programs,
    )?;

    // Components are paid later; keep the burned shares in the denominator.
    let basket = &mut ctx.accounts.basket;
    basket.pending_redeem_shares =
        basket.pending_redeem_shares.checked_add(net).ok_or_else(|| error!(BasketError::MathOverflow))?;
    basket.touch(now);
    let red = &mut ctx.accounts.redemption;
    red.bump = ctx.bumps.redemption;
    red.basket = basket_key;
    red.holder = holder.key();
    red.shares = net;
    red.position_count = basket.position_count;
    red.paid_count = 0;
    red.paid_mints = Vec::new();
    red.created_at = now;

    emit!(Redeemed {
        basket: basket_key,
        holder: holder.key(),
        shares_burned: net,
        fee_shares: fee,
        treasury_shares_burned: sleeve.treasury_shares_burned,
        lamports_out: sleeve.lamports_out,
        components_paid: 0,
        claims_created: 0,
        complete: false,
    });
    Ok(())
}

pub fn handle_redeem_components<'info>(ctx: Context<'info, RedeemComponents<'info>>, count: u16) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    let n = count as usize;
    let components = load_components(&basket_key, ctx.remaining_accounts, n)?;
    let tail = ComponentAccounts::trailing(ctx.remaining_accounts, n);

    // Not yet paid in this redemption.
    for c in &components {
        require!(
            !ctx.accounts.redemption.paid_mints.contains(&c.position.mint),
            BasketError::DuplicateMint
        );
    }

    // Same denominator as at `redeem_begin`: the burned shares are still
    // counted through `pending_redeem_shares`.
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

    let holder = ctx.accounts.holder.to_account_info();
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let redeemer = Redeemer { ai: &holder, seeds: None, payer: &holder, record_frozen: true };
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
        basket.touch(now);
        ctx.accounts.redemption.close(holder.clone())?;
    }
    emit!(RedemptionComponentsPaid { basket: basket_key, holder: holder.key(), count: legs.paid, complete });
    Ok(())
}

pub fn handle_claim_frozen(ctx: Context<ClaimFrozen>) -> Result<()> {
    let amount = ctx.accounts.claim.amount;
    require!(amount > 0, BasketError::ZeroAmount);
    let wallet = ctx.accounts.wallet.to_account_info();
    let tp = if ctx.accounts.position.token_program == token::ID {
        ctx.accounts.token_program.to_account_info()
    } else {
        ctx.accounts.token_2022_program.to_account_info()
    };
    ensure_ata(
        &wallet,
        &ctx.accounts.wallet_ata,
        &wallet,
        &ctx.accounts.mint,
        &ctx.accounts.system_program.to_account_info(),
        &tp,
        &ctx.accounts.associated_token_program.to_account_info(),
    )?;
    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_seeds = signer.basket_seeds();
    token_interface::transfer_checked(
        CpiContext::new_with_signer(
            tp.key(),
            token_interface::TransferChecked {
                from: ctx.accounts.vault.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
                to: ctx.accounts.wallet_ata.to_account_info(),
                authority: ctx.accounts.basket.to_account_info(),
            },
            &[&basket_seeds],
        ),
        amount,
        ctx.accounts.position.decimals,
    )?;
    let position = &mut ctx.accounts.position;
    position.owed = position.owed.saturating_sub(amount);
    emit!(FrozenClaimPaid {
        basket: ctx.accounts.basket.key(),
        wallet: wallet.key(),
        mint: ctx.accounts.mint.key(),
        amount,
    });
    Ok(())
}
