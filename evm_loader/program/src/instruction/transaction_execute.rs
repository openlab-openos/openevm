use crate::account::{AccountsDB, AllocateResult};
use crate::account_storage::ProgramAccountStorage;
use crate::debug::log_data;
use crate::error::{Error, Result};
use crate::evm::tracing::NoopEventListener;
use crate::evm::Machine;
use crate::executor::{ExecutorState, SyncedExecutorState};
use crate::gasometer::Gasometer;
use crate::instruction::transaction_step::log_return_value;
use crate::types::{Address, Transaction};

pub fn execute(
    accounts: AccountsDB<'_>,
    mut gasometer: Gasometer,
    trx: Transaction,
    origin: Address,
) -> Result<()> {
    let chain_id = trx.chain_id().unwrap_or(crate::config::DEFAULT_CHAIN_ID);
    let gas_limit = trx.gas_limit();
    let gas_price = trx.gas_price();

    let mut account_storage = ProgramAccountStorage::new(accounts)?;

    trx.validate(origin, &account_storage)?;

    account_storage.origin(origin, &trx)?.increment_nonce()?;

    let (exit_reason, apply_state, steps_executed) = {
        let mut backend = ExecutorState::new(&account_storage);

        let mut evm = Machine::new(&trx, origin, &mut backend, None::<NoopEventListener>)?;
        let (result, steps_executed, _) = evm.execute(u64::MAX, &mut backend)?;

        let actions = backend.into_actions();

        (result, actions, steps_executed)
    };

    log_data(&[
        b"STEPS",
        &steps_executed.to_le_bytes(), // Iteration steps
        &steps_executed.to_le_bytes(), // Total steps is the same as iteration steps
    ]);

    let allocate_result = account_storage.allocate(&apply_state)?;
    if allocate_result != AllocateResult::Ready {
        return Err(Error::AccountSpaceAllocationFailure);
    }

    account_storage.apply_state_change(apply_state)?;
    account_storage.transfer_treasury_payment()?;

    gasometer.record_operator_expenses(account_storage.operator());
    let used_gas = gasometer.used_gas();
    if used_gas > gas_limit {
        return Err(Error::OutOfGas(gas_limit, used_gas));
    }

    log_data(&[b"GAS", &used_gas.to_le_bytes(), &used_gas.to_le_bytes()]);

    let gas_cost = used_gas.saturating_mul(gas_price);
    account_storage.transfer_gas_payment(origin, chain_id, gas_cost)?;

    log_return_value(&exit_reason);

    Ok(())
}

pub fn execute_with_solana_call(
    accounts: AccountsDB<'_>,
    mut gasometer: Gasometer,
    trx: Transaction,
    origin: Address,
) -> Result<()> {
    let chain_id = trx.chain_id().unwrap_or(crate::config::DEFAULT_CHAIN_ID);
    let gas_limit = trx.gas_limit();
    let gas_price = trx.gas_price();

    let mut account_storage = ProgramAccountStorage::new(accounts)?;

    trx.validate(origin, &account_storage)?;

    account_storage.origin(origin, &trx)?.increment_nonce()?;

    let (exit_reason, steps_executed) = {
        let mut backend = SyncedExecutorState::new(&mut account_storage);

        let mut evm = Machine::new(&trx, origin, &mut backend, None::<NoopEventListener>)?;
        let (result, steps_executed, _) = evm.execute(u64::MAX, &mut backend)?;

        (result, steps_executed)
    };

    log_data(&[
        b"STEPS",
        &steps_executed.to_le_bytes(), // Iteration steps
        &steps_executed.to_le_bytes(), // Total steps is the same as iteration steps
    ]);

    account_storage.increment_revision_for_modified_contracts()?;
    account_storage.transfer_treasury_payment()?;

    gasometer.record_operator_expenses(account_storage.operator());
    let used_gas = gasometer.used_gas();
    if used_gas > gas_limit {
        return Err(Error::OutOfGas(gas_limit, used_gas));
    }

    log_data(&[b"GAS", &used_gas.to_le_bytes(), &used_gas.to_le_bytes()]);

    let gas_cost = used_gas.saturating_mul(gas_price);
    account_storage.transfer_gas_payment(origin, chain_id, gas_cost)?;

    log_return_value(&exit_reason);

    Ok(())
}
