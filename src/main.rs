use anyhow::Result;
use clap::{Parser, ValueEnum};
use graphiti_rs::{config::Config, ingest::Memory, mcp::MemoryService};
use rmcp::{ServiceExt, transport::stdio};
use std::future::IntoFuture;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
#[derive(Parser)]
#[command(version, about = "Lightweight temporal graph memory for Raspberry Pi")]
struct Args {
    #[arg(long, value_enum, default_value = "stdio")]
    transport: Transport,
}
#[derive(Clone, ValueEnum)]
enum Transport {
    Stdio,
    Http,
}
fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?;
    let result = runtime.block_on(run());
    // Local CPU forward passes are not interruptible; never wait indefinitely at process shutdown.
    runtime.shutdown_timeout(Duration::from_secs(10));
    result
}
async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let config = Config::from_env()?;
    let memory = Arc::new(Memory::new(&config).await?);
    match args.transport {
        Transport::Stdio => {
            let service = MemoryService::new(memory).serve(stdio()).await?;
            let cancel = service.cancellation_token();
            let waiting = service.waiting();
            tokio::pin!(waiting);
            tokio::select! {_ = shutdown_signal()=>{cancel.cancel();let _=tokio::time::timeout(Duration::from_secs(10),&mut waiting).await;}, result=&mut waiting=>{result?;}}
        }
        Transport::Http => {
            let cancellation = CancellationToken::new();
            let router = graphiti_rs::http::router(memory, &config, cancellation.clone())?;
            let listener = tokio::net::TcpListener::bind(config.bind).await?;
            tracing::info!(bind=%config.bind,"MCP HTTP listening at /mcp");
            let shutdown = cancellation.clone();
            let server = axum::serve(listener, router).with_graceful_shutdown(async move {
                shutdown_signal().await;
                shutdown.cancel();
            });
            let result = server.into_future();
            tokio::pin!(result);
            tokio::select! {r=&mut result=>r?,_=cancellation.cancelled()=>{let _=tokio::time::timeout(Duration::from_secs(10),&mut result).await;}}
        }
    }
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
