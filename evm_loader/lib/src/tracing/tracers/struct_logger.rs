use async_trait::async_trait;
use std::collections::BTreeMap;

use ethnum::U256;
use evm_loader::evm::database::Database;
use serde::Serialize;
use serde_json::Value;
use web3::types::Bytes;

use evm_loader::evm::opcode_table::Opcode;
use evm_loader::evm::tracing::{Event, EventListener};
use evm_loader::evm::{opcode_table, ExitStatus};
use evm_loader::types::Address;

use crate::tracing::tracers::Tracer;
use crate::tracing::TraceConfig;
use crate::types::TxParams;

/// `StructLoggerResult` groups all structured logs emitted by the EVM
/// while replaying a transaction in debug mode as well as transaction
/// execution status, the amount of gas used and the return value
/// see <https://github.com/ethereum/go-ethereum/blob/master/eth/tracers/logger/logger.go#L404>
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StructLoggerResult {
    /// Total used gas but include the refunded gas
    gas: u64,
    /// Is execution failed or not
    failed: bool,
    /// The data after execution or revert reason
    return_value: String,
    /// Logs emitted during execution
    struct_logs: Vec<StructLog>,
}

/// `StructLog` stores a structured log emitted by the EVM while replaying a
/// transaction in debug mode
/// see <https://github.com/ethereum/go-ethereum/blob/master/eth/tracers/logger/logger.go#L413>
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StructLog {
    /// Program counter.
    pc: u64,
    /// Operation name
    op: Opcode,
    /// Amount of used gas
    gas: u64,
    /// Gas cost for this instruction.
    gas_cost: u64,
    /// Current depth
    depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// Snapshot of the current stack sate
    #[serde(skip_serializing_if = "Option::is_none")]
    stack: Option<Vec<U256>>,
    #[serde(skip_serializing_if = "is_empty")]
    return_data: Bytes,
    /// Snapshot of the current memory sate
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<Vec<String>>, // chunks of 32 bytes
    /// Result of the step
    /// Snapshot of the current storage
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<BTreeMap<String, String>>,
    /// Refund counter
    #[serde(skip_serializing_if = "is_zero")]
    refund: u64,
}

fn is_empty(bytes: &Bytes) -> bool {
    bytes.0.is_empty()
}

/// This is only used for serialize
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(num: &u64) -> bool {
    *num == 0
}

pub struct StructLogger {
    actual_gas_used: Option<U256>,
    config: TraceConfig,
    logs: Vec<StructLog>,
    depth: usize,
    storage: BTreeMap<Address, BTreeMap<String, String>>,
    exit_status: Option<ExitStatus>,
}

impl StructLogger {
    #[must_use]
    pub fn new(config: TraceConfig, tx: &TxParams) -> Self {
        Self {
            actual_gas_used: tx.actual_gas_used,
            config,
            logs: vec![],
            depth: 0,
            storage: BTreeMap::new(),
            exit_status: None,
        }
    }
}

#[async_trait(?Send)]
impl EventListener for StructLogger {
    /// See <https://github.com/ethereum/go-ethereum/blob/master/eth/tracers/logger/logger.go#L151>
    async fn event(
        &mut self,
        executor_state: &impl Database,
        event: Event,
    ) -> evm_loader::error::Result<()> {
        match event {
            Event::BeginVM { .. } => {
                self.depth += 1;
            }
            Event::EndVM { status, .. } => {
                if self.depth == 1 {
                    self.exit_status = Some(status);
                }
                self.depth -= 1;
            }
            Event::BeginStep {
                context,
                opcode,
                pc,
                stack,
                memory,
                return_data,
                ..
            } => {
                if self.config.limit > 0 && self.logs.len() >= self.config.limit {
                    return Ok(());
                }

                let storage = if self.config.disable_storage {
                    None
                } else if opcode == opcode_table::SLOAD && !stack.is_empty() {
                    let index = U256::from_be_bytes(stack[stack.len() - 1]);

                    self.storage.entry(context.contract).or_default().insert(
                        hex::encode(index.to_be_bytes()),
                        hex::encode(executor_state.storage(context.contract, index).await?),
                    );

                    Some(
                        self.storage
                            .get(&context.contract)
                            .cloned()
                            .unwrap_or_default(),
                    )
                } else if opcode == opcode_table::SSTORE && stack.len() >= 2 {
                    self.storage.entry(context.contract).or_default().insert(
                        hex::encode(stack[stack.len() - 1]),
                        hex::encode(stack[stack.len() - 2]),
                    );

                    Some(
                        self.storage
                            .get(&context.contract)
                            .cloned()
                            .unwrap_or_default(),
                    )
                } else {
                    None
                };
                let stack = if self.config.disable_stack {
                    None
                } else {
                    Some(stack.into_iter().map(U256::from_be_bytes).collect())
                };

                let memory = if self.config.enable_memory {
                    Some(memory.chunks(32).map(hex::encode).collect())
                } else {
                    None
                };

                self.logs.push(StructLog {
                    pc: pc as u64,
                    op: opcode,
                    gas: 0,
                    gas_cost: 0,
                    depth: self.depth,
                    memory,
                    stack,
                    return_data: return_data.into(),
                    storage,
                    error: None,
                    refund: 0,
                });
            }
        };
        Ok(())
    }
}

impl Tracer for StructLogger {
    fn into_traces(self, emulator_gas_used: u64) -> Value {
        let exit_status = self.exit_status.expect("Exit status should be set");
        let result = StructLoggerResult {
            gas: self.actual_gas_used.map_or(emulator_gas_used, U256::as_u64),
            failed: !exit_status
                .is_succeed()
                .expect("Emulation is not completed"),
            return_value: hex::encode(exit_status.into_result().unwrap_or_default()),
            struct_logs: self.logs,
        };
        serde_json::to_value(result).expect("serialization should not fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_struct_logger_result_all_fields() {
        let struct_logger_result = StructLoggerResult {
            gas: 20000,
            failed: false,
            return_value: "000000000000000000000000000000000000000000000000000000000000001b"
                .to_string(),
            struct_logs: vec![StructLog {
                pc: 8,
                op: opcode_table::PUSH2,
                gas: 0,
                gas_cost: 0,
                depth: 1,
                stack: Some(vec![U256::from(0u8), U256::from(1u8)]),
                memory: Some(vec![
                    "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                    "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
                    "0000000000000000000000000000000000000000000000000000000000000080".to_string(),
                ]),
                return_data: vec![].into(),
                storage: None,
                refund: 0,
                error: None,
            }],
        };
        assert_eq!(serde_json::to_string(&struct_logger_result).unwrap(), "{\"gas\":20000,\"failed\":false,\"returnValue\":\"000000000000000000000000000000000000000000000000000000000000001b\",\"structLogs\":[{\"pc\":8,\"op\":\"PUSH2\",\"gas\":0,\"gasCost\":0,\"depth\":1,\"stack\":[\"0x0\",\"0x1\"],\"memory\":[\"0000000000000000000000000000000000000000000000000000000000000000\",\"0000000000000000000000000000000000000000000000000000000000000000\",\"0000000000000000000000000000000000000000000000000000000000000080\"]}]}");
    }

    #[test]
    fn test_serialize_struct_logger_result_no_optional_fields() {
        let struct_logger_result = StructLoggerResult {
            gas: 20000,
            failed: false,
            return_value: "000000000000000000000000000000000000000000000000000000000000001b"
                .to_string(),
            struct_logs: vec![StructLog {
                pc: 0,
                op: opcode_table::PUSH1,
                gas: 0,
                gas_cost: 0,
                depth: 1,
                stack: None,
                memory: None,
                return_data: vec![].into(),
                storage: None,
                refund: 0,
                error: None,
            }],
        };
        assert_eq!(serde_json::to_string(&struct_logger_result).unwrap(), "{\"gas\":20000,\"failed\":false,\"returnValue\":\"000000000000000000000000000000000000000000000000000000000000001b\",\"structLogs\":[{\"pc\":0,\"op\":\"PUSH1\",\"gas\":0,\"gasCost\":0,\"depth\":1}]}");
    }

    #[test]
    fn test_serialize_struct_logger_result_empty_stack_empty_memory() {
        let struct_logger_result = StructLoggerResult {
            gas: 20000,
            failed: false,
            return_value: "000000000000000000000000000000000000000000000000000000000000001b"
                .to_string(),
            struct_logs: vec![StructLog {
                pc: 0,
                op: opcode_table::PUSH1,
                gas: 0,
                gas_cost: 0,
                depth: 1,
                stack: Some(vec![]),
                memory: Some(vec![]),
                return_data: vec![].into(),
                storage: None,
                refund: 0,
                error: None,
            }],
        };
        assert_eq!(serde_json::to_string(&struct_logger_result).unwrap(), "{\"gas\":20000,\"failed\":false,\"returnValue\":\"000000000000000000000000000000000000000000000000000000000000001b\",\"structLogs\":[{\"pc\":0,\"op\":\"PUSH1\",\"gas\":0,\"gasCost\":0,\"depth\":1,\"stack\":[],\"memory\":[]}]}");
    }
}
