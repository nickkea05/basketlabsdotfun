//! `seed` — the first buy (D4, D5, D8, §4, §5).
//!
//! Deposits every component in kind, mints `X = initial_shares` (less the
//! mint fee, which is withheld as shares into the FeeVault) to the buyer and
//! `Y = X·r/(1−r)` treasury shares, opens the DAMM v2 pool at
//! `price = sleeve_lamports / Y` with the profile's range and fee config, and
//! parks the position NFT with the basket. The 0.1 SOL creation fee goes to
//! the treasury.
//!
//! cp-amm's `payer` funds account rent through the system program, which
//! refuses to debit an account that carries data, so the buyer (not the
//! basket PDA) is cp-amm's payer: treasury shares and wrapped SOL are staged
//! in the buyer's own ATAs, the pool pulls them, and the leftovers are
//! burned / unwrapped in the same instruction. The basket is cp-amm's
//! `creator`, which makes it the position NFT owner.
//!
//! Remaining accounts: `[position, vault, buyer_ata, mint]` per position, in
//! book order.

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token};
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
pub struct Seed<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
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
    /// CHECK: ATA(payer, wSOL); staging for the SOL leg, closed afterwards
    /// if this instruction created it.
    #[account(mut)]
    pub payer_wsol_ata: UncheckedAccount<'info>,

    #[account(
        init,
        payer = payer,
        space = 8 + FeeVault::INIT_SPACE,
        seeds = [seeds::FEES, share_mint.key().as_ref()],
        bump,
    )]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint), created here.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// CHECK: receives the creation fee.
    #[account(mut, address = config.treasury @ BasketError::Unauthorized)]
    pub treasury: UncheckedAccount<'info>,

    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: ATA(basket, share_mint): treasury shares pass through here.
    #[account(mut)]
    pub basket_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(basket, wSOL): the sleeve's SOL leg passes through here.
    #[account(mut)]
    pub basket_wsol_ata: UncheckedAccount<'info>,

    /// New keypair; cp-amm initializes it (Token-2022, decimals 0).
    #[account(mut)]
    pub position_nft_mint: Signer<'info>,
    /// CHECK: cp-amm PDA, owned by the basket after the CPI.
    #[account(mut, address = damm::position_nft_account(&position_nft_mint.key()))]
    pub position_nft_account: UncheckedAccount<'info>,
    /// CHECK: cp-amm pool authority.
    #[account(address = damm::pool_authority())]
    pub pool_authority: UncheckedAccount<'info>,
    /// CHECK: cp-amm customizable pool PDA for (share_mint, wSOL).
    #[account(mut, address = damm::customizable_pool(&share_mint.key(), &wsol_mint.key()))]
    pub pool: UncheckedAccount<'info>,
    /// CHECK: cp-amm position PDA.
    #[account(mut, address = damm::position(&position_nft_mint.key()))]
    pub pool_position: UncheckedAccount<'info>,
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

pub fn handle_seed<'info>(ctx: Context<'info, Seed<'info>>, args: SeedArgs) -> Result<()> {
    let config = &ctx.accounts.config;
    require!(!config.paused, BasketError::Paused);
    let now = Clock::get()?.unix_timestamp;
    let basket_key = ctx.accounts.basket.key();
    {
        let basket = &ctx.accounts.basket;
        require!(basket.complete, BasketError::BasketIncomplete);
        require!(!basket.seeded, BasketError::AlreadySeeded);
        require!(basket.gate_open_at(now), BasketError::MintGateClosed);
    }
    require!(args.initial_shares > 0, BasketError::ZeroAmount);

    // 1. Components in.
    let n = ctx.accounts.basket.position_count as usize;
    require!(ctx.remaining_accounts.len() == n * COMPONENT_GROUP, BasketError::ComponentCountMismatch);
    let components = load_components(&basket_key, ctx.remaining_accounts, n)?;
    let payer_ai = ctx.accounts.payer.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let token_2022_program = ctx.accounts.token_2022_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    transfer_components_in(&components, &args.deposits, &payer_ai, &token_program, &token_2022_program)?;

    // 2. Sleeve split. The pool is empty, so the step ratio applies.
    let basket = &ctx.accounts.basket;
    let r_bps = effective_r_bps(&basket.sleeve, 0);
    let nominal_treasury_shares = math::treasury_shares(args.initial_shares, r_bps)?;
    require!(nominal_treasury_shares > 0 && args.sleeve_lamports > 0, BasketError::ZeroAmount);
    let implied_total = math::implied_total_lamports(args.sleeve_lamports, r_bps)?
        .checked_add(config.fees.creation_fee_lamports)
        .ok_or_else(|| error!(BasketError::MathOverflow))?;
    require!(implied_total >= config.fees.min_seed_lamports, BasketError::BelowMinimum);
    let (to_buyer, fee_shares) = split_mint_fee(args.initial_shares, basket.fees.mint_fee_bps)?;

    // 3. Pool geometry: marginal price = NAV = sleeve / nominal Y, range from
    //    the profile, liquidity sized to absorb the whole SOL leg; the share
    //    side follows from the range (see `math::sleeve_quote`).
    let sqrt_price = math::sqrt_price_from_amounts(args.sleeve_lamports, nominal_treasury_shares)?;
    let (sqrt_min, sqrt_max) = math::sqrt_price_bounds(sqrt_price, basket.sleeve.price_range)?;
    let quote = math::sleeve_quote(args.sleeve_lamports, nominal_treasury_shares, sqrt_min, sqrt_price, sqrt_max)?;
    require!(quote.liquidity > 0 && quote.amount_a > 0, BasketError::ZeroAmount);
    let treasury_shares = quote.amount_a;
    let liquidity = quote.liquidity;
    let pool_fees = pool_fee_parameters(config, basket)?;

    // 4. Creation fee.
    system_program::transfer(
        CpiContext::new(
            system_program.key(),
            system_program::Transfer { from: payer_ai.clone(), to: ctx.accounts.treasury.to_account_info() },
        ),
        config.fees.creation_fee_lamports,
    )?;

    // 5. Token accounts. The basket's own ATAs are created now so `mint`
    //    can rely on them.
    let basket_ai = ctx.accounts.basket.to_account_info();
    let share_mint_ai = ctx.accounts.share_mint.to_account_info();
    let wsol_mint_ai = ctx.accounts.wsol_mint.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let payer_wsol_ata = ctx.accounts.payer_wsol_ata.to_account_info();
    let payer_share_ata = ctx.accounts.payer_share_ata.to_account_info();
    let created_wsol_ata = payer_wsol_ata.data_is_empty();
    for (ata, authority, mint) in [
        (&payer_share_ata, &payer_ai, &share_mint_ai),
        (&payer_wsol_ata, &payer_ai, &wsol_mint_ai),
        (&ctx.accounts.fee_vault_share_ata.to_account_info(), &fee_vault_ai, &share_mint_ai),
        (&ctx.accounts.basket_share_ata.to_account_info(), &basket_ai, &share_mint_ai),
        (&ctx.accounts.basket_wsol_ata.to_account_info(), &basket_ai, &wsol_mint_ai),
    ] {
        ensure_ata(&payer_ai, ata, authority, mint, &system_program, &token_program, &ata_program)?;
    }

    // 6. Treasury shares and the SOL leg staged in the buyer's ATAs. The
    //    buyer holds no shares yet (supply is zero before the first buy).
    let signer = BasketSigner::new(&ctx.accounts.basket);
    let share_auth_ai = ctx.accounts.share_auth.to_account_info();
    mint_shares(&share_mint_ai, &payer_share_ata, &share_auth_ai, &token_program, &signer, treasury_shares)?;
    let wsol_before = crate::instructions::positions::read_token_amount(&payer_wsol_ata)?;
    wrap_sol(&payer_ai, &payer_wsol_ata, &system_program, &token_program, args.sleeve_lamports)?;

    // 7. Open the pool: buyer pays rent, basket owns the position NFT.
    cp_amm::cpi::initialize_customizable_pool(
        CpiContext::new(
            ctx.accounts.cp_amm_program.key(),
            cp_amm::cpi::accounts::InitializeCustomizablePool {
                creator: basket_ai.clone(),
                position_nft_mint: ctx.accounts.position_nft_mint.to_account_info(),
                position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
                payer: payer_ai.clone(),
                pool_authority: ctx.accounts.pool_authority.to_account_info(),
                pool: ctx.accounts.pool.to_account_info(),
                position: ctx.accounts.pool_position.to_account_info(),
                token_a_mint: share_mint_ai.clone(),
                token_b_mint: wsol_mint_ai.clone(),
                token_a_vault: ctx.accounts.token_a_vault.to_account_info(),
                token_b_vault: ctx.accounts.token_b_vault.to_account_info(),
                payer_token_a: payer_share_ata.clone(),
                payer_token_b: payer_wsol_ata.clone(),
                token_a_program: token_program.clone(),
                token_b_program: token_program.clone(),
                token_2022_program: token_2022_program.clone(),
                system_program: system_program.clone(),
                event_authority: ctx.accounts.event_authority.to_account_info(),
                program: ctx.accounts.cp_amm_program.to_account_info(),
            },
        ),
        cp_amm::types::InitializeCustomizablePoolParameters {
            pool_fees,
            sqrt_min_price: sqrt_min,
            sqrt_max_price: sqrt_max,
            has_alpha_vault: false,
            liquidity,
            sqrt_price,
            activation_type: damm::ACTIVATION_TYPE_TIMESTAMP,
            collect_fee_mode: damm::COLLECT_FEE_MODE_ONLY_B,
            activation_point: None,
        },
    )?;

    // 8. Leftovers: burn treasury-share dust the pool did not pull, and
    //    hand the buyer back their wSOL (closing the account if we made it).
    let share_dust = crate::instructions::positions::read_token_amount(&payer_share_ata)?;
    if share_dust > 0 {
        token::burn(
            CpiContext::new(
                token_program.key(),
                token::Burn { mint: share_mint_ai.clone(), from: payer_share_ata.clone(), authority: payer_ai.clone() },
            ),
            share_dust,
        )?;
    }
    if created_wsol_ata {
        token::close_account(CpiContext::new(
            token_program.key(),
            token::CloseAccount { account: payer_wsol_ata.clone(), destination: payer_ai.clone(), authority: payer_ai.clone() },
        ))?;
    } else {
        // Pre-existing wSOL account: only what we wrapped may have been spent.
        let wsol_after = crate::instructions::positions::read_token_amount(&payer_wsol_ata)?;
        require!(wsol_after >= wsol_before, BasketError::MathOverflow);
    }

    // 9. Buyer and fee shares.
    mint_shares(&share_mint_ai, &payer_share_ata, &share_auth_ai, &token_program, &signer, to_buyer)?;
    mint_shares(&share_mint_ai, &ctx.accounts.fee_vault_share_ata.to_account_info(), &share_auth_ai, &token_program, &signer, fee_shares)?;

    // 10. Bookkeeping.
    let fee_vault = &mut ctx.accounts.fee_vault;
    fee_vault.bump = ctx.bumps.fee_vault;
    fee_vault.basket = basket_key;
    let basket = &mut ctx.accounts.basket;
    basket.seeded = true;
    basket.pool = ctx.accounts.pool.key();
    basket.position_nft_mint = ctx.accounts.position_nft_mint.key();
    basket.pool_position = ctx.accounts.pool_position.key();
    basket.touch(now);

    emit!(Seeded {
        basket: basket_key,
        buyer: ctx.accounts.payer.key(),
        pool: basket.pool,
        position_nft_mint: basket.position_nft_mint,
        shares_to_buyer: to_buyer,
        fee_shares,
        treasury_shares,
        sleeve_lamports: args.sleeve_lamports,
        creation_fee_lamports: config.fees.creation_fee_lamports,
        sqrt_price,
    });
    Ok(())
}
