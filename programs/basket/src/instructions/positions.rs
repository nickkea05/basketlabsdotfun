//! Shared component-position plumbing: creating `Position` PDAs and vault
//! ATAs from remaining accounts (used by `create_basket`, `add_positions`
//! and `finalize_rebalance`), and reading back `[position, vault, ...]`
//! groups for `mint` / `redeem`.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::program::invoke_signed;
use anchor_lang::solana_program::system_instruction;
use anchor_spl::associated_token::{self, get_associated_token_address_with_program_id};
use anchor_spl::token_2022::spl_token_2022;
use anchor_spl::token_interface::{Mint, TokenAccount};

use crate::constants::*;
use crate::error::BasketError;
use crate::state::*;

pub struct PositionPrograms<'a, 'info> {
    pub payer: &'a AccountInfo<'info>,
    pub token_program: &'a AccountInfo<'info>,
    pub token_2022_program: &'a AccountInfo<'info>,
    pub associated_token_program: &'a AccountInfo<'info>,
    pub system_program: &'a AccountInfo<'info>,
}

pub fn token_program_for_mint(mint: &AccountInfo) -> Result<Pubkey> {
    if *mint.owner == anchor_spl::token::ID || *mint.owner == spl_token_2022::ID {
        Ok(*mint.owner)
    } else {
        err!(BasketError::BadTokenProgram)
    }
}

/// Create one `Position` PDA + vault ATA per `(mint, position, vault)`
/// account triplet, in order, appending to the basket's book. Verifies
/// whitelist membership, the token program, the PDA and the ATA addresses.
/// Returns how many were added.
pub fn append_positions<'info>(
    basket: &mut Account<'info, Basket>,
    whitelist: &Whitelist,
    programs: &PositionPrograms<'_, 'info>,
    remaining: &[AccountInfo<'info>],
    positions: &[PositionArg],
) -> Result<u16> {
    require!(remaining.len() == positions.len() * 3, BasketError::ComponentCountMismatch);
    // Duplicates inside the chunk. Across chunks the PDA already exists.
    for (i, a) in positions.iter().enumerate() {
        require!(
            positions[..i].iter().all(|b| b.mint != a.mint),
            BasketError::DuplicateMint
        );
    }

    let basket_key = basket.key();
    let share_mint = basket.share_mint;
    for (i, arg) in positions.iter().enumerate() {
        let mint_ai = &remaining[i * 3];
        let pos_ai = &remaining[i * 3 + 1];
        let vault_ai = &remaining[i * 3 + 2];

        require_keys_eq!(mint_ai.key(), arg.mint, BasketError::ComponentMismatch);
        require!(arg.weight_bps > 0, BasketError::InvalidArgument);
        require!(whitelist.contains(&arg.mint), BasketError::MintNotWhitelisted);
        let token_program = token_program_for_mint(mint_ai)?;
        let decimals = {
            let data = mint_ai.try_borrow_data()?;
            Mint::try_deserialize(&mut &data[..])?.decimals
        };

        let (pos_key, pos_bump) = Pubkey::find_program_address(
            &[seeds::POSITION, share_mint.as_ref(), arg.mint.as_ref()],
            &crate::ID,
        );
        require_keys_eq!(pos_ai.key(), pos_key, BasketError::ComponentMismatch);
        require!(
            pos_ai.data_is_empty() && pos_ai.lamports() == 0,
            BasketError::DuplicateMint
        );

        let vault = get_associated_token_address_with_program_id(&basket_key, &arg.mint, &token_program);
        require_keys_eq!(vault_ai.key(), vault, BasketError::ComponentMismatch);

        // Position account.
        let space = 8 + Position::INIT_SPACE;
        let lamports = Rent::get()?.minimum_balance(space);
        invoke_signed(
            &system_instruction::create_account(
                programs.payer.key,
                &pos_key,
                lamports,
                space as u64,
                &crate::ID,
            ),
            &[programs.payer.clone(), pos_ai.clone(), programs.system_program.clone()],
            &[&[seeds::POSITION, share_mint.as_ref(), arg.mint.as_ref(), &[pos_bump]]],
        )?;
        let index = basket.position_count;
        let position = Position {
            bump: pos_bump,
            basket: basket_key,
            mint: arg.mint,
            weight_bps: arg.weight_bps,
            token_program,
            decimals,
            vault,
            owed: 0,
            index,
        };
        {
            let mut data = pos_ai.try_borrow_mut_data()?;
            position.try_serialize(&mut &mut data[..])?;
        }

        // Vault ATA (idempotent: someone may have pre-created it).
        let tp = if token_program == anchor_spl::token::ID {
            programs.token_program
        } else {
            programs.token_2022_program
        };
        associated_token::create_idempotent(CpiContext::new(
            programs.associated_token_program.key(),
            associated_token::Create {
                payer: programs.payer.clone(),
                associated_token: vault_ai.clone(),
                authority: basket.to_account_info(),
                mint: mint_ai.clone(),
                system_program: programs.system_program.clone(),
                token_program: tp.clone(),
            },
        ))?;

        basket.book_acc = Basket::chain_hash(&basket.book_acc, &arg.mint, arg.weight_bps);
        basket.weight_acc = basket
            .weight_acc
            .checked_add(arg.weight_bps)
            .filter(|w| *w <= BPS_TOTAL)
            .ok_or_else(|| error!(BasketError::WeightsMustSumToTotal))?;
        basket.position_count = basket
            .position_count
            .checked_add(1)
            .filter(|c| *c <= basket.asset_count)
            .ok_or_else(|| error!(BasketError::TooManyAssets))?;
    }
    Ok(positions.len() as u16)
}

/// Once every position is in, the running hash and weight sum must match the
/// signed commitment. Only then does the basket accept a `seed`.
pub fn try_complete(basket: &mut Basket) -> Result<bool> {
    if basket.position_count != basket.asset_count {
        return Ok(false);
    }
    require!(basket.weight_acc == BPS_TOTAL, BasketError::WeightsMustSumToTotal);
    require!(basket.book_acc == basket.book_hash, BasketError::BookHashMismatch);
    basket.complete = true;
    Ok(true)
}

/// One component as passed to `mint` / `redeem`: `[position, vault, user_ata, mint]`.
pub struct ComponentAccounts<'info> {
    pub position: Account<'info, Position>,
    pub position_ai: &'info AccountInfo<'info>,
    pub vault: &'info AccountInfo<'info>,
    pub user_ata: &'info AccountInfo<'info>,
    pub mint: &'info AccountInfo<'info>,
    pub vault_amount: u64,
    /// `vault_amount − position.owed`: what backs holder shares. Amounts
    /// owed to frozen-redemption claimants (D10) are excluded from every
    /// pro-rata rule.
    pub available: u64,
    pub vault_frozen: bool,
    pub user_frozen: bool,
}

impl ComponentAccounts<'_> {
    /// `remaining` accounts after the component groups (e.g. FrozenClaim PDAs).
    pub fn trailing<'a, 'info>(remaining: &'a [AccountInfo<'info>], n: usize) -> &'a [AccountInfo<'info>] {
        &remaining[(n * COMPONENT_GROUP).min(remaining.len())..]
    }
}

pub const COMPONENT_GROUP: usize = 4;

/// Parse `n` leading `[position, vault, user_ata, mint]` groups (anything
/// after them is left to the caller), verify each belongs to `basket` with
/// no duplicates, and read the vault balances. Callers that need the full
/// book pass `n = basket.position_count`.
pub fn load_components<'info>(
    basket_key: &Pubkey,
    remaining: &'info [AccountInfo<'info>],
    n: usize,
) -> Result<Vec<ComponentAccounts<'info>>> {
    require!(n > 0 && remaining.len() >= n * COMPONENT_GROUP, BasketError::ComponentCountMismatch);
    let mut out: Vec<ComponentAccounts<'info>> = Vec::with_capacity(n);
    for i in 0..n {
        let g = &remaining[i * COMPONENT_GROUP..(i + 1) * COMPONENT_GROUP];
        let position_ai = &g[0];
        let position = Account::<Position>::try_from(position_ai)?;
        require_keys_eq!(position.basket, *basket_key, BasketError::ComponentMismatch);
        require!(
            out.iter().all(|c| c.position.mint != position.mint),
            BasketError::DuplicateMint
        );
        require_keys_eq!(g[1].key(), position.vault, BasketError::ComponentMismatch);
        require_keys_eq!(g[3].key(), position.mint, BasketError::ComponentMismatch);

        let (vault_amount, vault_frozen) = read_token_account(&g[1])?;
        let user_frozen = if g[2].data_is_empty() {
            false
        } else {
            let (_, frozen) = read_token_account(&g[2])?;
            frozen
        };
        let available = vault_amount.saturating_sub(position.owed);
        out.push(ComponentAccounts {
            position,
            position_ai,
            vault: &g[1],
            user_ata: &g[2],
            mint: &g[3],
            vault_amount,
            available,
            vault_frozen,
            user_frozen,
        });
    }
    Ok(out)
}

/// `(amount, frozen)` of a Token or Token-2022 account.
pub fn read_token_account(ai: &AccountInfo) -> Result<(u64, bool)> {
    let data = ai.try_borrow_data()?;
    let acc = TokenAccount::try_deserialize(&mut &data[..])?;
    Ok((acc.amount, acc.state == spl_token_2022::state::AccountState::Frozen))
}

pub fn read_token_amount(ai: &AccountInfo) -> Result<u64> {
    Ok(read_token_account(ai)?.0)
}
