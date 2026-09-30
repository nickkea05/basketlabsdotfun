//! Phase 7: holder rewards (D15), the creator lock, and the close crank.
//!
//! Rewards: the keeper commits one Merkle root per epoch over
//! `(index, wallet, amount)` leaves, funded from the FeeVault's holder
//! reserve in shares or SOL. `distribute_rewards` is permissionless (the
//! keeper pushes; anyone may), pays each leaf once, and closes the root when
//! everything is paid. `close_rewards_root` lets the keeper retire a stale
//! root and return the unpaid remainder to the reserve.
//!
//! Creator lock: shares parked in ATA(CreatorLock, share_mint) until
//! `unlock_at`; top-ups extend, never shorten.
//!
//! Close: once no one outside the FeeVault holds shares and the basket has
//! been idle for `close_idle_s`, the keeper runs `close_positions` (vault
//! dust to the treasury, rent to the basket's payer) in chunks, then
//! `close_basket` (sleeve drained to the treasury, leftover shares burned,
//! cp-amm position, FeeVault and Basket closed, rent to the payer).

use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;
use anchor_lang::AccountsClose;
use anchor_spl::associated_token::{get_associated_token_address_with_program_id, AssociatedToken};
use anchor_spl::token::{self, Mint, Token, TokenAccount};
use anchor_spl::token_2022::Token2022;
use anchor_spl::token_interface;

use crate::constants::*;
use crate::damm::{self, cp_amm};
use crate::error::BasketError;
use crate::events::*;
use crate::instructions::fees::move_lamports;
use crate::instructions::positions::read_token_amount;
use crate::instructions::sleeve::*;
use crate::state::*;

// ------------------------------------------------------- post_rewards_root

#[derive(Accounts)]
#[instruction(epoch: u64)]
pub struct PostRewardsRoot<'info> {
    /// Pays the root's rent; refunded when it closes.
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(seeds = [seeds::BASKET, share_mint.key().as_ref()], bump = basket.bump, has_one = share_mint)]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: `["rewards", share_mint, epoch]`, created here (variable size).
    #[account(mut, seeds = [seeds::REWARDS, share_mint.key().as_ref(), &epoch.to_le_bytes()], bump)]
    pub rewards_root: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

pub fn handle_post_rewards_root(
    ctx: Context<PostRewardsRoot>,
    epoch: u64,
    root: [u8; 32],
    reward_mint: Pubkey,
    total_amount: u64,
    leaf_count: u32,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require!(total_amount > 0, BasketError::ZeroAmount);
    require!(leaf_count > 0 && leaf_count <= MAX_REWARD_LEAVES, BasketError::InvalidArgument);
    let root_ai = ctx.accounts.rewards_root.to_account_info();
    require!(root_ai.data_is_empty(), BasketError::ReplayDetected);

    // Commit the amount out of the reserve now, so two roots cannot promise
    // the same shares/SOL.
    let vault = &mut ctx.accounts.fee_vault;
    let share_mint = ctx.accounts.basket.share_mint;
    if reward_mint == share_mint {
        vault.holder_reserve_shares =
            vault.holder_reserve_shares.checked_sub(total_amount).ok_or_else(|| error!(BasketError::BelowMinimum))?;
    } else if reward_mint == anchor_spl::token::spl_token::native_mint::ID {
        vault.holder_reserve_lamports =
            vault.holder_reserve_lamports.checked_sub(total_amount).ok_or_else(|| error!(BasketError::BelowMinimum))?;
    } else {
        return err!(BasketError::InvalidArgument);
    }

    let bump = ctx.bumps.rewards_root;
    let space = RewardsRoot::space_for(leaf_count);
    let lamports = Rent::get()?.minimum_balance(space);
    let epoch_le = epoch.to_le_bytes();
    invoke_signed(
        &system_instruction::create_account(ctx.accounts.keeper.key, root_ai.key, lamports, space as u64, &crate::ID),
        &[ctx.accounts.keeper.to_account_info(), root_ai.clone(), ctx.accounts.system_program.to_account_info()],
        &[&[seeds::REWARDS, share_mint.as_ref(), &epoch_le, &[bump]]],
    )?;
    let state = RewardsRoot {
        bump,
        basket: ctx.accounts.basket.key(),
        payer: ctx.accounts.keeper.key(),
        epoch,
        root,
        reward_mint,
        total_amount,
        distributed_amount: 0,
        leaf_count,
        posted_at: now,
        claimed: vec![0u8; leaf_count.div_ceil(8) as usize],
    };
    {
        let mut data = root_ai.try_borrow_mut_data()?;
        state.try_serialize(&mut &mut data[..])?;
    }

    emit!(RewardsRootPosted {
        basket: ctx.accounts.basket.key(),
        epoch,
        root,
        reward_mint,
        total_amount,
        leaf_count,
    });
    Ok(())
}

// ------------------------------------------------------ distribute_rewards

#[derive(Accounts)]
pub struct DistributeRewards<'info> {
    /// Anyone; pays the wallet's share ATA if missing.
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(seeds = [seeds::BASKET, share_mint.key().as_ref()], bump = basket.bump, has_one = share_mint)]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    /// CHECK: ATA(fee_vault, share_mint); address-checked when used.
    #[account(mut)]
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    #[account(mut, has_one = basket @ BasketError::ComponentMismatch)]
    pub rewards_root: Account<'info, RewardsRoot>,
    /// CHECK: `rewards_root.payer`, gets the rent back when the root closes.
    #[account(mut, address = rewards_root.payer @ BasketError::Unauthorized)]
    pub root_payer: UncheckedAccount<'info>,
    /// CHECK: the leaf's wallet.
    #[account(mut)]
    pub wallet: UncheckedAccount<'info>,
    /// CHECK: ATA(wallet, share_mint), created if missing (share rewards).
    #[account(mut)]
    pub wallet_share_ata: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_distribute_rewards(ctx: Context<DistributeRewards>, index: u32, amount: u64, proof: Vec<[u8; 32]>) -> Result<()> {
    require!(amount > 0, BasketError::ZeroAmount);
    let root = &ctx.accounts.rewards_root;
    require!(index < root.leaf_count && !root.is_claimed(index), BasketError::AlreadyClaimed);
    let leaf = RewardsRoot::leaf(index, ctx.accounts.wallet.key, amount);
    require!(RewardsRoot::verify(&root.root, leaf, &proof), BasketError::BadProof);
    let distributed = root.distributed_amount.checked_add(amount).filter(|d| *d <= root.total_amount);
    let distributed = distributed.ok_or_else(|| error!(BasketError::InvalidArgument))?;

    let fee_vault_ai = ctx.accounts.fee_vault.to_account_info();
    let wallet = ctx.accounts.wallet.to_account_info();
    let fee_seeds: [&[u8]; 3] = [seeds::FEES, ctx.accounts.basket.share_mint.as_ref(), &[ctx.accounts.fee_vault.bump]];
    if root.is_native() {
        // Never leave a system account below rent exemption.
        let rent = Rent::get()?.minimum_balance(wallet.data_len());
        require!(wallet.lamports().saturating_add(amount) >= rent, BasketError::BelowMinimum);
        move_lamports(&fee_vault_ai, &wallet, amount)?;
        ctx.accounts.fee_vault.distributed_lamports += amount;
    } else {
        let share_mint = ctx.accounts.share_mint.to_account_info();
        let token_program = ctx.accounts.token_program.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        let ata_program = ctx.accounts.associated_token_program.to_account_info();
        let caller = ctx.accounts.caller.to_account_info();
        ensure_ata(&caller, &ctx.accounts.fee_vault_share_ata, &fee_vault_ai, &share_mint, &system_program, &token_program, &ata_program)?;
        ensure_ata(&caller, &ctx.accounts.wallet_share_ata, &wallet, &share_mint, &system_program, &token_program, &ata_program)?;
        token::transfer(
            CpiContext::new_with_signer(
                token_program.key(),
                token::Transfer {
                    from: ctx.accounts.fee_vault_share_ata.to_account_info(),
                    to: ctx.accounts.wallet_share_ata.to_account_info(),
                    authority: fee_vault_ai.clone(),
                },
                &[&fee_seeds],
            ),
            amount,
        )?;
        ctx.accounts.fee_vault.distributed_shares += amount;
    }

    let root = &mut ctx.accounts.rewards_root;
    root.set_claimed(index);
    root.distributed_amount = distributed;
    let epoch = root.epoch;
    if distributed == root.total_amount {
        ctx.accounts.rewards_root.close(ctx.accounts.root_payer.to_account_info())?;
    }

    emit!(RewardDistributed { basket: ctx.accounts.basket.key(), epoch, wallet: ctx.accounts.wallet.key(), amount, index });
    Ok(())
}

// ------------------------------------------------------ close_rewards_root

#[derive(Accounts)]
pub struct CloseRewardsRoot<'info> {
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(seeds = [seeds::BASKET, share_mint.key().as_ref()], bump = basket.bump, has_one = share_mint)]
    pub basket: Account<'info, Basket>,
    /// CHECK: `basket.share_mint`.
    pub share_mint: UncheckedAccount<'info>,
    #[account(mut, seeds = [seeds::FEES, share_mint.key().as_ref()], bump = fee_vault.bump)]
    pub fee_vault: Account<'info, FeeVault>,
    #[account(mut, has_one = basket @ BasketError::ComponentMismatch)]
    pub rewards_root: Account<'info, RewardsRoot>,
    /// CHECK: `rewards_root.payer`.
    #[account(mut, address = rewards_root.payer @ BasketError::Unauthorized)]
    pub root_payer: UncheckedAccount<'info>,
}

pub fn handle_close_rewards_root(ctx: Context<CloseRewardsRoot>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    let root = &ctx.accounts.rewards_root;
    let paid = root.distributed_amount == root.total_amount;
    require!(paid || now >= root.posted_at.saturating_add(REWARDS_ROOT_TTL_S), BasketError::TimelockActive);
    let returned = root.total_amount - root.distributed_amount;
    let vault = &mut ctx.accounts.fee_vault;
    if root.is_native() {
        vault.holder_reserve_lamports += returned;
    } else {
        vault.holder_reserve_shares += returned;
    }
    let (epoch, distributed_amount) = (root.epoch, root.distributed_amount);
    ctx.accounts.rewards_root.close(ctx.accounts.root_payer.to_account_info())?;
    emit!(RewardsRootClosed { basket: ctx.accounts.basket.key(), epoch, distributed_amount, returned_amount: returned });
    Ok(())
}

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
    pub basket: Account<'info, Basket>,
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
    pub basket: Account<'info, Basket>,
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

/// Shared precondition: idle for `close_idle_s`, nothing in flight, and no
/// shares outside the FeeVault's own fee stash (holder shares == fee shares).
fn require_closable(
    basket: &Basket,
    config: &Config,
    now: i64,
    supply: u64,
    pool: Option<&AccountLoader<cp_amm::accounts::Pool>>,
    pool_position: Option<&AccountLoader<cp_amm::accounts::Position>>,
    fee_vault_share_ata: &AccountInfo,
    pending_book: &AccountInfo,
) -> Result<()> {
    require!(!basket.rebalance.active && basket.pending_redeem_shares == 0, BasketError::NotClosable);
    require!(pending_book.data_is_empty(), BasketError::NotClosable);
    require!(now >= basket.last_activity_at.saturating_add(config.close_idle_s), BasketError::NotClosable);
    let fee_shares = if fee_vault_share_ata.data_is_empty() { 0 } else { read_token_amount(fee_vault_share_ata)? };
    let h = if basket.seeded {
        let pool = pool.ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let position = pool_position.ok_or_else(|| error!(BasketError::PoolMismatch))?;
        require_keys_eq!(pool.key(), basket.pool, BasketError::PoolMismatch);
        require_keys_eq!(position.key(), basket.pool_position, BasketError::PoolMismatch);
        let (pool, position) = (pool.load()?, position.load()?);
        holder_shares(supply, &pool, &position, 0)?
    } else {
        supply
    };
    // Liquidity ↔ share-amount conversion is only exact to a few base units
    // after partial removals; a stray holder of that much owns nothing.
    require!(h <= fee_shares.saturating_add(CLOSE_DUST_SHARES), BasketError::NotClosable);
    Ok(())
}

#[derive(Accounts)]
pub struct ClosePositions<'info> {
    /// Pays treasury ATAs for dust if missing.
    #[account(mut)]
    pub keeper: Signer<'info>,
    #[account(seeds = [seeds::CONFIG], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = payer @ BasketError::Unauthorized,
    )]
    pub basket: Account<'info, Basket>,
    pub share_mint: Account<'info, Mint>,
    /// CHECK: ATA(fee_vault, share_mint), may not exist.
    pub fee_vault_share_ata: UncheckedAccount<'info>,
    pub pool: Option<AccountLoader<'info, cp_amm::accounts::Pool>>,
    pub pool_position: Option<AccountLoader<'info, cp_amm::accounts::Position>>,
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
    // remaining: `[position, vault, mint, treasury_ata]` per position.
}

pub fn handle_close_positions<'info>(ctx: Context<'info, ClosePositions<'info>>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require_closable(
        &ctx.accounts.basket,
        &ctx.accounts.config,
        now,
        ctx.accounts.share_mint.supply,
        ctx.accounts.pool.as_ref(),
        ctx.accounts.pool_position.as_ref(),
        &ctx.accounts.fee_vault_share_ata,
        &ctx.accounts.pending_book,
    )?;
    let remaining = ctx.remaining_accounts;
    require!(!remaining.is_empty() && remaining.len() % 4 == 0, BasketError::ComponentCountMismatch);

    let basket_key = ctx.accounts.basket.key();
    let basket_ai = ctx.accounts.basket.to_account_info();
    let signer = BasketSigner::new(&ctx.accounts.basket);
    let basket_seeds = signer.basket_seeds();
    let keeper = ctx.accounts.keeper.to_account_info();
    let treasury = ctx.accounts.treasury.to_account_info();
    let payer = ctx.accounts.payer.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();

    let mut closing: Vec<Account<Position>> = Vec::with_capacity(remaining.len() / 4);
    for group in remaining.chunks(4) {
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
    pub config: Account<'info, Config>,
    #[account(
        mut,
        seeds = [seeds::BASKET, share_mint.key().as_ref()],
        bump = basket.bump,
        has_one = share_mint,
        has_one = creator @ BasketError::Unauthorized,
        has_one = payer @ BasketError::Unauthorized,
    )]
    pub basket: Account<'info, Basket>,
    #[account(mut)]
    pub share_mint: Account<'info, Mint>,
    /// Seeded baskets only.
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
    /// CHECK: `basket.payer`, receives rent.
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
    /// CHECK: cp-amm position NFT mint (Token-2022).
    #[account(mut)]
    pub position_nft_mint: Option<UncheckedAccount<'info>>,
    /// CHECK: position NFT account owned by the basket.
    #[account(mut)]
    pub position_nft_account: Option<UncheckedAccount<'info>>,
    /// CHECK: cp-amm pool authority.
    #[account(address = damm::pool_authority())]
    pub pool_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub pool: Option<AccountLoader<'info, cp_amm::accounts::Pool>>,
    #[account(mut)]
    pub pool_position: Option<AccountLoader<'info, cp_amm::accounts::Position>>,
    /// CHECK: cp-amm vault for the share mint.
    #[account(mut)]
    pub token_a_vault: Option<UncheckedAccount<'info>>,
    /// CHECK: cp-amm vault for wSOL.
    #[account(mut)]
    pub token_b_vault: Option<UncheckedAccount<'info>>,
    pub cp_amm_program: Program<'info, cp_amm::program::CpAmm>,
    /// CHECK: cp-amm event authority PDA.
    #[account(address = damm::event_authority())]
    pub event_authority: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub token_2022_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

pub fn handle_close_basket(ctx: Context<CloseBasket>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    require!(ctx.accounts.config.is_keeper(ctx.accounts.keeper.key), BasketError::Unauthorized);
    require_closable(
        &ctx.accounts.basket,
        &ctx.accounts.config,
        now,
        ctx.accounts.share_mint.supply,
        ctx.accounts.pool.as_ref(),
        ctx.accounts.pool_position.as_ref(),
        &ctx.accounts.fee_vault_share_ata,
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
    let token_program = ctx.accounts.token_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let ata_program = ctx.accounts.associated_token_program.to_account_info();

    if ctx.accounts.basket.seeded {
        let pool = ctx.accounts.pool.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let position = ctx.accounts.pool_position.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let nft_mint = ctx.accounts.position_nft_mint.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let nft_account = ctx.accounts.position_nft_account.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let token_a_vault = ctx.accounts.token_a_vault.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let token_b_vault = ctx.accounts.token_b_vault.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let b = &ctx.accounts.basket;
        require_keys_eq!(nft_mint.key(), b.position_nft_mint, BasketError::PoolMismatch);
        require_keys_eq!(nft_account.key(), damm::position_nft_account(&b.position_nft_mint), BasketError::PoolMismatch);
        require_keys_eq!(token_a_vault.key(), damm::token_vault(&b.share_mint, &b.pool), BasketError::PoolMismatch);
        require_keys_eq!(token_b_vault.key(), damm::token_vault(&ctx.accounts.wsol_mint.key(), &b.pool), BasketError::PoolMismatch);

        ensure_ata(&keeper, &ctx.accounts.basket_share_ata, &basket_ai, &share_mint, &system_program, &token_program, &ata_program)?;
        ensure_ata(&keeper, &ctx.accounts.treasury_wsol_ata, &treasury, &ctx.accounts.wsol_mint.to_account_info(), &system_program, &token_program, &ata_program)?;

        let liquidity = position.load()?.unlocked_liquidity;
        let pool_accounts = || cp_amm::cpi::accounts::RemoveLiquidity {
            pool_authority: ctx.accounts.pool_authority.to_account_info(),
            pool: pool.to_account_info(),
            position: position.to_account_info(),
            token_a_account: ctx.accounts.basket_share_ata.to_account_info(),
            token_b_account: ctx.accounts.treasury_wsol_ata.to_account_info(),
            token_a_vault: token_a_vault.to_account_info(),
            token_b_vault: token_b_vault.to_account_info(),
            token_a_mint: share_mint.clone(),
            token_b_mint: ctx.accounts.wsol_mint.to_account_info(),
            position_nft_account: nft_account.to_account_info(),
            signer: basket_ai.clone(),
            token_a_program: token_program.clone(),
            token_b_program: token_program.clone(),
            event_authority: ctx.accounts.event_authority.to_account_info(),
            program: ctx.accounts.cp_amm_program.to_account_info(),
        };
        if liquidity > 0 {
            cp_amm::cpi::remove_liquidity(
                CpiContext::new_with_signer(ctx.accounts.cp_amm_program.key(), pool_accounts(), &[&basket_seeds]),
                cp_amm::types::RemoveLiquidityParameters { liquidity_delta: liquidity, token_a_amount_threshold: 0, token_b_amount_threshold: 0 },
            )?;
        }
        // Pending swap fees, then the empty position itself (rent to payer).
        let r = pool_accounts();
        cp_amm::cpi::claim_position_fee(CpiContext::new_with_signer(
            ctx.accounts.cp_amm_program.key(),
            cp_amm::cpi::accounts::ClaimPositionFee {
                pool_authority: r.pool_authority,
                pool: r.pool,
                position: r.position,
                token_a_account: r.token_a_account,
                token_b_account: r.token_b_account,
                token_a_vault: r.token_a_vault,
                token_b_vault: r.token_b_vault,
                token_a_mint: r.token_a_mint,
                token_b_mint: r.token_b_mint,
                position_nft_account: r.position_nft_account,
                signer: r.signer,
                token_a_program: r.token_a_program,
                token_b_program: r.token_b_program,
                event_authority: r.event_authority,
                program: r.program,
            },
            &[&basket_seeds],
        ))?;
        cp_amm::cpi::close_position(CpiContext::new_with_signer(
            ctx.accounts.cp_amm_program.key(),
            cp_amm::cpi::accounts::ClosePosition {
                position_nft_mint: nft_mint.to_account_info(),
                position_nft_account: nft_account.to_account_info(),
                pool: pool.to_account_info(),
                position: position.to_account_info(),
                pool_authority: ctx.accounts.pool_authority.to_account_info(),
                rent_receiver: payer.clone(),
                owner: basket_ai.clone(),
                token_program: ctx.accounts.token_2022_program.to_account_info(),
                event_authority: ctx.accounts.event_authority.to_account_info(),
                program: ctx.accounts.cp_amm_program.to_account_info(),
            },
            &[&basket_seeds],
        ))?;
        // Withdrawn treasury shares.
        burn_basket_share_dust(&basket_ai, &ctx.accounts.basket_share_ata, &share_mint, &token_program, &signer)?;
        token::close_account(CpiContext::new_with_signer(
            token_program.key(),
            token::CloseAccount { account: ctx.accounts.basket_share_ata.to_account_info(), destination: payer.clone(), authority: basket_ai.clone() },
            &[&basket_seeds],
        ))?;

        // FeeVault: burn its fee shares, pay the creator's held-back SOL, the
        // rest of the free SOL to the treasury, rent to the payer.
        let vault = ctx.accounts.fee_vault.as_ref().ok_or_else(|| error!(BasketError::PoolMismatch))?;
        let vault_ai = vault.to_account_info();
        let fee_seeds: [&[u8]; 3] = [seeds::FEES, b.share_mint.as_ref(), &[vault.bump]];
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
        vault.close(payer.clone())?;

        // Liquidity→amount flooring leaves a few base units in the pool vault.
        ctx.accounts.share_mint.reload()?;
        require!(ctx.accounts.share_mint.supply <= CLOSE_DUST_SHARES, BasketError::NotClosable);
    } else {
        require!(ctx.accounts.share_mint.supply == 0, BasketError::NotClosable);
    }

    let lamports = basket_ai.lamports();
    let refunded_to = payer.key();
    ctx.accounts.basket.close(payer)?;
    emit!(BasketClosed { basket: basket_key, share_mint: share_mint.key(), refunded_to, lamports });
    Ok(())
}
