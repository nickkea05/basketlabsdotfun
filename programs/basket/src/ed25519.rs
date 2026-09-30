//! Ed25519 attestation check by instruction introspection (D10 in the
//! change order: a keeper-signed creator tier passed to `create_basket`).
//!
//! The transaction carries an `Ed25519Program` instruction immediately before
//! the instruction that consumes it. The precompile has already verified the
//! signature by the time we run; what we check is *what* it verified: one
//! signature, by the expected signer, over exactly the expected bytes.
//!
//! The message may be embedded in the precompile instruction or referenced
//! from another instruction's data.
use anchor_lang::prelude::*;
use solana_instructions_sysvar::{load_current_index_checked, load_instruction_at_checked};
use solana_sdk_ids::ed25519_program;

use crate::error::BasketError;

const OFFSETS_START: usize = 2;
const OFFSETS_LEN: usize = 14;
const SIGNATURE_LEN: usize = 64;
const PUBKEY_LEN: usize = 32;

/// Verify that the instruction right before the current one is an ed25519
/// precompile call with a single signature by `expected_signer` over
/// `expected_message`.
pub fn verify_ed25519_signature(
    instructions_sysvar: &AccountInfo,
    expected_signer: &Pubkey,
    expected_message: &[u8],
) -> Result<()> {
    let current = load_current_index_checked(instructions_sysvar)? as usize;
    require!(current > 0, BasketError::BadSignature);
    let ed = load_instruction_at_checked(current - 1, instructions_sysvar)?;
    require_keys_eq!(ed.program_id, ed25519_program::ID, BasketError::BadSignature);
    require!(ed.accounts.is_empty(), BasketError::BadSignature);

    let data = &ed.data;
    require!(data.len() >= OFFSETS_START + OFFSETS_LEN, BasketError::BadSignature);
    require!(data[0] == 1, BasketError::BadSignature);

    let o = &data[OFFSETS_START..OFFSETS_START + OFFSETS_LEN];
    let u16_at = |i: usize| u16::from_le_bytes([o[i], o[i + 1]]);
    let signature_offset = u16_at(0) as usize;
    let signature_ix = u16_at(2);
    let pubkey_offset = u16_at(4) as usize;
    let pubkey_ix = u16_at(6);
    let message_offset = u16_at(8) as usize;
    let message_size = u16_at(10) as usize;
    let message_ix = u16_at(12);

    // Signature and pubkey must live in the precompile instruction itself.
    require!(signature_ix == u16::MAX, BasketError::BadSignature);
    require!(pubkey_ix == u16::MAX, BasketError::BadSignature);
    require!(
        signature_offset + SIGNATURE_LEN <= data.len() && pubkey_offset + PUBKEY_LEN <= data.len(),
        BasketError::BadSignature
    );
    let signer = Pubkey::try_from(&data[pubkey_offset..pubkey_offset + PUBKEY_LEN])
        .map_err(|_| error!(BasketError::BadSignature))?;
    require_keys_eq!(signer, *expected_signer, BasketError::BadSignature);

    // The message: embedded (u16::MAX) or referenced from another instruction.
    require!(message_size == expected_message.len(), BasketError::BadSignature);
    let message: Vec<u8> = if message_ix == u16::MAX {
        require!(message_offset + message_size <= data.len(), BasketError::BadSignature);
        data[message_offset..message_offset + message_size].to_vec()
    } else {
        let referenced = load_instruction_at_checked(message_ix as usize, instructions_sysvar)?;
        require!(
            message_offset + message_size <= referenced.data.len(),
            BasketError::BadSignature
        );
        referenced.data[message_offset..message_offset + message_size].to_vec()
    };
    require!(message.as_slice() == expected_message, BasketError::BadSignature);
    Ok(())
}

/// Build the precompile instruction for tests and clients: signature and
/// pubkey embedded, message referenced from instruction `message_ix` at
/// `message_offset` (`8` for an Anchor instruction's first argument).
pub fn ed25519_instruction_referencing(
    pubkey: &[u8; 32],
    signature: &[u8; 64],
    message_ix: u16,
    message_offset: u16,
    message_size: u16,
) -> anchor_lang::solana_program::instruction::Instruction {
    let mut data = Vec::with_capacity(OFFSETS_START + OFFSETS_LEN + PUBKEY_LEN + SIGNATURE_LEN);
    data.push(1); // num signatures
    data.push(0); // padding
    let pubkey_offset = (OFFSETS_START + OFFSETS_LEN) as u16;
    let signature_offset = pubkey_offset + PUBKEY_LEN as u16;
    data.extend_from_slice(&signature_offset.to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&pubkey_offset.to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&message_offset.to_le_bytes());
    data.extend_from_slice(&message_size.to_le_bytes());
    data.extend_from_slice(&message_ix.to_le_bytes());
    data.extend_from_slice(pubkey);
    data.extend_from_slice(signature);
    anchor_lang::solana_program::instruction::Instruction {
        program_id: ed25519_program::ID,
        accounts: vec![],
        data,
    }
}

/// Build the precompile instruction with signature, pubkey and message all
/// embedded (small messages such as the tier attestation).
pub fn ed25519_instruction_embedded(
    pubkey: &[u8; 32],
    signature: &[u8; 64],
    message: &[u8],
) -> anchor_lang::solana_program::instruction::Instruction {
    let mut data = Vec::with_capacity(OFFSETS_START + OFFSETS_LEN + PUBKEY_LEN + SIGNATURE_LEN + message.len());
    data.push(1);
    data.push(0);
    let pubkey_offset = (OFFSETS_START + OFFSETS_LEN) as u16;
    let signature_offset = pubkey_offset + PUBKEY_LEN as u16;
    let message_offset = signature_offset + SIGNATURE_LEN as u16;
    data.extend_from_slice(&signature_offset.to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&pubkey_offset.to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(&message_offset.to_le_bytes());
    data.extend_from_slice(&(message.len() as u16).to_le_bytes());
    data.extend_from_slice(&u16::MAX.to_le_bytes());
    data.extend_from_slice(pubkey);
    data.extend_from_slice(signature);
    data.extend_from_slice(message);
    anchor_lang::solana_program::instruction::Instruction {
        program_id: ed25519_program::ID,
        accounts: vec![],
        data,
    }
}