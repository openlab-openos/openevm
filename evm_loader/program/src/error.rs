//! Error types
#![allow(clippy::use_self)]

use crate::account::InterruptedState;
use crate::allocator::acc_allocator;
use crate::debug::log_data;
use crate::types::{Address, Vector};
use ethnum::U256;
use solana_program::{
    program_error::ProgramError,
    pubkey::{Pubkey, PubkeyError},
    secp256k1_recover::Secp256k1RecoverError,
};
use std::{array::TryFromSliceError, num::TryFromIntError, str::Utf8Error};
use thiserror::Error;

mod as_display_string {
    use std::fmt::Display;

    use serde::ser::Serializer;

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Display,
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }
}

/// Errors that may be returned by the EVM Loader program.
#[derive(Error, Debug, strum::EnumDiscriminants, serde::Serialize)]
pub enum Error {
    #[error("Error: {0}")]
    Custom(String),

    #[error("Solana Program Error: {0}")]
    ProgramError(
        #[from]
        #[serde(with = "as_display_string")]
        ProgramError,
    ),

    #[error("Solana Pubkey Error: {0}")]
    PubkeyError(
        #[from]
        #[serde(with = "as_display_string")]
        PubkeyError,
    ),

    #[error("RLP error: {0}")]
    RlpError(
        #[from]
        #[serde(with = "as_display_string")]
        rlp::DecoderError,
    ),

    #[error("Secp256k1 error: {0}")]
    Secp256k1Error(
        #[from]
        #[serde(with = "as_display_string")]
        Secp256k1RecoverError,
    ),

    #[error("Bincode error: {0}")]
    BincodeError(
        #[from]
        #[serde(with = "as_display_string")]
        bincode::Error,
    ),

    #[error("IO error: {0}")]
    BorshError(
        #[from]
        #[serde(with = "as_display_string")]
        std::io::Error,
    ),

    #[error("FromHexError error: {0}")]
    FromHexError(
        #[from]
        #[serde(with = "as_display_string")]
        hex::FromHexError,
    ),

    #[error("TryFromIntError error: {0}")]
    TryFromIntError(
        #[from]
        #[serde(with = "as_display_string")]
        TryFromIntError,
    ),

    #[error("TryFromSliceError error: {0}")]
    TryFromSliceError(
        #[from]
        #[serde(with = "as_display_string")]
        TryFromSliceError,
    ),

    #[error("Utf8Error error: {0}")]
    Utf8Error(
        #[from]
        #[serde(with = "as_display_string")]
        Utf8Error,
    ),

    #[error("Account {0} - not found")]
    AccountMissing(Pubkey),

    #[error("Account {0} - blocked, trying to execute transaction on rw locked account")]
    AccountBlocked(Pubkey),

    #[error("Account {0} - was empty, created by another transaction")]
    AccountCreatedByAnotherTransaction(Pubkey),

    #[error("Account {0} - invalid tag, expected {1}")]
    AccountInvalidTag(Pubkey, u8),

    #[error("Account {0} - invalid owner, expected {1}")]
    AccountInvalidOwner(Pubkey, Pubkey),

    #[error("Account {0} - invalid public key, expected {1}")]
    AccountInvalidKey(Pubkey, Pubkey),

    #[error("Account {0} - invalid data")]
    AccountInvalidData(Pubkey),

    #[error("Account {0} - not writable")]
    AccountNotWritable(Pubkey),

    #[error("Account {0} - not signer")]
    AccountNotSigner(Pubkey),

    #[error("Account {0} - not rent exempt")]
    AccountNotRentExempt(Pubkey),

    #[error("Account {0} - already initialized")]
    AccountAlreadyInitialized(Pubkey),

    #[error("Account {0} - in legacy format")]
    AccountLegacy(Pubkey),

    #[error("Operator is not authorized")]
    UnauthorizedOperator,

    #[error("Storage Account is uninitialized")]
    StorageAccountUninitialized,

    #[error("Transaction already finalized")]
    StorageAccountFinalized,

    #[error("Storage Account {0} has invalid tag, actual {1}")]
    StorageAccountInvalidTag(Pubkey, u8),

    #[error("Unknown extension method selector {1:?}, contract {0}")]
    UnknownPrecompileMethodSelector(Address, [u8; 4]),

    #[error("Insufficient balance for transfer, account = {0}, chain = {1}, required = {2}")]
    InsufficientBalance(
        Address,
        u64,
        #[serde(with = "ethnum::serde::bytes::le")] U256,
    ),

    #[error("Invalid token for transfer, account = {0}, chain = {1}")]
    InvalidTransferToken(Address, u64),

    #[error("Out of Gas, limit = {0}, required = {1}")]
    OutOfGas(
        #[serde(with = "ethnum::serde::bytes::le")] U256,
        #[serde(with = "ethnum::serde::bytes::le")] U256,
    ),

    #[error("Out of Priority Fee, limit = {0}, required = {1}")]
    OutOfPriorityFee(
        #[serde(with = "ethnum::serde::bytes::le")] U256,
        #[serde(with = "ethnum::serde::bytes::le")] U256,
    ),

    #[error("Invalid gas balance account")]
    GasReceiverInvalidChainId,

    #[error("EVM Stack Overflow")]
    StackOverflow,

    #[error("EVM Stack Underflow")]
    StackUnderflow,

    #[error("EVM Push opcode out of bounds, contract = {0}")]
    PushOutOfBounds(Address),

    #[error("EVM Memory Access at offset = {0} with length = {1} is out of limits")]
    MemoryAccessOutOfLimits(usize, usize),

    #[error("EVM RETURNDATACOPY offset = {0} with length = {1} exceeds data size")]
    ReturnDataCopyOverflow(usize, usize),

    #[error("EVM static mode violation, contract = {0}")]
    StaticModeViolation(Address),

    #[error("EVM invalid jump destination = {1}, contract = {0}")]
    InvalidJump(Address, usize),

    #[error("EVM encountered invalid opcode, contract = {0}, opcode = {1:X}")]
    InvalidOpcode(Address, u8),

    #[error("EVM encountered unknown opcode, contract = {0}, opcode = {1:X}")]
    UnknownOpcode(Address, u8),

    #[error("Account {0} - nonce overflow")]
    NonceOverflow(Address),

    #[error("Invalid Nonce, origin {0} nonce {1} != Transaction nonce {2}")]
    InvalidTransactionNonce(Address, u64, u64),

    #[error("Invalid Chain ID {0}")]
    InvalidChainId(u64),

    #[error("Attempt to deploy to existing account {0}, caller = {1}")]
    DeployToExistingAccount(Address, Address),

    #[error("New contract code starting with the 0xEF byte (EIP-3541), contract = {0}")]
    EVMObjectFormatNotSupported(Address),

    #[error("New contract code size exceeds 24kb (EIP-170), contract = {0}, size = {1}")]
    ContractCodeSizeLimit(Address, usize),

    #[error("Transaction is rejected from a sender with deployed code (EIP-3607), contract = {0}")]
    SenderHasDeployedCode(Address),

    #[error("Checked Integer Math Overflow")]
    IntegerOverflow,

    #[error("Index out of bounds")]
    OutOfBounds,

    #[error("Holder Account - invalid owner {0}, expected = {1}")]
    HolderInvalidOwner(Pubkey, Pubkey),

    #[error("Holder Account - insufficient size {0}, required = {1}")]
    HolderInsufficientSize(usize, usize),

    #[error("Holder Account - invalid transaction hash {}, expected = {}", hex::encode(.0), hex::encode(.1))]
    HolderInvalidHash([u8; 32], [u8; 32]),

    #[error(
        "Deployment of contract which needs more than 10kb of account space needs several \
    transactions for reallocation and cannot be performed in a single instruction. \
    That's why you have to use iterative transaction for the deployment."
    )]
    AccountSpaceAllocationFailure,

    #[error("Invalid account for call {0}")]
    InvalidAccountForCall(Pubkey),

    #[error("Call for external Solana programs not available in this mode")]
    UnavalableExternalSolanaCall,

    #[error("Program not allowed to call itself")]
    RecursiveCall,

    #[error("External call fails {0}: {1}")]
    ExternalCallFailed(Pubkey, String),

    #[error("Operator Balance - invalid owner {0}, expected = {1}")]
    OperatorBalanceInvalidOwner(Pubkey, Pubkey),

    #[error("Operator Balance - not found")]
    OperatorBalanceMissing,

    #[error("Operator Balance - invalid chainId")]
    OperatorBalanceInvalidChainId,

    #[error("Operator Balance - invalid address")]
    OperatorBalanceInvalidAddress,

    #[error(
        "Instructions that execute Ethereum DynamicGas transaction (EIP-1559) should specify priority fee."
    )]
    PriorityFeeNotSpecified,

    #[error("Error while parsing priority fee instructions: {0}")]
    PriorityFeeParsingError(String),

    #[error("Priority fee calculation error: {0}")]
    PriorityFeeError(String),

    #[error("Transaction Tree - not ready for destruction")]
    TreeAccountNotReadyForDestruction,

    #[error("Transaction Tree - last index overflow")]
    TreeAccountLastIndexOverflow,

    #[error("Transaction Tree - invalid payer")]
    TreeAccountInvalidPayer,

    #[error("Transaction Tree - invalid chainId")]
    TreeAccountInvalidChainId,

    #[error("Transaction Tree - invalid transaction type")]
    TreeAccountTxInvalidType,

    #[error("Transaction Tree - invalid transaction data")]
    TreeAccountTxInvalidData,

    #[error("Transaction Tree - invalid child transaction index")]
    TreeAccountTxInvalidChildIndex,

    #[error("Transaction Tree - transaction invalid parent count")]
    TreeAccountTxInvalidParentCount,

    #[error("Transaction Tree - transaction invalid success execute limit")]
    TreeAccountTxInvalidSuccessLimit,

    #[error("Transaction Tree - transaction not found")]
    TreeAccountTxNotFound,

    #[error("Transaction Tree - transaction invalid status")]
    TreeAccountTxInvalidStatus,

    #[error("Transaction Tree - transaction requires at least 1.1 GAlan for gas price")]
    TreeAccountInvalidPriorityFeePerGas,

    #[error("Transaction Tree - transaction requires at least 25'000 gas limit")]
    TreeAccountInvalidGasLimit,

    #[error("Transaction Tree - transaction with the same nonce already exists")]
    TreeAccountAlreadyExists,

    #[error("Attempt to perform an operation with classic transaction, whereas scheduled transaction is expected")]
    NotScheduledTransaction,

    #[error("Scheduled Transaction has invalid tree account: expected={0}, actual={1}")]
    ScheduledTxInvalidTreeAccount(Pubkey, Pubkey),

    #[error("Scheduled Transaction is not ready to be finalized: holder={0}")]
    ScheduledTxNoExitStatus(Pubkey),

    #[error("Schedule Transaction is already in progress, holder={0}")]
    ScheduledTxAlreadyInProgress(Pubkey),

    #[error("Schedule Transaction is already complete, holder={0}")]
    ScheduledTxAlreadyComplete(Pubkey),

    #[error("Scheduled Transaction has invalid index: inside transaction={0}, inside instruction data={1}")]
    ScheduledTxInvalidIndex(u16, u16),

    #[error("Attempt to perform an operation with scheduled transaction, whereas classic transaction is expected")]
    NotClassicTransaction,

    #[error("Treasury Account - not found")]
    TreasuryMissing,

    #[error("Account {0} - invalid header version {1}")]
    AccountInvalidHeader(Pubkey, u8),

    #[error("Revert after Solana Call is not supported")]
    RevertAfterSolanaCall,

    #[error("Unsupported EIP-2718 Transaction type | First byte: {0}")]
    UnsuppotedEthereumTransactionType(u8),

    #[error("Unsupported Neon Transaction type | Second byte: {0}")]
    UnsuppotedNeonTransactionType(u8),

    #[error("Solana programs was interrupted")]
    InterruptedCall(#[serde(skip)] Box<Option<InterruptedState>>),
}

impl Error {
    #[must_use]
    pub fn code(&self) -> u8 {
        let discriminant = ErrorDiscriminants::from(self);
        discriminant as u8
    }

    pub fn log_data(&self) {
        let bytes = bincode::serialize(self).unwrap();
        log_data(&[
            b"ERROR",
            &self.code().to_le_bytes(),
            &bytes,
            (self.to_string().as_bytes()),
        ]);
    }
}
pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for ProgramError {
    fn from(e: Error) -> Self {
        log_msg!("{}", e);
        match e {
            Error::ProgramError(e) => e,
            _ => Self::Custom(0),
        }
    }
}

impl From<&'static str> for Error {
    fn from(value: &'static str) -> Self {
        Self::Custom(value.to_string())
    }
}

impl From<String> for Error {
    fn from(value: String) -> Self {
        Self::Custom(value)
    }
}

macro_rules! panic_with_error {
    ($e:expr) => {{
        let error = $crate::error::Error::from($e);
        error.log_data();
        panic!("{}", error);
    }};
}

#[must_use]
pub fn format_revert_error(msg: &[u8]) -> Option<&str> {
    if msg.starts_with(&[0x08, 0xc3, 0x79, 0xa0]) {
        // Error(string) function selector
        let msg = &msg[4..];
        if msg.len() < 64 {
            return None;
        }

        let offset = U256::from_be_bytes(*arrayref::array_ref![msg, 0, 32]);
        if offset != 32 {
            return None;
        }

        let length = U256::from_be_bytes(*arrayref::array_ref![msg, 32, 32]);
        let length: usize = length.try_into().ok()?;

        let begin = 64_usize;
        let end = begin.checked_add(length)?;

        let reason = msg.get(begin..end)?;
        std::str::from_utf8(reason).ok()
    } else {
        None
    }
}

#[must_use]
pub fn format_revert_panic(msg: &[u8]) -> Option<U256> {
    if msg.starts_with(&[0x4e, 0x48, 0x7b, 0x71]) {
        // Panic(uint256) function selector
        let msg = &msg[4..];
        if msg.len() != 32 {
            return None;
        }

        let value = arrayref::array_ref![msg, 0, 32];
        Some(U256::from_be_bytes(*value))
    } else {
        None
    }
}

pub fn print_revert_message(msg: &[u8]) {
    if msg.is_empty() {
        return log_msg!("Revert");
    }

    if let Some(reason) = format_revert_error(msg) {
        return log_msg!("Revert: Error(\"{}\")", reason);
    }

    if let Some(reason) = format_revert_panic(msg) {
        return log_msg!("Revert: Panic({:#x})", reason);
    }

    log_msg!("Revert: {}", hex::encode(msg));
}

#[must_use]
pub fn build_revert_message(msg: &str) -> Vector<u8> {
    let data_len = if msg.len() % 32 == 0 {
        std::cmp::max(msg.len(), 32)
    } else {
        ((msg.len() / 32) + 1) * 32
    };

    let capacity = 4 + 32 + 32 + data_len;
    let mut result = Vector::with_capacity_in(capacity, acc_allocator());
    result.extend_from_slice(&[0x08, 0xc3, 0x79, 0xa0]); // Error(string) function selector

    let offset = U256::new(0x20);
    result.extend_from_slice(&offset.to_be_bytes());

    let length = U256::new(msg.len() as u128);
    result.extend_from_slice(&length.to_be_bytes());

    result.extend_from_slice(msg.as_bytes());

    assert!(result.len() <= capacity);
    result.resize(capacity, 0);

    result
}
