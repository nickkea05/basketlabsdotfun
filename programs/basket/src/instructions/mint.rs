//! `mint` — later buys (D4, D5, D6, §4, §5).
//!
//! `X = min_i(deposit_i · H / vault_i)` over the vaults before the deposit
//! (creation-unit rule; H = holder shares outstanding). The sleeve then adds
//! `Y = X·r/(1−r)` treasury shares to the basket's pool position at the
//! pool's current price, which fixes the SOL leg the buyer must supply. `r`
//! follows the step rule. The mint fee is withheld as shares into the
//! FeeVault. Excess deposit above the min ratio stays in the vaults.
//!
//! Remaining accounts: `[position, vault, buyer_ata, mint]` per position, in
//! book order.

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Mint, Token};
use anchor_spl::token_2022::Token2022;

use crate::constants::*;
use crate::damm::{self, cp_amm};
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::{load_components, COMPONENT_GROUP};
use crate::instructions::sleeve::*;
use crate::math;
use crate::state::*;

#[derive(Accounts)]
pub struct MintShares<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
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
    /// CHECK: mint authority PDA.
    #[account(seeds = [seeds::SHARE_AUTH, share_mint.key().as_ref()], bump = basket.share_auth_bump)]
    pub share_auth: UncheckedAccount<'info>,
    /// CHECK: ATA(payer, share_mint), created if missing.
    #[account(mut)]
    pub payer_share_ata: UncheckedAccount<'info>,
    #[account(seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint).
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,

    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint).
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL).
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,
    /// CHECK: cp-amm position NFT account, owned by the basket.
    #[account(address = damm::position_nft_account(&basket.position_nft_mint))]
    pub position_nft_account: UncheckedAccount<'info>,
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

pub fn handle_mint<'info>(ctx: Context<'info, MintShares<'info>>, args: MintArgs) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    {
        let basket = &ctx.accounts.basket;
        require!(basket.seeded, BasketError::NotSeeded);
        require!(basket.gate_open_at(now), BasketError::MintGateClosed);
    }

    // 1. Shares from the vaults *before* the deposit (creation-unit rule).
    let n = ctx.accounts.basket.position_count as usize;
    require!(
        args.deposits.len() == n && ctx.remaining_accounts.len() == n * COMPONENT_GROUP,
        BasketError::ComponentCountMismatch
    );
    let components = load_components(&basket_key, ctx.remaining_accounts, n)?;
    let (holder_shares_before, r_bps, sqrt_min, sqrt_price, sqrt_max) = {
        let pool = ctx.accounts.pool.load()?;
        let position = ctx.accounts.pool_position.load()?;
        require_keys_eq!(position.pool, ctx.accounts.pool.key(), BasketError::PoolMismatch);
        let basket = &ctx.accounts.basket;
        let h = holder_shares(ctx.accounts.share_mint.supply, &pool, &position, basket.pending_redeem_shares)?;
        let r = effective_r_bps(&basket.sleeve, pool.token_b_amount);
        (h, r, pool.sqrt_min_price, pool.sqrt_price, pool.sqrt_max_price)
    };
    let vaults: Vec<u64> = components.iter().map(|c| c.available).collect();
    let gross_shares = math::shares_for_deposits(&args.deposits, &vaults, holder_shares_before)?;
    require!(gross_shares > 0, BasketError::ZeroAmount);
    let (to_buyer, fee_shares) = split_mint_fee(gross_shares, ctx.accounts.basket.fees.mint_fee_bps)?;
    require!(to_buyer >= args.min_shares_out, BasketError::SlippageExceeded);

    // 2. Sleeve at the pool's ratio: the SOL leg is the nominal treasury
    //    shares valued at the pool's marginal price (= r/(1−r) of the
    //    components' value when the pool sits at NAV); the share side follows
    //    from the range geometry.
    let nominal_treasury_shares = math::treasury_shares(gross_shares, r_bps)?;
    let target_b = math::lamports_for_shares(nominal_treasury_shares, sqrt_price)?;
    let quote = math::sleeve_quote(target_b, nominal_treasury_shares, sqrt_min, sqrt_price, sqrt_max)?;
    require!(quote.amount_b <= args.max_sleeve_lamports, BasketError::SlippageExceeded);
    if quote.amount_b > 0 {
        let implied_total = math::implied_total_lamports(quote.amount_b, r_bps)?;
        require!(implied_total >= config.fees.min_mint_lamports, BasketError::BelowMinimum);
    }

    // 3. Components in.
    let payer_ai = ctx.accounts.payer.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let token_2022_program = ctx.accounts.token_2022_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    transfer_components_in(&components, &args.deposits, &payer_ai, &token_program, &token_2022_program)?;

    // 4. Sleeve in, staged through the basket's own ATAs (address-checked).
    let basket_ai = ctx.accounts.basket.to_account_info();
    let share_mint_ai = ctx.accounts.share_mint.to_account_info();
    let share_auth_ai = ctx.accounts.share_auth.to_account_info();
    let signer = BasketSigner::new(&ctx.accounts.basket);
    if quote.liquidity > 0 {
        for (ata, mint) in [
            (&ctx.accounts.basket_share_ata, &share_mint_ai),
            (&ctx.accounts.basket_wsol_ata, &ctx.accounts.wsol_mint.to_account_info()),
        ] {
            ensure_ata(&payer_ai, &ata.to_account_info(), &basket_ai, mint, &system_program, &token_program, &ata_program)?;
        }
        mint_shares(
            &share_mint_ai,
            &ctx.accounts.basket_share_ata.to_account_info(),
            &share_auth_ai,
            &token_program,
            &signer,
            quote.amount_a,
        )?;
        wrap_sol(
            &payer_ai,
            &ctx.accounts.basket_wsol_ata.to_account_info(),
            &system_program,
            &token_program,
            quote.amount_b,
        )?;
        let basket_seeds = signer.basket_seeds();
        cp_amm::cpi::add_liquidity(
            CpiContext::new_with_signer(
                ctx.accounts.cp_amm_program.key(),
                cp_amm::cpi::accounts::AddLiquidity {
                    pool: ctx.accounts.pool.to_account_info(),
                    position: ctx.accounts.pool_position.to_account_info(),
                    token_a_account: ctx.accounts.basket_share_ata.to_account_info(),
                    token_b_account: ctx.accounts.basket_wsol_ata.to_account_info(),
                    token_a_vault: ctx.accounts.token_a_vault.to_account_info(),
                    token_b_vault: ctx.accounts.token_b_vault.to_account_info(),
                    token_a_mint: share_mint_ai.clone(),
                    token_b_mint: ctx.accounts.wsol_mint.to_account_info(),
                    position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
                    signer: basket_ai.clone(),
                    token_a_program: token_program.clone(),
                    token_b_program: token_program.clone(),
                    event_authority: ctx.accounts.event_authority.to_account_info(),
                    program: ctx.accounts.cp_amm_program.to_account_info(),
                },
                &[&basket_seeds],
            ),
            cp_amm::types::AddLiquidityParameters {
                liquidity_delta: quote.liquidity,
                token_a_amount_threshold: quote.amount_a,
                token_b_amount_threshold: quote.amount_b,
            },
        )?;
        burn_basket_share_dust(
            &basket_ai,
            &ctx.accounts.basket_share_ata.to_account_info(),
            &share_mint_ai,
            &token_program,
            &signer,
        )?;
    }

    // 5. Buyer and fee shares (fee ATA address verified; buyer ATA created if missing).
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    for (ata, authority) in [
        (&ctx.accounts.payer_share_ata, &payer_ai),
        (&ctx.accounts.fee_vault_share_ata, &fee_vault_ai),
    ] {
        ensure_ata(&payer_ai, &ata.to_account_info(), authority, &share_mint_ai, &system_program, &token_program, &ata_program)?;
    }
    mint_shares(&share_mint_ai, &ctx.accounts.payer_share_ata.to_account_info(), &share_auth_ai, &token_program, &signer, to_buyer)?;
    mint_shares(&share_mint_ai, &ctx.accounts.fee_vault_share_ata.to_account_info(), &share_auth_ai, &token_program, &signer, fee_shares)?;

    ctx.accounts.basket.touch(now);
    emit!(Minted {
        basket: basket_key,
        buyer: ctx.accounts.payer.key(),
        shares_to_buyer: to_buyer,
        fee_shares,
        treasury_shares: quote.amount_a,
        sleeve_lamports: quote.amount_b,
        r_bps,
    });
    Ok(())
}
