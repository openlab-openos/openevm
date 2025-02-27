#![allow(clippy::missing_errors_doc)]

use crate::commands::emulate::EmulateResponse;
use crate::config::DbConfig;
use log::info;
use serde_json::Value;
use solana_sdk::pubkey::Pubkey;

use crate::commands::get_config::BuildConfigSimulator;
use crate::errors::NeonError;
use crate::rpc::Rpc;
use crate::tracing::tracers::new_tracer;
use crate::types::EmulateRequest;

pub async fn trace_transaction(
    rpc: &(impl Rpc + BuildConfigSimulator),
    db_config: &Option<DbConfig>,
    program_id: &Pubkey,
    emulate_request: EmulateRequest,
) -> Result<(EmulateResponse, Option<Value>), NeonError> {
    let trace_config = emulate_request
        .trace_config
        .as_ref()
        .map(|c| c.trace_config.clone())
        .unwrap_or_default();

    let tracer = new_tracer(&emulate_request.tx, trace_config)?;

    let response =
        super::emulate::execute(rpc, db_config, program_id, emulate_request, Some(tracer)).await?;

    info!("response: {:?}", response);

    Ok(response)
}
