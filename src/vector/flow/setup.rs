// Copyright (C) 2026 NodePassProject <https://github.com/NodePassProject>
// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::vector) async fn prepare_with_fallback<T, F, Fut>(
    client: &Arc<PortalClient>,
    initial_lease: FlowLease,
    plan: RoutePlan,
    prepare: F,
) -> std::result::Result<(T, FlowLease, ResolvedRoute), OpenFlowError>
where
    F: FnMut(u32, ResolvedRoute) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    prepare_with_fallback_timeout(client, initial_lease, plan, mix_fallback_timeout(), prepare)
        .await
}

pub(super) async fn prepare_with_fallback_timeout<T, F, Fut>(
    client: &Arc<PortalClient>,
    initial_lease: FlowLease,
    plan: RoutePlan,
    primary_timeout: Duration,
    mut prepare: F,
) -> std::result::Result<(T, FlowLease, ResolvedRoute), OpenFlowError>
where
    F: FnMut(u32, ResolvedRoute) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let initial_id = initial_lease.id();
    let primary = if plan.fallback.is_some() {
        timeout(primary_timeout, prepare(initial_id, plan.primary))
            .await
            .unwrap_or_else(|_| {
                Err(anyhow!(
                    "route {} preparation timed out after {}",
                    plan.primary.label(),
                    humantime::format_duration(primary_timeout),
                ))
            })
    } else {
        prepare(initial_id, plan.primary).await
    };
    match primary {
        Ok(prepared) => Ok((prepared, initial_lease, plan.primary)),
        Err(primary_error) => {
            let Some(fallback) = plan.fallback else {
                return Err(OpenFlowError::Transport(primary_error));
            };
            client.telemetry.emit_runtime(RuntimeEvent::new(
                RuntimeLevel::Warn,
                RuntimeKind::Carrier,
                format!(
                    "route {} failed before commit; retrying {}: {primary_error:#}",
                    plan.primary.label(),
                    fallback.label(),
                ),
            ));
            drop(initial_lease);
            let fallback_lease = client.flow_ids.allocate().map_err(|error| {
                OpenFlowError::Protocol(anyhow!(
                    "route {} failed before commit: {primary_error:#}; failed to allocate fallback flow ID: {error:#}",
                    plan.primary.label(),
                ))
            })?;
            let fallback_id = fallback_lease.id();
            match prepare(fallback_id, fallback).await {
                Ok(prepared) => Ok((prepared, fallback_lease, fallback)),
                Err(fallback_error) => Err(OpenFlowError::Transport(anyhow!(
                    "route {} failed before commit: {primary_error:#}; fallback route {} failed before commit: {fallback_error:#}",
                    plan.primary.label(),
                    fallback.label(),
                ))),
            }
        }
    }
}

pub(in crate::vector) async fn write_open_request(
    writer: &mut BoxWriter,
    pending_auth: Option<AuthFrame>,
    header: FlowHeader,
    target: &Target,
) -> Result<()> {
    let flow = write_flow_header(header)?;
    let mut request = [0u8; AUTH_FRAME_LEN + FLOW_HEADER_LEN + TARGET_MAX_ENCODED_LEN];
    let auth_len = if let Some(auth) = pending_auth {
        request[..AUTH_FRAME_LEN].copy_from_slice(&auth);
        AUTH_FRAME_LEN
    } else {
        0
    };
    request[auth_len..auth_len + FLOW_HEADER_LEN].copy_from_slice(&flow);
    let target_offset = auth_len + FLOW_HEADER_LEN;
    let target_len = encode_target_into(target, &mut request[target_offset..])?;
    timeout(handshake_timeout(), async {
        writer
            .write_all(&request[..target_offset + target_len])
            .await?;
        writer.flush().await
    })
    .await
    .map_err(|_| anyhow!("vector::flow::write_open_request: request write timeout"))?
    .context("vector::flow::write_open_request: failed to write request")?;
    Ok(())
}

pub(in crate::vector) async fn write_header(
    writer: &mut BoxWriter,
    pending_auth: Option<AuthFrame>,
    header: FlowHeader,
) -> Result<()> {
    let flow = write_flow_header(header)?;
    let mut request = [0u8; AUTH_FRAME_LEN + FLOW_HEADER_LEN];
    let auth_len = if let Some(auth) = pending_auth {
        request[..AUTH_FRAME_LEN].copy_from_slice(&auth);
        AUTH_FRAME_LEN
    } else {
        0
    };
    request[auth_len..auth_len + FLOW_HEADER_LEN].copy_from_slice(&flow);
    timeout(handshake_timeout(), async {
        writer
            .write_all(&request[..auth_len + FLOW_HEADER_LEN])
            .await?;
        writer.flush().await
    })
    .await
    .map_err(|_| anyhow!("vector::flow::write_header: flow header write timeout"))?
    .context("vector::flow::write_header: failed to write flow header")?;
    Ok(())
}

pub(in crate::vector) async fn read_ready(
    reader: &mut BoxReader,
) -> std::result::Result<(), SetupResult> {
    read_ready_with_timeout(reader, flow_setup_timeout()).await
}

pub(super) async fn read_ready_with_timeout(
    reader: &mut BoxReader,
    setup_timeout: Duration,
) -> std::result::Result<(), SetupResult> {
    let result = timeout(setup_timeout, read_flow_result(reader))
        .await
        .map_err(|_| SetupResult::InternalError)
        .and_then(|result| result.map_err(|_| SetupResult::InternalError))?;
    match result {
        FlowResult::Ready => Ok(()),
        FlowResult::Reject(error) => Err(error.into()),
    }
}
