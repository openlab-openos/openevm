use std::cell::RefCell;

use ethnum::{AsU256, U256};
use maybe_async::maybe_async;
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;

use super::precompile_extension::PrecompiledContracts;
use super::state::TimestampedContracts;
use super::{BlockParams, OwnedAccountInfo};

use crate::account_storage::{AccountStorage, LogCollector, SyncedAccountStorage};
use crate::allocator::acc_allocator;
use crate::error::{Error, Result};
use crate::evm::database::Database;
use crate::evm::precompile::is_precompile_address;
use crate::evm::Context;
use crate::executor::action;
use crate::executor::ExecutorStateData;
use crate::types::{Address, TreeMap, Vector};

enum Action {
    SetTransientStorage {
        address: Address,
        index: U256,
        value: [u8; 32],
    },
}

pub struct SyncedExecutorState<'a, B: AccountStorage> {
    pub backend: &'a mut B,
    pub block_params: Option<BlockParams>,
    pub timestamped_contracts: RefCell<TimestampedContracts>,
    actions: Vector<Action>,
    stack: Vector<usize>,
}

impl<'a, B: SyncedAccountStorage> SyncedExecutorState<'a, B> {
    #[must_use]
    pub fn new(backend: &'a mut B) -> Self {
        Self {
            backend,
            actions: Vector::with_capacity_in(64, acc_allocator()),
            stack: Vector::with_capacity_in(16, acc_allocator()),
            block_params: None,
            timestamped_contracts: RefCell::new(TreeMap::new()),
        }
    }

    #[must_use]
    pub fn new_with_state_data(backend: &'a mut B, state_data: &'a ExecutorStateData) -> Self {
        let mut actions = Vector::with_capacity_in(64, acc_allocator());
        let mut stack = state_data.into_stack().clone();

        for (action_idx, action) in state_data.into_actions().iter().enumerate() {
            if let action::Action::EvmSetTransientStorage {
                address,
                index,
                value,
            } = action
            {
                actions.push(Action::SetTransientStorage {
                    address: *address,
                    index: *index,
                    value: *value,
                });
            } else {
                for (frame_idx, frame) in stack.iter_mut().enumerate() {
                    if state_data.into_stack()[frame_idx] >= action_idx {
                        *frame -= 1;
                    }
                }
            }
        }

        Self {
            backend,
            actions,
            stack,
            block_params: Some(state_data.block_params),
            timestamped_contracts: RefCell::clone(&state_data.timestamped_contracts),
        }
    }

    #[must_use]
    pub fn backend(&self) -> &B {
        self.backend
    }
}

impl<B: AccountStorage> LogCollector for SyncedExecutorState<'_, B> {
    fn collect_log<const N: usize>(
        &mut self,
        address: &[u8; 20],
        topics: [[u8; 32]; N],
        data: &[u8],
    ) {
        self.backend.collect_log(address, topics, data);
    }
}

#[maybe_async(?Send)]
impl<'a, B: SyncedAccountStorage> Database for SyncedExecutorState<'a, B> {
    fn is_synced_state(&self) -> bool {
        true
    }
    fn program_id(&self) -> &Pubkey {
        self.backend.program_id()
    }
    fn operator(&self) -> Pubkey {
        self.backend.operator()
    }
    fn chain_id_to_token(&self, chain_id: u64) -> Pubkey {
        self.backend.chain_id_to_token(chain_id)
    }
    fn contract_pubkey(&self, address: Address) -> (Pubkey, u8) {
        self.backend.contract_pubkey(address)
    }

    async fn solana_user_address(&self, address: Address) -> Result<Option<Pubkey>> {
        let pubkey = self.backend.solana_user_address(address).await;
        Ok(pubkey)
    }

    async fn nonce(&self, from_address: Address, from_chain_id: u64) -> Result<u64> {
        let nonce = self.backend.nonce(from_address, from_chain_id).await;
        Ok(nonce)
    }

    async fn increment_nonce(&mut self, address: Address, chain_id: u64) -> Result<()> {
        self.backend.increment_nonce(address, chain_id).await?;
        Ok(())
    }

    async fn balance(&self, from_address: Address, from_chain_id: u64) -> Result<U256> {
        let balance = self.backend.balance(from_address, from_chain_id).await;
        Ok(balance)
    }

    async fn transfer(
        &mut self,
        source: Address,
        target: Address,
        chain_id: u64,
        value: U256,
    ) -> Result<()> {
        if value == U256::ZERO {
            return Ok(());
        }

        let target_chain_id = self.contract_chain_id(target).await.unwrap_or(chain_id);

        if (self.code_size(target).await? > 0) && (target_chain_id != chain_id) {
            return Err(Error::InvalidTransferToken(source, chain_id));
        }

        if source == target {
            return Ok(());
        }

        if self.balance(source, chain_id).await? < value {
            return Err(Error::InsufficientBalance(source, chain_id, value));
        }

        self.backend
            .transfer(source, target, chain_id, value)
            .await?;
        Ok(())
    }

    async fn burn(&mut self, source: Address, chain_id: u64, value: U256) -> Result<()> {
        self.backend.burn(source, chain_id, value).await?;
        Ok(())
    }

    async fn code_size(&self, from_address: Address) -> Result<usize> {
        if PrecompiledContracts::is_precompile_extension(&from_address) {
            return Ok(1);
        }
        if is_precompile_address(&from_address) {
            return Ok(0);
        }

        Ok(self.backend.code_size(from_address).await)
    }

    async fn code(&self, from_address: Address) -> Result<crate::evm::Buffer> {
        if PrecompiledContracts::is_precompile_extension(&from_address) {
            return Ok(crate::evm::Buffer::from_slice(&[0xFE]));
        }
        if is_precompile_address(&from_address) {
            return Ok(crate::evm::Buffer::from_slice(&[]));
        }

        Ok(self.backend.code(from_address).await)
    }

    async fn set_code(&mut self, address: Address, chain_id: u64, code: Vector<u8>) -> Result<()> {
        if code.starts_with(&[0xEF]) {
            // https://eips.ethereum.org/EIPS/eip-3541
            return Err(Error::EVMObjectFormatNotSupported(address));
        }

        if code.len() > 0x6000 {
            // https://eips.ethereum.org/EIPS/eip-170
            return Err(Error::ContractCodeSizeLimit(address, code.len()));
        }

        self.backend.set_code(address, chain_id, code).await?;
        Ok(())
    }

    async fn storage(&self, from_address: Address, from_index: U256) -> Result<[u8; 32]> {
        Ok(self.backend.storage(from_address, from_index).await)
    }

    async fn set_storage(&mut self, address: Address, index: U256, value: [u8; 32]) -> Result<()> {
        self.backend.set_storage(address, index, value).await?;
        Ok(())
    }

    async fn transient_storage(&self, from_address: Address, from_index: U256) -> Result<[u8; 32]> {
        for action in self.actions.iter().rev() {
            #[allow(irrefutable_let_patterns)]
            if let Action::SetTransientStorage {
                address,
                index,
                value,
            } = action
            {
                if (&from_address == address) && (&from_index == index) {
                    return Ok(*value);
                }
            }
        }

        Ok([0; 32])
    }

    fn set_transient_storage(
        &mut self,
        address: Address,
        index: U256,
        value: [u8; 32],
    ) -> Result<()> {
        self.actions.push(Action::SetTransientStorage {
            address,
            index,
            value,
        });
        Ok(())
    }

    async fn block_hash(&self, number: U256) -> Result<[u8; 32]> {
        // geth:
        //  - checks the overflow
        //  - converts to u64
        //  - checks on last 256 blocks

        if number >= u64::MAX.as_u256() {
            return Ok(<[u8; 32]>::default());
        }

        let number = number.as_u64();
        let block_slot = self.backend.block_number().as_u64();
        let lower_block_slot = if block_slot < 257 {
            0
        } else {
            block_slot - 256
        };

        if number >= block_slot || lower_block_slot > number {
            return Ok(<[u8; 32]>::default());
        }

        Ok(self.backend.block_hash(number).await)
    }

    fn block_number(&self, current_contract: Address) -> Result<U256> {
        let mut timestamped_contracts = self.timestamped_contracts.borrow_mut();
        timestamped_contracts.insert_if_not_exists(current_contract, ());

        if let Some(block) = self.block_params {
            return Ok(block.number);
        }

        Ok(self.backend.block_number())
    }

    fn block_timestamp(&self, current_contract: Address) -> Result<U256> {
        let mut timestamped_contracts = self.timestamped_contracts.borrow_mut();
        timestamped_contracts.insert_if_not_exists(current_contract, ());

        if let Some(block) = self.block_params {
            return Ok(block.timestamp);
        }

        Ok(self.backend.block_timestamp())
    }

    async fn external_account(&self, address: Pubkey) -> Result<OwnedAccountInfo> {
        let account = self.backend.clone_solana_account(&address).await;
        return Ok(account);
    }

    fn rent(&self) -> &Rent {
        self.backend.rent()
    }

    fn return_data(&self) -> Option<(Pubkey, Vec<u8>)> {
        self.backend.return_data()
    }

    fn set_return_data(&mut self, data: &[u8]) {
        self.backend.set_return_data(data);
    }

    async fn map_solana_account<F, R>(&self, address: &Pubkey, action: F) -> R
    where
        F: FnOnce(&solana_program::account_info::AccountInfo) -> R,
    {
        self.backend.map_solana_account(address, action).await
    }

    fn snapshot(&mut self) {
        self.stack.push(self.actions.len());
        self.backend.snapshot();
    }

    fn revert_snapshot(&mut self) {
        let actions_len = self
            .stack
            .pop()
            .expect("Fatal Error: Inconsistent EVM Call Stack");

        self.actions.truncate(actions_len);

        if self.stack.is_empty() {
            // sanity check
            assert!(self.actions.is_empty());
        }

        self.backend.revert_snapshot();
    }

    fn commit_snapshot(&mut self) {
        self.stack
            .pop()
            .expect("Fatal Error: Inconsistent EVM Call Stack");
        self.backend.commit_snapshot();
    }

    async fn precompile_extension(
        &mut self,
        context: &Context,
        address: &Address,
        data: &[u8],
        is_static: bool,
    ) -> Option<Result<Vector<u8>>> {
        PrecompiledContracts::call_precompile_extension(self, context, address, data, is_static)
            .await
    }

    fn default_chain_id(&self) -> u64 {
        self.backend.default_chain_id()
    }

    fn is_valid_chain_id(&self, chain_id: u64) -> bool {
        self.backend.is_valid_chain_id(chain_id)
    }

    async fn contract_chain_id(&self, contract: Address) -> Result<u64> {
        if PrecompiledContracts::is_precompile_extension(&contract)
            || is_precompile_address(&contract)
        {
            return Ok(self.default_chain_id());
        }
        self.backend.contract_chain_id(contract).await
    }

    async fn queue_external_instruction(
        &mut self,
        instruction: Instruction,
        seeds: Vector<Vector<Vector<u8>>>,
        emulated_internally: bool,
    ) -> Result<()> {
        self.backend
            .execute_external_instruction(instruction, seeds, emulated_internally)
            .await?;
        Ok(())
    }
}
