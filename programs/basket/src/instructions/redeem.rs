//! `redeem`, `redeem_begin` / `redeem_components`, `claim_frozen` (D9, D10,
//! §4, §5).
//!
//! A redemption of `shares` (gross) withholds the redeem fee as shares into
//! the FeeVault, burns the net amount, and pays `net / H` of everything the
//! holders own: each component vault (less amounts already owed to frozen
//! claimants) and the basket's pool position (SOL to the holder, the
//! withdrawn treasury shares burned). `H` is the holder-share count before
//! the burn. Redeem has no gate and ignores the pause flag.
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
//! in book order, then any FrozenClaim PDAs.

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token};
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface;

use crate::constants::*;
use crate::damm::{self, cp_amm};
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::{load_components, read_token_amount, ComponentAccounts, COMPONENT_GROUP};
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
        has_one = pool @ BasketError::PoolMismatch,
        has_one = pool_position @ BasketError::PoolMismatch,
    )]
    pub basket: Account<'info, Basket>,
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
    /// CHECK: cp-amm position NFT account owned by the basket.
    #[account(address = damm::position_nft_account(&basket.position_nft_mint))]
    pub position_nft_account: UncheckedAccount<'info>,
    /// CHECK: cp-amm pool authority.
    #[account(address = damm::pool_authority())]
    pub pool_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub pool: AccountLoader<'info, cp_amm::accounts::Pool>,
    #[account(mut)]
    pub pool_position: AccountLoader<'info, cp_amm::accounts::Position>,
    /// CHECK: cp-amm vault for the share mint.
    #[account(mut, address = damm::token_vault(&share_mint.key(), &pool.key()))]
    pub token_a_vault: UncheckedAccount<'info>,
    /// CHECK: cp-amm vault for wSOL.
    #[account(mut, address = damm::token_vault(&wsol_mint.key(), &pool.key()))]
    pub token_b_vault: UncheckedAccount<'info>,
    pub cp_amm_program: Program<'info, cp_amm::program::CpAmm>,
    /// CHECK: cp-amm event authority PDA.
    #[account(address = damm::event_authority())]
    pub event_authority: UncheckedAccount<'info>,

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
        has_one = pool @ BasketError::PoolMismatch,
        has_one = pool_position @ BasketError::PoolMismatch,
    )]
    pub basket: Account<'info, Basket>,
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
    /// CHECK: cp-amm position NFT account owned by the basket.
    #[account(address = damm::position_nft_account(&basket.position_nft_mint))]
    pub position_nft_account: UncheckedAccount<'info>,
    /// CHECK: cp-amm pool authority.
    #[account(address = damm::pool_authority())]
    pub pool_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub pool: AccountLoader<'info, cp_amm::accounts::Pool>,
    #[account(mut)]
    pub pool_position: AccountLoader<'info, cp_amm::accounts::Position>,
    /// CHECK: cp-amm vault for the share mint.
    #[account(mut, address = damm::token_vault(&share_mint.key(), &pool.key()))]
    pub token_a_vault: UncheckedAccount<'info>,
    /// CHECK: cp-amm vault for wSOL.
    #[account(mut, address = damm::token_vault(&wsol_mint.key(), &pool.key()))]
    pub token_b_vault: UncheckedAccount<'info>,
    pub cp_amm_program: Program<'info, cp_amm::program::CpAmm>,
    /// CHECK: cp-amm event authority PDA.
    #[account(address = damm::event_authority())]
    pub event_authority: UncheckedAccount<'info>,

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
        has_one = pool @ BasketError::PoolMismatch,
        has_one = pool_position @ BasketError::PoolMismatch,
    )]
    pub basket: Account<'info, Basket>,
    pub share_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [seeds::REDEMPTION, share_mint.key().as_ref(), holder.key().as_ref()],
        bump = redemption.bump,
        has_one = holder,
    )]
    pub redemption: Account<'info, Redemption>,
    pub pool: AccountLoader<'info, cp_amm::accounts::Pool>,
    pub pool_position: AccountLoader<'info, cp_amm::accounts::Position>,
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
    pub basket: Account<'info, Basket>,
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
// Shared pieces
// ---------------------------------------------------------------------------

struct Programs<'a, 'info> {
    token: &'a AccountInfo<'info>,
    token_2022: &'a AccountInfo<'info>,
    ata: &'a AccountInfo<'info>,
    system: &'a AccountInfo<'info>,
}

/// Fee to the FeeVault (as shares), net burned. Returns `(net, fee)`.
fn take_shares<'info>(
    holder: &AccountInfo<'info>,
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
    if fee > 0 {
        token::transfer(
            CpiContext::new(
                token_program.key(),
                token::Transfer { from: holder_share_ata.clone(), to: fee_vault_share_ata.clone(), authority: holder.clone() },
            ),
            fee,
        )?;
    }
    token::burn(
        CpiContext::new(
            token_program.key(),
            token::Burn { mint: share_mint.clone(), from: holder_share_ata.clone(), authority: holder.clone() },
        ),
        net,
    )?;
    Ok((net, fee))
}

struct SleeveOut {
    treasury_shares_burned: u64,
    lamports_out: u64,
}

/// Remove `net / h` of the basket's position: SOL straight to the holder's
/// wSOL account (unwrapped if we created it), shares to the basket and burned.
#[allow(clippy::too_many_arguments)]
fn remove_sleeve<'info>(
    liquidity: u128,
    net: u64,
    h: u64,
    signer: &BasketSigner,
    basket: &AccountInfo<'info>,
    holder: &AccountInfo<'info>,
    share_mint: &AccountInfo<'info>,
    wsol_mint: &AccountInfo<'info>,
    basket_share_ata: &AccountInfo<'info>,
    holder_wsol_ata: &AccountInfo<'info>,
    cpi: cp_amm::cpi::accounts::RemoveLiquidity<'info>,
    cp_amm_program: &AccountInfo<'info>,
    programs: &Programs<'_, 'info>,
) -> Result<SleeveOut> {
    let delta = math::liquidity_share(liquidity, net, h)?;
    if delta == 0 {
        return Ok(SleeveOut { treasury_shares_burned: 0, lamports_out: 0 });
    }
    ensure_ata(holder, basket_share_ata, basket, share_mint, programs.system, programs.token, programs.ata)?;
    let created_wsol = holder_wsol_ata.data_is_empty();
    ensure_ata(holder, holder_wsol_ata, holder, wsol_mint, programs.system, programs.token, programs.ata)?;
    let wsol_before = read_token_amount(holder_wsol_ata)?;

    let basket_seeds = signer.basket_seeds();
    cp_amm::cpi::remove_liquidity(
        CpiContext::new_with_signer(cp_amm_program.key(), cpi, &[&basket_seeds]),
        cp_amm::types::RemoveLiquidityParameters {
            liquidity_delta: delta,
            token_a_amount_threshold: 0,
            token_b_amount_threshold: 0,
        },
    )?;

    let treasury_shares_burned =
        burn_basket_share_dust(basket, basket_share_ata, share_mint, programs.token, signer)?;
    let lamports_out = read_token_amount(holder_wsol_ata)? - wsol_before;
    if created_wsol {
        token::close_account(CpiContext::new(
            programs.token.key(),
            token::CloseAccount { account: holder_wsol_ata.clone(), destination: holder.clone(), authority: holder.clone() },
        ))?;
    }
    Ok(SleeveOut { treasury_shares_burned, lamports_out })
}

struct PaidLegs {
    paid: u16,
    claims: u16,
}

/// Pay `net / h` of each component to the holder, or into a FrozenClaim
/// when the leg is frozen. `claims` are the trailing remaining accounts.
#[allow(clippy::too_many_arguments)]
fn pay_components<'info>(
    components: &[ComponentAccounts<'info>],
    claims: &[AccountInfo<'info>],
    net: u64,
    h: u64,
    signer: &BasketSigner,
    basket: &AccountInfo<'info>,
    basket_key: &Pubkey,
    holder: &AccountInfo<'info>,
    programs: &Programs<'_, 'info>,
    now: i64,
) -> Result<PaidLegs> {
    let mut out = PaidLegs { paid: 0, claims: 0 };
    let basket_seeds = signer.basket_seeds();
    for c in components {
        let amount = math::component_for_shares(net, c.available, h, Rounding::Down)?;
        out.paid += 1;
        if amount == 0 {
            continue;
        }
        let tp = if c.position.token_program == token::ID { programs.token } else { programs.token_2022 };
        if c.vault_frozen || c.user_frozen {
            let claim = claims
                .iter()
                .find(|ai| ai.key() == frozen_claim_address(&signer.share_mint, holder.key, &c.position.mint).0)
                .ok_or_else(|| error!(BasketError::ComponentFrozen))?;
            record_claim(claim, &signer.share_mint, basket_key, holder, &c.position.mint, amount, now, programs.system)?;
            let mut position = c.position.clone();
            position.owed = position.owed.checked_add(amount).ok_or_else(|| error!(BasketError::MathOverflow))?;
            let mut data = c.position_ai.try_borrow_mut_data()?;
            position.try_serialize(&mut &mut data[..])?;
            out.claims += 1;
            emit!(FrozenClaimCreated { basket: *basket_key, wallet: holder.key(), mint: c.position.mint, amount });
            continue;
        }
        ensure_ata(holder, c.user_ata, holder, c.mint, programs.system, tp, programs.ata)?;
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
    share_mint: &Pubkey,
    basket_key: &Pubkey,
    holder: &AccountInfo<'info>,
    mint: &Pubkey,
    amount: u64,
    now: i64,
    system_program: &AccountInfo<'info>,
) -> Result<()> {
    let (_, bump) = frozen_claim_address(share_mint, holder.key, mint);
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
    let claims = ComponentAccounts::trailing(ctx.remaining_accounts, n);

    let (h, liquidity) = {
        let pool = ctx.accounts.pool.load()?;
        let position = ctx.accounts.pool_position.load()?;
        require_keys_eq!(position.pool, ctx.accounts.pool.key(), BasketError::PoolMismatch);
        let h = holder_shares(ctx.accounts.share_mint.supply, &pool, &position, ctx.accounts.basket.pending_redeem_shares)?;
        (h, position.unlocked_liquidity)
    };

    let holder = ctx.accounts.holder.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    ensure_ata(&holder, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, programs.system, programs.token, programs.ata)?;
    let (net, fee) = take_shares(
        &holder,
        &ctx.accounts.holder_share_ata,
        &ctx.accounts.fee_vault_share_ata,
        &share_mint,
        programs.token,
        shares,
        ctx.accounts.basket.fees.redeem_fee_bps,
    )?;

    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_ai = ctx.accounts.basket.to_account_info();
    let sleeve = remove_sleeve(
        liquidity,
        net,
        h,
        &signer,
        &basket_ai,
        &holder,
        &share_mint,
        &ctx.accounts.wsol_mint.to_account_info(),
        &ctx.accounts.basket_share_ata,
        &ctx.accounts.holder_wsol_ata,
        cp_amm::cpi::accounts::RemoveLiquidity {
            pool_authority: ctx.accounts.pool_authority.to_account_info(),
            pool: ctx.accounts.pool.to_account_info(),
            position: ctx.accounts.pool_position.to_account_info(),
            token_a_account: ctx.accounts.basket_share_ata.to_account_info(),
            token_b_account: ctx.accounts.holder_wsol_ata.to_account_info(),
            token_a_vault: ctx.accounts.token_a_vault.to_account_info(),
            token_b_vault: ctx.accounts.token_b_vault.to_account_info(),
            token_a_mint: share_mint.clone(),
            token_b_mint: ctx.accounts.wsol_mint.to_account_info(),
            position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
            signer: basket_ai.clone(),
            token_a_program: programs.token.clone(),
            token_b_program: programs.token.clone(),
            event_authority: ctx.accounts.event_authority.to_account_info(),
            program: ctx.accounts.cp_amm_program.to_account_info(),
        },
        &ctx.accounts.cp_amm_program.to_account_info(),
        &programs,
    )?;

    let legs = pay_components(&components, claims, net, h, &signer, &basket_ai, &basket_key, &holder, &programs, now)?;

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

    let (h, liquidity) = {
        let pool = ctx.accounts.pool.load()?;
        let position = ctx.accounts.pool_position.load()?;
        require_keys_eq!(position.pool, ctx.accounts.pool.key(), BasketError::PoolMismatch);
        let h = holder_shares(ctx.accounts.share_mint.supply, &pool, &position, ctx.accounts.basket.pending_redeem_shares)?;
        (h, position.unlocked_liquidity)
    };

    let holder = ctx.accounts.holder.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    ensure_ata(&holder, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, programs.system, programs.token, programs.ata)?;
    let (net, fee) = take_shares(
        &holder,
        &ctx.accounts.holder_share_ata,
        &ctx.accounts.fee_vault_share_ata,
        &share_mint,
        programs.token,
        shares,
        ctx.accounts.basket.fees.redeem_fee_bps,
    )?;

    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_ai = ctx.accounts.basket.to_account_info();
    let sleeve = remove_sleeve(
        liquidity,
        net,
        h,
        &signer,
        &basket_ai,
        &holder,
        &share_mint,
        &ctx.accounts.wsol_mint.to_account_info(),
        &ctx.accounts.basket_share_ata,
        &ctx.accounts.holder_wsol_ata,
        cp_amm::cpi::accounts::RemoveLiquidity {
            pool_authority: ctx.accounts.pool_authority.to_account_info(),
            pool: ctx.accounts.pool.to_account_info(),
            position: ctx.accounts.pool_position.to_account_info(),
            token_a_account: ctx.accounts.basket_share_ata.to_account_info(),
            token_b_account: ctx.accounts.holder_wsol_ata.to_account_info(),
            token_a_vault: ctx.accounts.token_a_vault.to_account_info(),
            token_b_vault: ctx.accounts.token_b_vault.to_account_info(),
            token_a_mint: share_mint.clone(),
            token_b_mint: ctx.accounts.wsol_mint.to_account_info(),
            position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
            signer: basket_ai.clone(),
            token_a_program: programs.token.clone(),
            token_b_program: programs.token.clone(),
            event_authority: ctx.accounts.event_authority.to_account_info(),
            program: ctx.accounts.cp_amm_program.to_account_info(),
        },
        &ctx.accounts.cp_amm_program.to_account_info(),
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
    let claims = ComponentAccounts::trailing(ctx.remaining_accounts, n);

    // Not yet paid in this redemption.
    for c in &components {
        require!(
            !ctx.accounts.redemption.paid_mints.contains(&c.position.mint),
            BasketError::DuplicateMint
        );
    }

    // Same denominator as at `redeem_begin`: the burned shares are still
    // counted through `pending_redeem_shares`.
    let h = {
        let pool = ctx.accounts.pool.load()?;
        let position = ctx.accounts.pool_position.load()?;
        holder_shares(ctx.accounts.share_mint.supply, &pool, &position, ctx.accounts.basket.pending_redeem_shares)?
    };
    let net = ctx.accounts.redemption.shares;

    let holder = ctx.accounts.holder.to_account_info();
    let programs = Programs {
        token: &ctx.accounts.token_program.to_account_info(),
        token_2022: &ctx.accounts.token_2022_program.to_account_info(),
        ata: &ctx.accounts.associated_token_program.to_account_info(),
        system: &ctx.accounts.system_program.to_account_info(),
    };
    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_ai = ctx.accounts.basket.to_account_info();
    let legs = pay_components(&components, claims, net, h, &signer, &basket_ai, &basket_key, &holder, &programs, now)?;

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
