//! Harness for the protocol vaults: `init_treasury_vaults`, `set_bskt_mint`,
//! `stage_buyback` / `execute_buyback`, `post_prize_payout`.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::{InstructionData, ToAccountMetas};
use anchor_spl::token::spl_token;
use litesvm::types::{FailedTransactionMetadata, TransactionMetadata};
use solana_signer::Signer;

use basket::state::*;

use super::*;

impl Env {
    pub fn init_treasury_vaults(&mut self) {
        let ix = Instruction {
            program_id: basket::id(),
            accounts: basket::accounts::InitTreasuryVaults {
                admin: self.admin.pubkey(),
                config: config_pda(),
                buyback_vault: buyback_vault_pda(),
                prize_vault: prize_vault_pda(),
                system_program: anchor_lang::system_program::ID,
            }
            .to_account_metas(None),
            data: basket::instruction::InitTreasuryVaults {}.data(),
        };
        let admin = self.admin.insecure_clone();
        self.send_ok(&[ix], &admin, &[]);
    }

    pub fn buyback_vault(&self) -> BuybackVault {
        self.load(&buyback_vault_pda())
    }

    pub fn prize_vault(&self) -> PrizeVault {
        self.load(&prize_vault_pda())
    }

    pub fn set_bskt_mint(&mut self, mint: Pubkey) -> std::result::Result<TransactionMetadata, FailedTransactionMetadata> {
        let ix = self.admin_ix(basket::instruction::SetBsktMint { mint }.data());
        let admin = self.admin.insecure_clone();
        self.send(&[ix], &admin, &[])
    }

    pub fn stage_buyback_ix(&self, keeper: &Pubkey, lamports: u64) -> Instruction {
        Instruction {
            program_id: basket::id(),
            accounts: basket::accounts::StageBuyback {
                keeper: *keeper,
                config: config_pda(),
                buyback_vault: buyback_vault_pda(),
                buyback_wsol_ata: Env::ata(&buyback_vault_pda(), &WSOL),
                wsol_mint: WSOL,
                token_program: spl_token::ID,
            }
            .to_account_metas(None),
            data: basket::instruction::StageBuyback { lamports }.data(),
        }
    }

    /// `execute_buyback` wrapping an inner venue swap (wSOL → $BSKT) with the
    /// BUYBACK PDA as user.
    pub fn execute_buyback_ix(&self, keeper: &Pubkey, bskt_mint: &Pubkey, amount_in: u64, min_amount_out: u64, inner: &Instruction) -> Instruction {
        let vault = buyback_vault_pda();
        let mut accounts = basket::accounts::ExecuteBuyback {
            keeper: *keeper,
            config: config_pda(),
            buyback_vault: vault,
            buyback_wsol_ata: Env::ata(&vault, &WSOL),
            wsol_mint: WSOL,
            bskt_mint: *bskt_mint,
            buyback_bskt_ata: Env::ata(&vault, bskt_mint),
            swap_program: inner.program_id,
            token_program: spl_token::ID,
            bskt_token_program: spl_token::ID,
            associated_token_program: anchor_spl::associated_token::ID,
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None);
        accounts.extend(inner.accounts.iter().cloned().map(|mut m| {
            if m.pubkey == vault {
                m.is_signer = false;
            }
            m
        }));
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::ExecuteBuyback { amount_in, min_amount_out, data: inner.data.clone() }.data(),
        }
    }

    /// `post_prize_payout` for `recipients` = `(creator wallet, share_mint)`
    /// in rank order.
    pub fn post_prize_payout_ix(&self, keeper: &Pubkey, epoch: u64, total_lamports: u64, recipients: &[(Pubkey, Pubkey)]) -> Instruction {
        let mut accounts = basket::accounts::PostPrizePayout {
            keeper: *keeper,
            config: config_pda(),
            prize_vault: prize_vault_pda(),
        }
        .to_account_metas(None);
        for (wallet, share_mint) in recipients {
            accounts.push(AccountMeta::new(*wallet, false));
            accounts.push(AccountMeta::new_readonly(creator_lock_pda(share_mint), false));
            accounts.push(AccountMeta::new_readonly(basket_pda(share_mint), false));
        }
        Instruction {
            program_id: basket::id(),
            accounts,
            data: basket::instruction::PostPrizePayout { epoch, total_lamports }.data(),
        }
    }
}
