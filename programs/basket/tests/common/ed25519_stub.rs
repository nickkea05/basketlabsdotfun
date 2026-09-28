//! In-process stand-in for the ed25519 precompile.
//!
//! Registered as a builtin at `Ed25519SigVerify111111111111111111111111111`.
//! It parses the precompile instruction format (count, 14-byte offsets,
//! signature / pubkey / message either inline or referenced from another
//! instruction in the transaction) and verifies each signature with
//! ed25519-dalek, failing the transaction exactly where the real precompile
//! would. Referenced instructions are read from the instructions sysvar
//! account, which every `create_basket` transaction carries anyway.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use solana_account::ReadableAccount;
use solana_program_runtime::declare_process_instruction;
use solana_program_runtime::invoke_context::InvokeContext;
pub use solana_program_runtime::solana_sbpf::program::BuiltinFunctionDefinition;
use solana_program_runtime::__private::InstructionError;

pub const ED25519_ID: anchor_lang::prelude::Pubkey = solana_sdk_ids::ed25519_program::ID;
const SYSVAR_INSTRUCTIONS: [u8; 32] = solana_sdk_ids::sysvar::instructions::ID.to_bytes();

// 3000 CU mirrors the real precompile's per-signature cost (the runtime also
// rejects builtins that consume zero compute units).
declare_process_instruction!(Ed25519Stub, 3000, |invoke_context| { verify(invoke_context) });

fn verify(invoke_context: &mut InvokeContext) -> Result<(), InstructionError> {
    let tc = &invoke_context.transaction_context;
    let ic = tc.get_current_instruction_context()?;
    let data = ic.get_instruction_data().to_vec();

    // All top-level instruction datas, from the sysvar (if present).
    let mut all_datas: Vec<Vec<u8>> = Vec::new();
    for i in 0..tc.get_number_of_accounts() {
        let key = tc.get_key_of_account_at_index(i)?;
        if key.to_bytes() == SYSVAR_INSTRUCTIONS {
            let acc = tc.accounts().try_borrow(i)?;
            all_datas = parse_sysvar_instruction_datas(acc.data());
            break;
        }
    }

    let fail = || InstructionError::Custom(1);
    if data.len() < 2 {
        return Err(fail());
    }
    let count = data[0] as usize;
    if count == 0 || data.len() < 2 + count * 14 {
        return Err(fail());
    }
    for n in 0..count {
        let o = &data[2 + n * 14..2 + (n + 1) * 14];
        let u16_at = |i: usize| u16::from_le_bytes([o[i], o[i + 1]]);
        let sig = slice(&data, &all_datas, u16_at(2), u16_at(0) as usize, 64).ok_or_else(fail)?;
        let pk = slice(&data, &all_datas, u16_at(6), u16_at(4) as usize, 32).ok_or_else(fail)?;
        let msg = slice(&data, &all_datas, u16_at(12), u16_at(8) as usize, u16_at(10) as usize).ok_or_else(fail)?;
        let vk = VerifyingKey::from_bytes(pk[..].try_into().unwrap()).map_err(|_| fail())?;
        let signature = Signature::from_bytes(sig[..].try_into().unwrap());
        vk.verify(&msg, &signature).map_err(|_| fail())?;
    }
    Ok(())
}

fn slice(own: &[u8], all: &[Vec<u8>], ix: u16, offset: usize, len: usize) -> Option<Vec<u8>> {
    let src: &[u8] = if ix == u16::MAX { own } else { all.get(ix as usize)?.as_slice() };
    src.get(offset..offset + len).map(|s| s.to_vec())
}

/// Instructions sysvar layout: u16 count, u16 offsets[count], then per
/// instruction: u16 num_accounts, (u8 meta, 32 pubkey)*, 32 program_id,
/// u16 data_len, data. Trailing u16 current index.
fn parse_sysvar_instruction_datas(data: &[u8]) -> Vec<Vec<u8>> {
    let rd16 = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]) as usize;
    if data.len() < 2 {
        return vec![];
    }
    let count = rd16(0);
    let mut out = Vec::with_capacity(count);
    for n in 0..count {
        let mut p = rd16(2 + n * 2);
        let num_accounts = rd16(p);
        p += 2 + num_accounts * 33 + 32;
        let len = rd16(p);
        p += 2;
        out.push(data[p..p + len].to_vec());
    }
    out
}
