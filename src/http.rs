use crate::{config::Config, ingest::Memory, mcp::MemoryService};
use anyhow::Result;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
struct Gate {
    token: Option<String>,
    slots: Arc<tokio::sync::Semaphore>,
}
pub fn router(
    memory: Arc<Memory>,
    config: &Config,
    cancellation: CancellationToken,
) -> Result<Router> {
    router_service(MemoryService::new(memory), config, cancellation)
}
pub fn router_service<S: rmcp::ServerHandler + Clone + Send + Sync + 'static>(
    handler: S,
    config: &Config,
    cancellation: CancellationToken,
) -> Result<Router> {
    config.validate()?;
    let mut transport = StreamableHttpServerConfig::default()
        .with_max_request_body_bytes(262144)
        .enforce_origin_validation();
    transport.legacy_session_mode = false;
    transport.json_response = true;
    transport.cancellation_token = cancellation;
    transport.allowed_hosts = config.allowed_hosts.clone();
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        transport,
    );
    Ok(Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(
            Gate {
                token: config.token.clone(),
                slots: Arc::new(tokio::sync::Semaphore::new(16)),
            },
            guard,
        )))
}
async fn guard(State(gate): State<Gate>, request: Request, next: Next) -> Response {
    if let Some(token) = &gate.token {
        let expected = format!("Bearer {token}");
        let supplied = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        use sha2::Digest;
        let a = sha2::Sha256::digest(expected.as_bytes());
        let b = sha2::Sha256::digest(supplied.as_bytes());
        let difference = a.iter().zip(b).fold(0u8, |x, (a, b)| x | (a ^ b));
        if difference != 0 {
            return (StatusCode::UNAUTHORIZED, "authentication required").into_response();
        }
    }
    let Ok(_permit) = gate.slots.try_acquire() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "server busy; retry later").into_response();
    };
    match tokio::time::timeout(std::time::Duration::from_secs(120), next.run(request)).await {
        Ok(response) => response,
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, "request timed out").into_response(),
    }
}
