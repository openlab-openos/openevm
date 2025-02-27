use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::{
    client_error::Result as SolanaClientResult, rpc_config::RpcSendTransactionConfig,
};
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel},
    instruction::Instruction,
    message::Message,
    signature::Signature,
    signer::Signer,
    transaction::Transaction,
};

pub mod collect_treasury;
pub mod emulate;
pub mod get_balance;
pub mod get_config;
pub mod get_contract;
pub mod get_holder;
pub mod get_neon_elf;
pub mod get_storage_at;
pub mod get_transaction_tree;
pub mod init_environment;
pub mod simulate_solana;
pub mod trace;
mod transaction_executor;

pub async fn send_transaction(
    rpc_client: &RpcClient,
    signer: &dyn Signer,
    instructions: &[Instruction],
) -> SolanaClientResult<Signature> {
    let message = Message::new(instructions, Some(&signer.pubkey()));
    let mut transaction = Transaction::new_unsigned(message);
    let signers = [signer];
    let (blockhash, _last_valid_slot) = rpc_client
        .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
        .await?;
    transaction.try_sign(&signers, blockhash)?;

    rpc_client
        .send_and_confirm_transaction_with_spinner_and_config(
            &transaction,
            CommitmentConfig::confirmed(),
            RpcSendTransactionConfig {
                preflight_commitment: Some(CommitmentLevel::Confirmed),
                ..RpcSendTransactionConfig::default()
            },
        )
        .await
}
