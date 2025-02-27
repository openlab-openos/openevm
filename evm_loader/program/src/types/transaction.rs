use ethnum::U256;
use maybe_async::maybe_async;
use rlp::{DecoderError, Rlp};
use serde::{Deserialize, Serialize};
use solana_program::instruction::{get_stack_height, TRANSACTION_LEVEL_STACK_HEIGHT};
use std::convert::TryInto;

use crate::account::TransactionTree;
use crate::types::vector::VectorVecExt;
use crate::{
    account_storage::AccountStorage, config::GAS_LIMIT_MULTIPLIER_NO_CHAINID, error::Error, vector,
};

use super::vector::VectorSliceExt;
use super::{Address, Vector};

use super::read_raw_utils::ReconstructRaw;
use crate::types::read_raw_utils::read_vec;
use evm_loader_macro::ReconstructRaw;

#[repr(transparent)]
#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct StorageKey([u8; 32]);

impl rlp::Decodable for StorageKey {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        rlp.decoder().decode_value(|bytes| {
            let array: [u8; 32] = bytes
                .try_into()
                .map_err(|_| rlp::DecoderError::RlpInvalidLength)?;
            Ok(Self(array))
        })
    }
}

impl TryFrom<Vec<u8>> for StorageKey {
    type Error = String;

    fn try_from(hex: Vec<u8>) -> Result<Self, Self::Error> {
        let bytes = hex;

        if bytes.len() != 32 {
            return Err(String::from("Hex string must be 32 bytes"));
        }

        let mut array = [0; 32];
        array.copy_from_slice(&bytes);

        Ok(StorageKey(array))
    }
}

impl AsRef<[u8]> for StorageKey {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionEnvelope {
    Legacy,
    AccessList,
    DynamicFee,
    Scheduled,
}

impl TransactionEnvelope {
    pub fn get_type(bytes: &[u8]) -> (Option<TransactionEnvelope>, &[u8]) {
        // Legacy transaction format
        if rlp::Rlp::new(bytes).is_list() {
            (None, bytes)
        // It's an EIP-2718 typed TX envelope.
        } else {
            match bytes[0] {
                0x00 => (Some(TransactionEnvelope::Legacy), &bytes[1..]),
                0x01 => (Some(TransactionEnvelope::AccessList), &bytes[1..]),
                0x02 => (Some(TransactionEnvelope::DynamicFee), &bytes[1..]),
                0x7f => {
                    let subtype = bytes[1];
                    if subtype == 0x01 {
                        (Some(TransactionEnvelope::Scheduled), &bytes[2..])
                    } else {
                        panic_with_error!(Error::UnsuppotedNeonTransactionType(subtype))
                    }
                }
                byte => panic_with_error!(Error::UnsuppotedEthereumTransactionType(byte)),
            }
        }
    }
}

#[derive(Debug, ReconstructRaw)]
#[repr(C)]
pub struct LegacyTx {
    pub nonce: u64,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub target: Option<Address>,
    pub value: U256,
    pub call_data: Vector<u8>,
    pub v: U256,
    pub r: U256,
    pub s: U256,
    pub chain_id: Option<U256>,
    pub recovery_id: u8,
}

impl rlp::Decodable for LegacyTx {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        let rlp_len = {
            let info = rlp.payload_info()?;
            info.header_len + info.value_len
        };

        if rlp.as_raw().len() != rlp_len {
            return Err(rlp::DecoderError::RlpInconsistentLengthAndData);
        }

        let nonce: u64 = rlp.val_at(0)?;
        let gas_price: U256 = u256(&rlp.at(1)?)?;
        let gas_limit: U256 = u256(&rlp.at(2)?)?;
        let target: Option<Address> = decode_optional_address(&rlp.at(3)?)?;
        let value: U256 = u256(&rlp.at(4)?)?;
        let call_data = decode_byte_vector(&rlp.at(5)?)?;
        let v: U256 = u256(&rlp.at(6)?)?;
        let r: U256 = u256(&rlp.at(7)?)?;
        let s: U256 = u256(&rlp.at(8)?)?;

        if rlp.at(9).is_ok() {
            return Err(rlp::DecoderError::RlpIncorrectListLen);
        }

        let (chain_id, recovery_id) = if v >= 35 {
            let chain_id = (v - 1) / 2 - 17;
            let recovery_id = u8::from((v % 2) == U256::ZERO);
            (Some(chain_id), recovery_id)
        } else if v == 27 {
            (None, 0_u8)
        } else if v == 28 {
            (None, 1_u8)
        } else {
            return Err(rlp::DecoderError::RlpExpectedToBeData);
        };

        let tx = LegacyTx {
            nonce,
            gas_price,
            gas_limit,
            target,
            value,
            call_data,
            v,
            r,
            s,
            chain_id,
            recovery_id,
        };

        Ok(tx)
    }
}

#[derive(Debug, ReconstructRaw)]
#[repr(C)]
pub struct AccessListTx {
    pub nonce: u64,
    pub gas_price: U256,
    pub gas_limit: U256,
    pub target: Option<Address>,
    pub value: U256,
    pub call_data: Vector<u8>,
    pub r: U256,
    pub s: U256,
    pub chain_id: U256,
    pub recovery_id: u8,
    pub access_list: Vector<(Address, Vector<StorageKey>)>,
}

impl rlp::Decodable for AccessListTx {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        let rlp_len = {
            let info = rlp.payload_info()?;
            info.header_len + info.value_len
        };

        if rlp.as_raw().len() != rlp_len {
            return Err(rlp::DecoderError::RlpInconsistentLengthAndData);
        }

        let chain_id: U256 = u256(&rlp.at(0)?)?;
        let nonce: u64 = rlp.val_at(1)?;
        let gas_price: U256 = u256(&rlp.at(2)?)?;
        let gas_limit: U256 = u256(&rlp.at(3)?)?;
        let target: Option<Address> = decode_optional_address(&rlp.at(4)?)?;

        let value: U256 = u256(&rlp.at(5)?)?;
        let call_data = decode_byte_vector(&rlp.at(6)?)?;

        let rlp_access_list = rlp.at(7)?;
        let mut access_list = vector![];

        for entry in &rlp_access_list {
            // Check if entry is a list
            if entry.is_list() {
                // Parse address from first element
                let address: Address = entry.at(0)?.as_val()?;

                // Get storage keys from second element
                let mut storage_keys: Vector<StorageKey> = vector![];

                for key in &entry.at(1)? {
                    storage_keys.push(key.as_val()?);
                }

                access_list.push((address, storage_keys));
            } else {
                return Err(rlp::DecoderError::RlpExpectedToBeList);
            }
        }

        let y_parity: u8 = rlp.at(8)?.as_val()?;
        let r: U256 = u256(&rlp.at(9)?)?;
        let s: U256 = u256(&rlp.at(10)?)?;

        if rlp.at(11).is_ok() {
            return Err(rlp::DecoderError::RlpIncorrectListLen);
        }

        let tx = AccessListTx {
            nonce,
            gas_price,
            gas_limit,
            target,
            value,
            call_data,
            r,
            s,
            chain_id,
            recovery_id: y_parity,
            access_list,
        };

        Ok(tx)
    }
}

#[derive(Debug, ReconstructRaw)]
#[repr(C)]
pub struct DynamicFeeTx {
    pub nonce: u64,
    pub max_priority_fee_per_gas: U256,
    pub max_fee_per_gas: U256,
    pub gas_limit: U256,
    pub target: Option<Address>,
    pub value: U256,
    pub call_data: Vector<u8>,
    pub r: U256,
    pub s: U256,
    pub chain_id: U256,
    pub recovery_id: u8,
    pub access_list: Vector<(Address, Vector<StorageKey>)>,
}

impl rlp::Decodable for DynamicFeeTx {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        let rlp_len = {
            let info = rlp.payload_info()?;
            info.header_len + info.value_len
        };

        if rlp.as_raw().len() != rlp_len {
            return Err(rlp::DecoderError::RlpInconsistentLengthAndData);
        }

        let chain_id: U256 = u256(&rlp.at(0)?)?;
        let nonce: u64 = rlp.val_at(1)?;

        let max_priority_fee_per_gas: U256 = u256(&rlp.at(2)?)?;
        let max_fee_per_gas: U256 = u256(&rlp.at(3)?)?;
        if max_fee_per_gas < max_priority_fee_per_gas {
            return Err(rlp::DecoderError::Custom(
                "max_fee_per_gas < max_priority_fee_per_gas",
            ));
        }

        let gas_limit: U256 = u256(&rlp.at(4)?)?;
        let target: Option<Address> = decode_optional_address(&rlp.at(5)?)?;

        let value: U256 = u256(&rlp.at(6)?)?;
        let call_data = decode_byte_vector(&rlp.at(7)?)?;

        let rlp_access_list = rlp.at(8)?;
        let mut access_list = vector![];

        for entry in &rlp_access_list {
            // Check if entry is a list
            if entry.is_list() {
                // Parse address from first element
                let address: Address = entry.at(0)?.as_val()?;

                // Get storage keys from second element
                let mut storage_keys: Vector<StorageKey> = vector![];

                for key in &entry.at(1)? {
                    storage_keys.push(key.as_val()?);
                }

                access_list.push((address, storage_keys));
            } else {
                return Err(rlp::DecoderError::RlpExpectedToBeList);
            }
        }

        let y_parity: u8 = rlp.at(9)?.as_val()?;
        let r: U256 = u256(&rlp.at(10)?)?;
        let s: U256 = u256(&rlp.at(11)?)?;

        if rlp.at(12).is_ok() {
            return Err(rlp::DecoderError::RlpIncorrectListLen);
        }

        let tx = DynamicFeeTx {
            nonce,
            max_priority_fee_per_gas,
            max_fee_per_gas,
            gas_limit,
            target,
            value,
            call_data,
            r,
            s,
            chain_id,
            recovery_id: y_parity,
            access_list,
        };

        Ok(tx)
    }
}

/// A "shell" representation of `ScheduledTx` without the persistent Vectors.
/// Intended for use in cases when there's no heap account.
/// TODO: rework the whole transaction to be able to use `ScheduledTx` when account heap is absent.
#[derive(Debug)]
#[repr(C)]
pub struct ScheduledTxShell {
    pub payer: Address,
    pub sender: Option<Address>,
    pub nonce: u64,
    pub index: u16,
    pub intent: Option<Address>,
    pub target: Option<Address>,
    pub value: U256,
    pub chain_id: U256,
    pub gas_limit: U256,
    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: U256,
    pub hash: [u8; 32],
}

impl ScheduledTxShell {
    pub fn from_rlp(message: &[u8]) -> crate::error::Result<Self> {
        use solana_program::keccak::hashv;

        let (tx_type, tx_body) = TransactionEnvelope::get_type(message);
        tx_type
            .map(|f| f == TransactionEnvelope::Scheduled)
            .ok_or(crate::error::Error::TreeAccountTxInvalidType)?;

        let rlp = rlp::Rlp::new(tx_body);
        let hash = hashv(&[&[0x7f, 0x01], tx_body]).to_bytes();
        ScheduledTxShell::decode(&rlp, hash).map_err(Error::from)
    }

    fn decode(rlp: &rlp::Rlp, hash: [u8; 32]) -> Result<Self, rlp::DecoderError> {
        let rlp_len = {
            let info = rlp.payload_info()?;
            info.header_len + info.value_len
        };

        if rlp.as_raw().len() != rlp_len {
            return Err(rlp::DecoderError::RlpInconsistentLengthAndData);
        }

        let payer: Address = rlp.at(0)?.as_val()?;
        let sender: Option<Address> = decode_optional_address(&rlp.at(1)?)?;

        let nonce: u64 = rlp.val_at(2)?;
        let index: u16 = rlp.val_at(3)?;

        let intent: Option<Address> = decode_optional_address(&rlp.at(4)?)?;
        // index 5 is skipped (intent_call_data).
        let target: Option<Address> = decode_optional_address(&rlp.at(6)?)?;
        // index 7 is skipped (call_data).

        let value: U256 = u256(&rlp.at(8)?)?;
        let chain_id: U256 = u256(&rlp.at(9)?)?;

        let gas_limit: U256 = u256(&rlp.at(10)?)?;
        let max_fee_per_gas: U256 = u256(&rlp.at(11)?)?;
        let max_priority_fee_per_gas: U256 = u256(&rlp.at(12)?)?;

        if max_fee_per_gas < max_priority_fee_per_gas {
            return Err(rlp::DecoderError::Custom(
                "max_fee_per_gas < max_priority_fee_per_gas",
            ));
        }

        if rlp.at(13).is_ok() {
            return Err(rlp::DecoderError::RlpIncorrectListLen);
        }

        let tx = ScheduledTxShell {
            payer,
            sender,
            nonce,
            index,
            intent,
            target,
            value,
            chain_id,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            hash,
        };

        Ok(tx)
    }
}

#[derive(Debug, ReconstructRaw)]
#[repr(C)]
pub struct ScheduledTx {
    pub payer: Address,
    pub sender: Option<Address>,
    pub nonce: u64,
    pub index: u16,
    pub intent: Option<Address>,
    pub intent_call_data: Vector<u8>,
    pub target: Option<Address>,
    pub call_data: Vector<u8>,
    pub value: U256,
    pub chain_id: U256,
    pub gas_limit: U256,
    pub max_fee_per_gas: U256,
    pub max_priority_fee_per_gas: U256,
}

// TODO remove if unused in the end. Possibly, the Transaction::hash() can be used instead.
impl ScheduledTx {
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        use solana_program::keccak::hashv;

        let rlp = rlp::encode(self);
        hashv(&[&[0x7f, 0x01], &rlp]).to_bytes()
    }
}

impl rlp::Encodable for ScheduledTx {
    fn rlp_append(&self, stream: &mut rlp::RlpStream) {
        // Only the body, tx_type is omitted (as in the decode).
        stream.begin_list(13);
        stream.append(&self.payer);
        stream.append(&self.sender);
        stream.append(&self.nonce);
        stream.append(&self.index);
        stream.append(&self.intent);
        stream.append(&self.intent_call_data.as_slice());
        stream.append(&self.target);
        stream.append(&self.call_data.as_slice());
        stream.append(&self.value.to_be_bytes().as_slice());
        stream.append(&self.chain_id.to_be_bytes().as_slice());
        stream.append(&self.gas_limit.to_be_bytes().as_slice());
        stream.append(&self.max_fee_per_gas.to_be_bytes().as_slice());
        stream.append(&self.max_priority_fee_per_gas.to_be_bytes().as_slice());
    }
}

impl rlp::Decodable for ScheduledTx {
    fn decode(rlp: &rlp::Rlp) -> Result<Self, rlp::DecoderError> {
        let rlp_len = {
            let info = rlp.payload_info()?;
            info.header_len + info.value_len
        };

        if rlp.as_raw().len() != rlp_len {
            return Err(rlp::DecoderError::RlpInconsistentLengthAndData);
        }

        let payer: Address = rlp.at(0)?.as_val()?;
        let sender: Option<Address> = decode_optional_address(&rlp.at(1)?)?;

        let nonce: u64 = rlp.val_at(2)?;
        let index: u16 = rlp.val_at(3)?;

        let intent: Option<Address> = decode_optional_address(&rlp.at(4)?)?;
        let intent_call_data: Vector<u8> = decode_byte_vector(&rlp.at(5)?)?;

        let target: Option<Address> = decode_optional_address(&rlp.at(6)?)?;
        let call_data = decode_byte_vector(&rlp.at(7)?)?;

        let value: U256 = u256(&rlp.at(8)?)?;
        let chain_id: U256 = u256(&rlp.at(9)?)?;

        let gas_limit: U256 = u256(&rlp.at(10)?)?;
        let max_fee_per_gas: U256 = u256(&rlp.at(11)?)?;
        let max_priority_fee_per_gas: U256 = u256(&rlp.at(12)?)?;

        if max_fee_per_gas < max_priority_fee_per_gas {
            return Err(rlp::DecoderError::Custom(
                "max_fee_per_gas < max_priority_fee_per_gas",
            ));
        }

        if rlp.at(13).is_ok() {
            return Err(rlp::DecoderError::RlpIncorrectListLen);
        }

        let tx = ScheduledTx {
            payer,
            sender,
            nonce,
            index,
            intent,
            intent_call_data,
            target,
            call_data,
            value,
            chain_id,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        };

        Ok(tx)
    }
}

#[derive(Debug)]
#[repr(C, u8)]
pub enum TransactionPayload {
    Legacy(LegacyTx),
    AccessList(AccessListTx),
    DynamicFee(DynamicFeeTx),
    Scheduled(ScheduledTx),
}

#[derive(Debug)]
#[repr(C)]
pub struct Transaction {
    pub transaction: TransactionPayload,
    pub byte_len: usize,
    pub hash: [u8; 32],
    pub signed_hash: [u8; 32],
}

impl Transaction {
    fn from_payload(
        transaction_type: &Option<TransactionEnvelope>,
        chain_id: Option<U256>,
        transaction_rlp: &rlp::Rlp,
        transaction: TransactionPayload,
    ) -> Result<Self, rlp::DecoderError> {
        use solana_program::keccak::{hash, hashv, Hash};

        let (hash, signed_hash) = match *transaction_type {
            // Legacy transaction wrapped in envelope
            Some(TransactionEnvelope::Legacy) => {
                let Hash(hash) = hashv(&[&[0x00], transaction_rlp.as_raw()]);
                let signed_hash = Self::calculate_legacy_signature(transaction_rlp, chain_id)?;

                (hash, signed_hash)
            }
            // Access List transaction
            Some(TransactionEnvelope::AccessList) => {
                let Hash(hash) = hashv(&[&[0x01], transaction_rlp.as_raw()]);
                let signed_hash = Self::eip2718_signed_hash(&[0x01], transaction_rlp, 8)?;

                (hash, signed_hash)
            }
            // Dynamic Fee transaction
            Some(TransactionEnvelope::DynamicFee) => {
                let Hash(hash) = hashv(&[&[0x02], transaction_rlp.as_raw()]);
                let signed_hash = Self::eip2718_signed_hash(&[0x02], transaction_rlp, 9)?;

                (hash, signed_hash)
            }
            // Scheduled transaction
            Some(TransactionEnvelope::Scheduled) => {
                let Hash(hash) = hashv(&[&[0x7f, 0x01], transaction_rlp.as_raw()]);
                (hash, [0_u8; 32])
            }
            // Legacy trasaction
            None => {
                let Hash(hash) = hash(transaction_rlp.as_raw());
                let signed_hash = Self::calculate_legacy_signature(transaction_rlp, chain_id)?;

                (hash, signed_hash)
            }
        };

        let info = transaction_rlp.payload_info()?;
        let byte_len = if transaction_type.is_none() {
            // Legacy transaction
            info.header_len + info.value_len
        } else {
            let subtype_present = match transaction_type.as_ref().unwrap() {
                TransactionEnvelope::Scheduled => 1,
                _ => 0,
            };

            // Transaction in the type envelope
            info.header_len + info.value_len + 1 + subtype_present // + 1 byte for type + 1 byte if subtype is used.
        };

        Ok(Transaction {
            transaction,
            byte_len,
            hash,
            signed_hash,
        })
    }

    fn eip2718_signed_hash(
        transaction_type: &[u8],
        transaction: &rlp::Rlp,
        middle_offset: usize,
    ) -> Result<[u8; 32], rlp::DecoderError> {
        let raw = transaction.as_raw();
        let payload_info = transaction.payload_info()?;
        let (_, middle_offset) = transaction.at_with_offset(middle_offset)?;

        let body = &raw[payload_info.header_len..middle_offset];

        let header: Vec<u8> = {
            let len = body.len();
            if len <= 55 {
                let len: u8 = len.try_into().unwrap();
                vec![0xC0 + len]
            } else {
                let len_bytes = {
                    let leading_empty_bytes = (len.leading_zeros() as usize) / 8;
                    let bytes = len.to_be_bytes();
                    bytes[leading_empty_bytes..].to_vec()
                };
                let len_bytes_len: u8 = len_bytes.len().try_into().unwrap();

                let mut header = Vec::with_capacity(10);
                header.extend_from_slice(&[0xF7 + len_bytes_len]);
                header.extend_from_slice(&len_bytes);

                header
            }
        };

        let hash = solana_program::keccak::hashv(&[transaction_type, &header, body]).to_bytes();

        Ok(hash)
    }

    fn calculate_legacy_signature(
        transaction: &rlp::Rlp,
        chain_id: Option<U256>,
    ) -> Result<[u8; 32], rlp::DecoderError> {
        let raw = transaction.as_raw();
        let payload_info = transaction.payload_info()?;
        let (_, v_offset) = transaction.at_with_offset(6)?;

        let middle = &raw[payload_info.header_len..v_offset];

        let trailer = chain_id.map_or_else(Vec::new, |chain_id| {
            let chain_id = {
                let leading_empty_bytes = (chain_id.leading_zeros() as usize) / 8;
                let bytes = chain_id.to_be_bytes();
                bytes[leading_empty_bytes..].to_vec()
            };

            let mut trailer = Vec::with_capacity(64);
            match chain_id.len() {
                0 => {
                    trailer.extend_from_slice(&[0x80]);
                }
                1 if chain_id[0] < 0x80 => {
                    trailer.extend_from_slice(&chain_id);
                }
                len @ 1..=55 => {
                    let len: u8 = len.try_into().unwrap();

                    trailer.extend_from_slice(&[0x80 + len]);
                    trailer.extend_from_slice(&chain_id);
                }
                _ => {
                    unreachable!("chain_id.len() <= 32")
                }
            }

            trailer.extend_from_slice(&[0x80, 0x80]);
            trailer
        });

        let header: Vec<u8> = {
            let len = middle.len() + trailer.len();
            if len <= 55 {
                let len: u8 = len.try_into().unwrap();
                vec![0xC0 + len]
            } else {
                let len_bytes = {
                    let leading_empty_bytes = (len.leading_zeros() as usize) / 8;
                    let bytes = len.to_be_bytes();
                    bytes[leading_empty_bytes..].to_vec()
                };
                let len_bytes_len: u8 = len_bytes.len().try_into().unwrap();

                let mut header = Vec::with_capacity(10);
                header.extend_from_slice(&[0xF7 + len_bytes_len]);
                header.extend_from_slice(&len_bytes);

                header
            }
        };

        let hash = solana_program::keccak::hashv(&[&header, middle, &trailer]).to_bytes();

        Ok(hash)
    }
}

impl Transaction {
    pub fn scheduled_from_rlp(transaction: &[u8]) -> Result<Self, Error> {
        let (transaction_type, transaction) = TransactionEnvelope::get_type(transaction);

        let tx = match transaction_type {
            Some(TransactionEnvelope::Scheduled) => {
                let scheduled_tx = rlp::decode::<ScheduledTx>(transaction).map_err(Error::from)?;
                let chain_id = scheduled_tx.chain_id;
                let tx = TransactionPayload::Scheduled(scheduled_tx);
                Transaction::from_payload(
                    &Some(TransactionEnvelope::Scheduled),
                    Some(chain_id),
                    &rlp::Rlp::new(transaction),
                    tx,
                )?
            }
            _ => {
                // Forbid constructing classic Eth transactions via code-path dedicated for scheduled txns.
                // Use `scheduled_from_rlp` instead.
                // N.B. Panic instead of error- usage would indicate a bug in the caller's code
                // (e.g. Neon Proxy) rather than an error.
                panic_with_error!(Error::NotScheduledTransaction);
            }
        };

        Ok(tx)
    }

    pub fn from_rlp(transaction: &[u8]) -> Result<Self, Error> {
        let (transaction_type, transaction) = TransactionEnvelope::get_type(transaction);

        let tx = match transaction_type {
            Some(TransactionEnvelope::Legacy) => {
                let legacy_tx = rlp::decode::<LegacyTx>(transaction).map_err(Error::from)?;
                let chain_id = legacy_tx.chain_id;
                let tx = TransactionPayload::Legacy(legacy_tx);
                Transaction::from_payload(
                    &Some(TransactionEnvelope::Legacy),
                    chain_id,
                    &rlp::Rlp::new(transaction),
                    tx,
                )?
            }
            Some(TransactionEnvelope::AccessList) => {
                let access_list_tx =
                    rlp::decode::<AccessListTx>(transaction).map_err(Error::from)?;
                let chain_id = access_list_tx.chain_id;
                let tx = TransactionPayload::AccessList(access_list_tx);
                Transaction::from_payload(
                    &Some(TransactionEnvelope::AccessList),
                    Some(chain_id),
                    &rlp::Rlp::new(transaction),
                    tx,
                )?
            }
            Some(TransactionEnvelope::DynamicFee) => {
                let dynamic_fee_tx =
                    rlp::decode::<DynamicFeeTx>(transaction).map_err(Error::from)?;
                let chain_id = dynamic_fee_tx.chain_id;
                let tx = TransactionPayload::DynamicFee(dynamic_fee_tx);
                Transaction::from_payload(
                    &Some(TransactionEnvelope::DynamicFee),
                    Some(chain_id),
                    &rlp::Rlp::new(transaction),
                    tx,
                )?
            }
            Some(TransactionEnvelope::Scheduled) => {
                // Forbid constructing ScheduledTx via `from_rlp`, so it doesn't interfere with the "classic"
                // Neon instructions and native Eth transactions.
                // Use `scheduled_from_rlp` instead.
                // N.B. Panic, instead of error, because usage would indicate a bug rather than an error.
                panic_with_error!(Error::NotClassicTransaction);
            }
            None => {
                let legacy_tx = rlp::decode::<LegacyTx>(transaction).map_err(Error::from)?;
                let chain_id = legacy_tx.chain_id;
                let tx = TransactionPayload::Legacy(legacy_tx);
                Transaction::from_payload(&None, chain_id, &rlp::Rlp::new(transaction), tx)?
            }
        };

        Ok(tx)
    }

    pub fn recover_caller_address(&self) -> Result<Address, Error> {
        use solana_program::keccak::{hash, Hash};
        use solana_program::secp256k1_recover::secp256k1_recover;

        let signature = [self.r().to_be_bytes(), self.s().to_be_bytes()].concat();
        let public_key = secp256k1_recover(&self.signed_hash(), self.recovery_id(), &signature)?;

        let Hash(address) = hash(&public_key.to_bytes());
        let address: [u8; 20] = address[12..32].try_into()?;

        Ok(Address::from(address))
    }

    #[must_use]
    pub fn nonce(&self) -> u64 {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { nonce, .. })
            | TransactionPayload::AccessList(AccessListTx { nonce, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { nonce, .. })
            | TransactionPayload::Scheduled(ScheduledTx { nonce, .. }) => nonce,
        }
    }

    #[must_use]
    pub fn gas_price(&self) -> U256 {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { gas_price, .. })
            | TransactionPayload::AccessList(AccessListTx { gas_price, .. }) => gas_price,
            TransactionPayload::DynamicFee(DynamicFeeTx {
                max_priority_fee_per_gas,
                max_fee_per_gas,
                ..
            })
            | TransactionPayload::Scheduled(ScheduledTx {
                max_priority_fee_per_gas,
                max_fee_per_gas,
                ..
            }) => {
                // If a priority fee is defined, use it to pay the operator the base cost
                //  for details see priority_fee_txn_calculator.rs
                if max_priority_fee_per_gas == U256::ZERO {
                    // if there is no priority fee, use the whole max-fee-per-gas
                    max_fee_per_gas
                } else {
                    max_priority_fee_per_gas
                }
            }
        }
    }

    #[must_use]
    pub fn gas_limit(&self) -> U256 {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { gas_limit, .. })
            | TransactionPayload::AccessList(AccessListTx { gas_limit, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { gas_limit, .. })
            | TransactionPayload::Scheduled(ScheduledTx { gas_limit, .. }) => gas_limit,
        }
    }

    pub fn gas_limit_in_tokens(&self) -> Result<U256, Error> {
        self.gas_price()
            .checked_mul(self.gas_limit())
            .ok_or(Error::IntegerOverflow)
    }

    pub fn priority_fee_limit_in_tokens(&self) -> Result<U256, Error> {
        self.base_fee_per_gas()
            .unwrap_or_default()
            .checked_mul(self.gas_limit())
            .ok_or(Error::IntegerOverflow)
    }

    #[must_use]
    pub fn payer(&self, origin: Address) -> Address {
        match self.transaction {
            TransactionPayload::Legacy(_)
            | TransactionPayload::AccessList(_)
            | TransactionPayload::DynamicFee(_) => origin,
            TransactionPayload::Scheduled(ScheduledTx { payer, .. }) => payer,
        }
    }

    #[must_use]
    pub fn target(&self) -> Option<Address> {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { target, .. })
            | TransactionPayload::AccessList(AccessListTx { target, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { target, .. })
            | TransactionPayload::Scheduled(ScheduledTx { target, .. }) => target,
        }
    }

    #[must_use]
    pub fn value(&self) -> U256 {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { value, .. })
            | TransactionPayload::AccessList(AccessListTx { value, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { value, .. })
            | TransactionPayload::Scheduled(ScheduledTx { value, .. }) => value,
        }
    }

    #[must_use]
    pub fn call_data(&self) -> &[u8] {
        match &self.transaction {
            TransactionPayload::Legacy(LegacyTx { call_data, .. })
            | TransactionPayload::AccessList(AccessListTx { call_data, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { call_data, .. })
            | TransactionPayload::Scheduled(ScheduledTx { call_data, .. }) => call_data,
        }
    }

    #[must_use]
    pub fn r(&self) -> U256 {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { r, .. })
            | TransactionPayload::AccessList(AccessListTx { r, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { r, .. }) => r,
            TransactionPayload::Scheduled(_) => unreachable!(),
        }
    }

    #[must_use]
    pub fn s(&self) -> U256 {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { s, .. })
            | TransactionPayload::AccessList(AccessListTx { s, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { s, .. }) => s,
            TransactionPayload::Scheduled(_) => unreachable!(),
        }
    }

    #[must_use]
    pub fn chain_id(&self) -> Option<u64> {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { chain_id, .. }) => chain_id,
            TransactionPayload::AccessList(AccessListTx { chain_id, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { chain_id, .. })
            | TransactionPayload::Scheduled(ScheduledTx { chain_id, .. }) => Some(chain_id),
        }
        .map(std::convert::TryInto::try_into)
        .transpose()
        .expect("chain_id < u64::max")
    }

    #[must_use]
    pub fn recovery_id(&self) -> u8 {
        match self.transaction {
            TransactionPayload::Legacy(LegacyTx { recovery_id, .. })
            | TransactionPayload::AccessList(AccessListTx { recovery_id, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { recovery_id, .. }) => recovery_id,
            TransactionPayload::Scheduled(_) => unreachable!(),
        }
    }

    #[must_use]
    pub fn rlp_len(&self) -> usize {
        self.byte_len
    }

    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }

    #[must_use]
    pub fn signed_hash(&self) -> [u8; 32] {
        self.signed_hash
    }

    #[must_use]
    pub fn tx_type(&self) -> u8 {
        match self.transaction {
            TransactionPayload::Legacy(_) => 0,
            TransactionPayload::AccessList(_) => 1,
            TransactionPayload::DynamicFee(_) => 2,
            TransactionPayload::Scheduled(_) => 0x80, // 0x7f (max envelope tx type) + 0x01 (scheduled tx subtype)
        }
    }

    #[must_use]
    pub fn is_scheduled_tx(&self) -> bool {
        if let TransactionPayload::Scheduled(_) = self.transaction {
            return true;
        }
        false
    }

    #[must_use]
    pub fn max_fee_per_gas(&self) -> Option<U256> {
        match self.transaction {
            TransactionPayload::Legacy(_) | TransactionPayload::AccessList(_) => None,
            TransactionPayload::DynamicFee(DynamicFeeTx {
                max_fee_per_gas, ..
            })
            | TransactionPayload::Scheduled(ScheduledTx {
                max_fee_per_gas, ..
            }) => Some(max_fee_per_gas),
        }
    }

    #[must_use]
    pub fn max_priority_fee_per_gas(&self) -> Option<U256> {
        match self.transaction {
            TransactionPayload::Legacy(_) | TransactionPayload::AccessList(_) => None,
            TransactionPayload::DynamicFee(DynamicFeeTx {
                max_priority_fee_per_gas,
                ..
            })
            | TransactionPayload::Scheduled(ScheduledTx {
                max_priority_fee_per_gas,
                ..
            }) => Some(max_priority_fee_per_gas),
        }
    }

    #[must_use]
    pub fn base_fee_per_gas(&self) -> Option<U256> {
        match self.transaction {
            TransactionPayload::Legacy(_) | TransactionPayload::AccessList(_) => None,
            TransactionPayload::DynamicFee(DynamicFeeTx {
                max_priority_fee_per_gas,
                max_fee_per_gas,
                ..
            })
            | TransactionPayload::Scheduled(ScheduledTx {
                max_priority_fee_per_gas,
                max_fee_per_gas,
                ..
            }) => {
                if max_priority_fee_per_gas == U256::ZERO
                    || max_fee_per_gas == max_priority_fee_per_gas
                {
                    None
                } else {
                    Some(max_fee_per_gas.saturating_sub(max_priority_fee_per_gas))
                }
            }
        }
    }

    #[must_use]
    pub fn access_list(&self) -> Option<&Vector<(Address, Vector<StorageKey>)>> {
        match &self.transaction {
            TransactionPayload::AccessList(AccessListTx { access_list, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { access_list, .. }) => Some(access_list),
            TransactionPayload::Legacy(_) | TransactionPayload::Scheduled(_) => None,
        }
    }

    #[must_use]
    pub fn if_scheduled(&self) -> Option<&ScheduledTx> {
        match &self.transaction {
            TransactionPayload::AccessList(_)
            | TransactionPayload::DynamicFee(_)
            | TransactionPayload::Legacy(_) => None,
            TransactionPayload::Scheduled(ref scheduled) => Some(scheduled),
        }
    }

    #[must_use]
    pub fn tree_account_index(&self) -> Option<u16> {
        match &self.transaction {
            TransactionPayload::AccessList(_)
            | TransactionPayload::DynamicFee(_)
            | TransactionPayload::Legacy(_) => None,
            TransactionPayload::Scheduled(ScheduledTx { index, .. }) => Some(*index),
        }
    }

    pub fn use_gas_limit_multiplier(&mut self) {
        let gas_multiplier = U256::from(GAS_LIMIT_MULTIPLIER_NO_CHAINID);

        match &mut self.transaction {
            TransactionPayload::AccessList(AccessListTx { gas_limit, .. })
            | TransactionPayload::DynamicFee(DynamicFeeTx { gas_limit, .. })
            | TransactionPayload::Scheduled(ScheduledTx { gas_limit, .. })
            | TransactionPayload::Legacy(LegacyTx { gas_limit, .. }) => {
                *gas_limit = gas_limit.saturating_mul(gas_multiplier);
            }
        }
    }

    #[maybe_async]
    pub async fn validate(
        &self,
        origin: Address,
        backend: &impl AccountStorage,
        tree: Option<&TransactionTree<'_>>,
    ) -> Result<(), crate::error::Error> {
        let chain_id = self
            .chain_id()
            .unwrap_or_else(|| backend.default_chain_id());

        if !backend.is_valid_chain_id(chain_id) {
            return Err(Error::InvalidChainId(chain_id));
        }

        if tree.is_some() != self.is_scheduled_tx() {
            return Err(Error::TreeAccountTxInvalidType);
        }

        // Nonce validation is slightly different for classic and scheduled transactions.
        //
        // Classic transactions:
        // origin's nonce should be equal to txn's nonce because it's validated during
        // the first iteration and then incremented.
        //
        // Scheduled transactions:
        // payer's nonce (origin) validated only for the first transaction in the tree
        let origin_nonce = backend.nonce(origin, chain_id).await;

        let validate_nonce = tree.map_or(true, TransactionTree::is_not_started);
        if validate_nonce && (origin_nonce != self.nonce()) {
            let error = Error::InvalidTransactionNonce(origin, origin_nonce, self.nonce());
            return Err(error);
        }

        // The reason to forbid the calls for DynamicFee transactions - priority fee calculation
        // uses get_processed_sibling_instruction syscall which doesn't work well for CPI.
        let is_root_transaction = get_stack_height() == TRANSACTION_LEVEL_STACK_HEIGHT;
        if matches!(self.tx_type(), 2 | 0x80) && !is_root_transaction {
            return Err(
                "CPI calls of Neon EVM are forbidden for DynamicFee transaction type.".into(),
            );
        }

        Ok(())
    }
}

#[inline]
fn decode_byte_vector(rlp: &Rlp) -> Result<Vector<u8>, DecoderError> {
    rlp.decoder().decode_value(|bytes| Ok(bytes.to_vector()))
}

#[inline]
fn decode_optional_address(rlp: &Rlp) -> Result<Option<Address>, DecoderError> {
    if rlp.is_empty() {
        if rlp.is_data() {
            Ok(None)
        } else {
            Err(rlp::DecoderError::RlpExpectedToBeData)
        }
    } else {
        Ok(Some(rlp.as_val()?))
    }
}

#[inline]
fn u256(rlp: &rlp::Rlp) -> Result<U256, rlp::DecoderError> {
    rlp.decoder().decode_value(|bytes| {
        if !bytes.is_empty() && bytes[0] == 0 {
            Err(rlp::DecoderError::RlpInvalidIndirection)
        } else if bytes.len() <= 32 {
            let mut buffer = [0_u8; 32];
            buffer[(32 - bytes.len())..].copy_from_slice(bytes);
            Ok(U256::from_be_bytes(buffer))
        } else {
            Err(rlp::DecoderError::RlpIsTooBig)
        }
    })
}
