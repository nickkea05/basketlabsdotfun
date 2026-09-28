//! `claim_pool_fees` and `sweep_fees` (§4, §5).
//!
//! The FeeVault (`["fees", share_mint]`) accumulates two things: shares
//! (mint / redeem fees, withheld in kind) in its share ATA, and native SOL
//! (pool swap fees, `collect_fee_mode = OnlyB`) as lamports on the PDA
//! itself. `claim_pool_fees` cranks the position's pending fees through a
//! temporary wSOL account and unwraps them onto the PDA; `sweep_fees` splits
//! whatever arrived since the last sweep by the schedule frozen into the
//! basket: the creator and protocol lines leave, the holder line stays
//! reserved (`holder_reserve_*`) for the rewards distributor (D15).

use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token};

use crate::constants::*;
use crate::damm::{self, cp_amm};
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::positions::read_token_amount;
use crate::instructions::sleeve::*;
use crate::math;
use crate::state::*;

#[derive(Accounts)]
pub struct ClaimPoolFees<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = pool @ BasketError::PoolMismatch,
        has_one = pool_position @ BasketError::PoolMismatch,
    )]
    pub basket: Account<'info, Basket>,
    pub share_mint: Account<'info, Mint>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint); receives any share-side fee (zero under OnlyB).
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// CHECK: ATA(fee_vault, wSOL); created, filled, closed onto the FeeVault within this ix.
    #[account(mut)]
    pub fee_vault_wsol_ata: UncheckedAccount<'info>,
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,
    /// CHECK: cp-amm position NFT account owned by the basket.
    #[account(address = damm::position_nft_account(&basket.position_nft_mint))]
    pub position_nft_account: UncheckedAccount<'info>,
    /// CHECK: cp-amm pool authority.
    #[account(address = damm::pool_authority())]
    pub pool_authority: UncheckedAccount<'info>,
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
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SweepFees<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
    )]
    pub basket: Account<'info, Basket>,
    pub share_mint: Account<'info, Mint>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint), address-checked in the handler.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    /// CHECK: `basket.creator`; receives the creator SOL line.
    #[account(mut)]
    pub creator: UncheckedAccount<'info>,
    /// CHECK: ATA(creator, share_mint), created if missing (caller pays).
    #[account(mut)]
    pub creator_share_ata: UncheckedAccount<'info>,
    /// CHECK: `config.treasury`; receives the protocol SOL line.
    #[account(mut, address = config.treasury @ BasketError::Unauthorized)]
    pub treasury: UncheckedAccount<'info>,
    /// CHECK: ATA(treasury, share_mint), created if missing (caller pays).
    #[account(mut)]
    pub treasury_share_ata: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Move lamports between two accounts this program may debit.
fn move_lamports<'info>(from: &AccountInfo<'info>, to: &AccountInfo<'info>, amount: u64) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    **from.try_borrow_mut_lamports()? = from
        .lamports()
        .checked_sub(amount)
        .ok_or_else(|| error!(BasketError::MathOverflow))?;
    **to.try_borrow_mut_lamports()? = to
        .lamports()
        .checked_add(amount)
        .ok_or_else(|| error!(BasketError::MathOverflow))?;
    Ok(())
}

pub fn handle_claim_pool_fees(ctx: Context<ClaimPoolFees>) -> Result<()> {
    let caller = ctx.accounts.caller.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let wsol_mint = ctx.accounts.wsol_mint.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let before = fee_vault_ai.lamports();

    ensure_ata(&caller, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, &system_program, &token_program, &ata_program)?;
    let created_wsol = ctx.accounts.fee_vault_wsol_ata.data_is_empty();
    ensure_ata(&caller, &ctx.accounts.fee_vault_wsol_ata, &fee_vault_ai, &wsol_mint, &system_program, &token_program, &ata_program)?;
    let wsol_rent = if created_wsol { ctx.accounts.fee_vault_wsol_ata.lamports() } else { 0 };
    let wsol_before = read_token_amount(&ctx.accounts.fee_vault_wsol_ata)?;

    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_seeds = signer.basket_seeds();
    cp_amm::cpi::claim_position_fee(CpiContext::new_with_signer(
        ctx.accounts.cp_amm_program.key(),
        cp_amm::cpi::accounts::ClaimPositionFee {
            pool_authority: ctx.accounts.pool_authority.to_account_info(),
            pool: ctx.accounts.pool.to_account_info(),
            position: ctx.accounts.pool_position.to_account_info(),
            token_a_account: ctx.accounts.fee_vault_share_ata.to_account_info(),
            token_b_account: ctx.accounts.fee_vault_wsol_ata.to_account_info(),
            token_a_vault: ctx.accounts.token_a_vault.to_account_info(),
            token_b_vault: ctx.accounts.token_b_vault.to_account_info(),
            token_a_mint: share_mint.clone(),
            token_b_mint: wsol_mint.clone(),
            position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
            signer: ctx.accounts.basket.to_account_info(),
            token_a_program: token_program.clone(),
            token_b_program: token_program.clone(),
            event_authority: ctx.accounts.event_authority.to_account_info(),
            program: ctx.accounts.cp_amm_program.to_account_info(),
        },
        &[&basket_seeds],
    ))?;

    let claimed = read_token_amount(&ctx.accounts.fee_vault_wsol_ata)? - wsol_before;

    // Unwrap onto the FeeVault PDA and hand the temporary account's rent back.
    let fee_seeds: [&[u8]; 3] = [seeds::FEES, signer.share_mint.as_ref(), &[ctx.accounts.fee_vault.bump]];
    token::close_account(CpiContext::new_with_signer(
        token_program.key(),
        token::CloseAccount {
            account: ctx.accounts.fee_vault_wsol_ata.to_account_info(),
            destination: fee_vault_ai.clone(),
            authority: fee_vault_ai.clone(),
        },
        &[&fee_seeds],
    ))?;
    move_lamports(&fee_vault_ai, &caller, wsol_rent)?;
    require!(fee_vault_ai.lamports() >= before, BasketError::MathOverflow);

    emit!(PoolFeesClaimed { basket: ctx.accounts.basket.key(), lamports: claimed });
    Ok(())
}

pub fn handle_sweep_fees(ctx: Context<SweepFees>) -> Result<()> {
    let caller = ctx.accounts.caller.to_account_info();
    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let share_mint = ctx.accounts.share_mint.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let fees = ctx.accounts.basket.fees;

    ensure_ata(&caller, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, &system_program, &token_program, &ata_program)?;
    let vault = &ctx.accounts.fee_vault;
    let shares_total = read_token_amount(&ctx.accounts.fee_vault_share_ata)?;
    let shares_new = shares_total.saturating_sub(vault.holder_reserve_shares);
    let rent = Rent::get()?;
    let free = fee_vault_ai.lamports().saturating_sub(rent.minimum_balance(fee_vault_ai.data_len()));
    let lamports_new = free.saturating_sub(vault.holder_reserve_lamports).saturating_sub(vault.creator_owed_lamports);

    let hs = math::bps(shares_new, fees.holder_split_bps)?;
    let cs = math::bps(shares_new, fees.creator_split_bps)?;
    let ps = shares_new - hs - cs;
    let hl = math::bps(lamports_new, fees.holder_split_bps)?;
    let cl = math::bps(lamports_new, fees.creator_split_bps)?;
    let pl = lamports_new - hl - cl;

    if cs + ps > 0 {
        let fee_seeds: [&[u8]; 3] = [seeds::FEES, ctx.accounts.basket.share_mint.as_ref(), &[vault.bump]];
        for (ata, owner, amount) in [
            (&ctx.accounts.creator_share_ata, &ctx.accounts.creator, cs),
            (&ctx.accounts.treasury_share_ata, &ctx.accounts.treasury, ps),
        ] {
            if amount == 0 {
                continue;
            }
            ensure_ata(&caller, ata, owner, &share_mint, &system_program, &token_program, &ata_program)?;
            token::transfer(
                CpiContext::new_with_signer(
                    token_program.key(),
                    token::Transfer {
                        from: ctx.accounts.fee_vault_share_ata.to_account_info(),
                        to: ata.to_account_info(),
                        authority: fee_vault_ai.clone(),
                    },
                    &[&fee_seeds],
                ),
                amount,
            )?;
        }
    }
    // The creator line (plus anything held back earlier) goes out only if
    // it leaves the creator's wallet rent-exempt; otherwise it waits.
    let creator_due = vault.creator_owed_lamports + cl;
    let creator_ai = ctx.accounts.creator.to_account_info();
    let creator_paid = if creator_due == 0 {
        0
    } else if creator_ai.lamports() + creator_due >= rent.minimum_balance(creator_ai.data_len()) {
        move_lamports(&fee_vault_ai, &creator_ai, creator_due)?;
        creator_due
    } else {
        0
    };
    move_lamports(&fee_vault_ai, &ctx.accounts.treasury, pl)?;

    let v = &mut ctx.accounts.fee_vault;
    v.holder_reserve_shares += hs;
    v.holder_reserve_lamports += hl;
    v.creator_owed_lamports = creator_due - creator_paid;
    v.swept_creator_shares += cs;
    v.swept_creator_lamports += creator_paid;
    v.swept_protocol_shares += ps;
    v.swept_protocol_lamports += pl;

    emit!(FeesSwept {
        basket: ctx.accounts.basket.key(),
        creator_shares: cs,
        creator_lamports: creator_paid,
        protocol_shares: ps,
        protocol_lamports: pl,
        holder_shares: hs,
        holder_lamports: hl,
    });
    Ok(())
}
