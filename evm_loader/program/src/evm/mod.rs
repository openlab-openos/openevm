#![allow(clippy::trait_duplication_in_bounds)]
#![allow(clippy::type_repetition_in_bounds)]
#![allow(clippy::unsafe_derive_deserialize)]
#![allow(clippy::future_not_send)]

use crate::account::InterruptedState;
use ethnum::U256;
use maybe_async::maybe_async;
use std::{fmt::Display, marker::PhantomData, mem::ManuallyDrop, ops::Range};

pub use buffer::Buffer;

#[cfg(target_os = "solana")]
use crate::evm::tracing::NoopEventListener;
use crate::executor::precompile_extension::PrecompiledContracts;
use crate::{
    debug::log_data,
    error::{build_revert_message, Error, Result},
    evm::{opcode::Action, precompile::is_precompile_address},
    types::{Address, Transaction, Vector},
};
use crate::{evm::tracing::EventListener, types::boxx::Boxx};

use self::{database::Database, memory::Memory, stack::Stack};

mod buffer;
pub mod database;
mod memory;
pub mod opcode;
pub mod opcode_table;
pub mod precompile;
mod stack;
pub mod tracing;
mod utils;

macro_rules! tracing_event {
    ($self:expr, $backend:expr, $event:expr) => {
        #[cfg(not(target_os = "solana"))]
        if let Some(tracer) = &mut $self.tracer {
            tracer.event($backend, $event).await?;
        }
    };
}

macro_rules! begin_vm {
    ($self:expr, $backend:expr, $context:expr, $chain_id:expr, $input:expr, $opcode:expr) => {
        tracing_event!(
            $self,
            $backend,
            crate::evm::tracing::Event::BeginVM {
                context: $context,
                chain_id: $chain_id,
                input: $input.to_vec(),
                opcode: $opcode
            }
        );
    };
    ($self:expr, $backend:expr, $context:expr, $chain_id:expr, $input:expr) => {
        begin_vm!(
            $self,
            $backend,
            $context,
            $chain_id,
            $input,
            $self.execution_code.get_or_default($self.pc).into()
        );
    };
}

macro_rules! end_vm {
    ($self:expr, $backend:expr, $status:expr) => {
        tracing_event!(
            $self,
            $backend,
            crate::evm::tracing::Event::EndVM {
                context: $self.context,
                chain_id: $self.chain_id,
                status: $status
            }
        );
    };
}

macro_rules! begin_step {
    ($self:expr, $backend:expr) => {
        tracing_event!(
            $self,
            $backend,
            crate::evm::tracing::Event::BeginStep {
                context: $self.context,
                chain_id: $self.chain_id,
                opcode: $self.execution_code.get_or_default($self.pc).into(),
                pc: $self.pc,
                stack: $self.stack.to_vec(),
                memory: $self.memory.to_vec(),
                return_data: $self.return_data.to_vec()
            }
        );
    };
}

pub(crate) use begin_step;
pub(crate) use begin_vm;
pub(crate) use end_vm;
pub(crate) use tracing_event;

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(C)]
pub enum ExitStatus {
    Stop,
    Return(Vector<u8>),
    Revert(Vector<u8>),
    Suicide,
    Interrupted(Box<Option<InterruptedState>>),
    StepLimit,
    Cancel,
}

impl Display for ExitStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.status())
    }
}

impl ExitStatus {
    #[must_use]
    pub fn status(&self) -> &'static str {
        match self {
            ExitStatus::Return(_) | ExitStatus::Stop | ExitStatus::Suicide => "succeed",
            ExitStatus::Revert(_) => "revert",
            ExitStatus::Interrupted(_) => "interrupted due Solana call",
            ExitStatus::StepLimit => "step limit exceeded",
            ExitStatus::Cancel => "cancel",
        }
    }

    #[must_use]
    pub fn is_succeed(&self) -> Option<bool> {
        match self {
            ExitStatus::Stop | ExitStatus::Return(_) | ExitStatus::Suicide => Some(true),
            ExitStatus::Revert(_) | ExitStatus::Cancel => Some(false),
            ExitStatus::Interrupted(_) | ExitStatus::StepLimit => None,
        }
    }

    #[must_use]
    pub fn into_result(self) -> Option<Vec<u8>> {
        match self {
            ExitStatus::Return(v) | ExitStatus::Revert(v) => Some(v.to_vec()),
            ExitStatus::Stop
            | ExitStatus::Suicide
            | ExitStatus::Interrupted(_)
            | ExitStatus::StepLimit
            | ExitStatus::Cancel => None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
#[repr(C)]
pub enum Reason {
    Call,
    Create,
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct Context {
    pub caller: Address,
    pub contract: Address,
    pub contract_chain_id: u64,
    pub value: U256,
    pub code_address: Option<Address>,
}

#[repr(C)]
pub struct Machine<B: Database, T: EventListener> {
    origin: Address,
    chain_id: u64,
    context: Context,

    gas_price: U256,
    gas_limit: U256,

    execution_code: Buffer,
    call_data: Buffer,
    return_data: Buffer,
    return_range: Range<usize>,

    stack: Stack,
    memory: Memory,
    pc: usize,

    is_static: bool,
    reason: Reason,

    parent: Option<Boxx<Self>>,

    phantom: PhantomData<*const B>,

    tracer: Option<T>,
}

#[cfg(target_os = "solana")]
impl<B: Database> Machine<B, NoopEventListener> {
    fn reinit_buffer(buffer: &mut Buffer, backend: &B) {
        if let Some((key, range)) = buffer.uninit_data() {
            *buffer =
                backend.map_solana_account(&key, |i| unsafe { Buffer::from_account(i, range) });
        }
    }

    pub fn reinit(&mut self, backend: &B) {
        let mut machine = self;
        loop {
            Self::reinit_buffer(&mut machine.call_data, backend);
            Self::reinit_buffer(&mut machine.execution_code, backend);
            Self::reinit_buffer(&mut machine.return_data, backend);
            match &mut machine.parent {
                None => break,
                Some(parent) => machine = parent,
            }
        }
    }
}

impl<B: Database, T: EventListener> Machine<B, T> {
    #[maybe_async]
    pub async fn new(
        trx: &Transaction,
        origin: Address,
        backend: &mut B,
        tracer: Option<T>,
    ) -> Result<Self> {
        let trx_chain_id = trx.chain_id().unwrap_or_else(|| backend.default_chain_id());

        if backend.balance(origin, trx_chain_id).await? < trx.value() {
            return Err(Error::InsufficientBalance(
                origin,
                trx_chain_id,
                trx.value(),
            ));
        }

        if trx.target().is_some() {
            Self::new_call(trx_chain_id, trx, origin, backend, tracer).await
        } else {
            Self::new_create(trx_chain_id, trx, origin, backend, tracer).await
        }
    }

    #[maybe_async]
    async fn new_call(
        chain_id: u64,
        trx: &Transaction,
        origin: Address,
        backend: &mut B,
        tracer: Option<T>,
    ) -> Result<Self> {
        assert!(trx.target().is_some());

        let target = trx.target().unwrap();
        log_data(&[b"ENTER", b"CALL", target.as_bytes()]);

        backend.snapshot();

        backend
            .transfer(origin, target, chain_id, trx.value())
            .await?;

        let execution_code = backend.code(target).await?;

        Ok(Self {
            origin,
            chain_id,
            context: Context {
                caller: origin,
                contract: target,
                contract_chain_id: backend.contract_chain_id(target).await.unwrap_or(chain_id),
                value: trx.value(),
                code_address: Some(target),
            },
            gas_price: trx.gas_price(),
            gas_limit: trx.gas_limit(),
            execution_code,
            call_data: Buffer::from_slice(trx.call_data()),
            return_data: Buffer::empty(),
            return_range: 0..0,
            stack: Stack::new(),
            memory: Memory::new(),
            pc: 0_usize,
            is_static: false,
            reason: Reason::Call,
            parent: None,
            phantom: PhantomData,
            tracer,
        })
    }

    #[maybe_async]
    async fn new_create(
        chain_id: u64,
        trx: &Transaction,
        origin: Address,
        backend: &mut B,
        tracer: Option<T>,
    ) -> Result<Self> {
        assert!(trx.target().is_none());

        let target = Address::from_create(&origin, trx.nonce());
        log_data(&[b"ENTER", b"CREATE", target.as_bytes()]);

        if (backend.nonce(target, chain_id).await? != 0) || (backend.code_size(target).await? != 0)
        {
            return Err(Error::DeployToExistingAccount(target, origin));
        }

        backend.snapshot();

        backend.increment_nonce(target, chain_id).await?;
        backend
            .transfer(origin, target, chain_id, trx.value())
            .await?;

        Ok(Self {
            origin,
            chain_id,
            context: Context {
                caller: origin,
                contract: target,
                contract_chain_id: chain_id,
                value: trx.value(),
                code_address: None,
            },
            gas_price: trx.gas_price(),
            gas_limit: trx.gas_limit(),
            return_data: Buffer::empty(),
            return_range: 0..0,
            stack: Stack::new(),
            memory: Memory::new(),
            pc: 0_usize,
            is_static: false,
            reason: Reason::Create,
            execution_code: Buffer::from_slice(trx.call_data()),
            call_data: Buffer::empty(),
            parent: None,
            phantom: PhantomData,
            tracer,
        })
    }

    #[maybe_async]
    pub async fn execute(
        &mut self,
        step_limit: u64,
        backend: &mut B,
    ) -> Result<(ExitStatus, u64, Option<u64>, Option<T>)> {
        let mut step = 0_u64;
        let mut step_call_solana: Option<u64> = None;

        begin_vm!(
            self,
            backend,
            self.context,
            self.chain_id,
            if self.reason == Reason::Call {
                self.call_data.to_vec()
            } else {
                self.execution_code.to_vec()
            },
            if self.reason == Reason::Call {
                opcode_table::CALL
            } else {
                opcode_table::CREATE
            }
        );

        let status = if is_precompile_address(&self.context.contract) {
            let value = Self::precompile(&self.context.contract, &self.call_data).unwrap();

            backend.commit_snapshot();

            end_vm!(self, backend, ExitStatus::Return(value.clone()));
            ExitStatus::Return(value)
        } else if PrecompiledContracts::is_precompile_extension(&self.context.contract) {
            let address = self.context.contract;
            let value = PrecompiledContracts::call_precompile_extension(
                backend,
                &self.context,
                &address,
                &self.call_data,
                self.is_static,
            )
            .await
            .unwrap()?;

            backend.commit_snapshot();
            end_vm!(self, backend, ExitStatus::Return(value.clone()));
            ExitStatus::Return(value)
        } else {
            loop {
                if step >= step_limit {
                    break ExitStatus::StepLimit;
                }
                step += 1;

                let opcode = self.execution_code.get_or_default(self.pc);
                begin_step!(self, backend);

                let opcode_result = match self.execute_opcode(backend, opcode).await {
                    Ok(result) => result,
                    Err(e) => {
                        let message = build_revert_message(&e.to_string());
                        self.opcode_revert_impl(message, backend).await?
                    }
                };

                match opcode_result {
                    Action::Continue => self.pc += 1,
                    Action::Jump(target) => self.pc = target,
                    Action::Stop => break ExitStatus::Stop,
                    Action::Return(value) => break ExitStatus::Return(value),
                    Action::Revert(value) => break ExitStatus::Revert(value),
                    Action::Suicide => break ExitStatus::Suicide,
                    Action::Interrupted(state) => {
                        if step_call_solana.is_none() && state.is_some() {
                            step_call_solana = Some(step);
                        }
                        break ExitStatus::Interrupted(state);
                    }
                    Action::Noop => {}
                };
            }
        };

        Ok((status, step, step_call_solana, self.tracer.take()))
    }

    fn fork(
        &mut self,
        reason: Reason,
        chain_id: u64,
        context: Context,
        execution_code: Buffer,
        call_data: Buffer,
        gas_limit: Option<U256>,
    ) {
        let mut other = Self {
            origin: self.origin,
            chain_id,
            context,
            gas_price: self.gas_price,
            gas_limit: gas_limit.unwrap_or(self.gas_limit),
            execution_code,
            call_data,
            return_data: Buffer::empty(),
            return_range: 0..0,
            stack: Stack::new(),
            memory: Memory::new(),
            pc: 0_usize,
            is_static: self.is_static,
            reason,
            parent: None,
            phantom: PhantomData,
            tracer: self.tracer.take(),
        };

        core::mem::swap(self, &mut other);
        self.parent = Some(crate::types::boxx::boxx(other));
    }

    fn join(&mut self) -> ManuallyDrop<Boxx<Self>> {
        assert!(self.parent.is_some());

        let mut other = self.parent.take().unwrap();
        core::mem::swap(self, other.as_mut());

        self.tracer = other.tracer.take();

        ManuallyDrop::new(other)
    }

    // backend and exit_status might not be used because end_vm! macros won't run on target_os is not solana
    #[allow(unused_variables)]
    pub async fn end_vm(&mut self, backend: &B, exit_status: ExitStatus) -> Result<()> {
        end_vm!(self, backend, exit_status);
        Ok(())
    }

    pub fn set_tracer(&mut self, tracer: Option<T>) {
        self.tracer = tracer;
    }

    pub fn increment_pc(&mut self) {
        self.pc += 1;
    }

    pub fn context(&self) -> &Context {
        &self.context
    }
}
