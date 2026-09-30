use anyhow::{Context, Result, ensure};
use std::{env, net::SocketAddr, time::Duration};
#[derive(Clone)]
pub struct Config {
    pub redis_url: String,
    pub graph: String,
    pub bind: SocketAddr,
    pub token: Option<String>,
    pub api_base: String,
    pub api_key: Option<String>,
    pub extraction_model: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_dim: usize,
    pub local_model_dir: Option<String>,
    pub candidates: usize,
    pub jobs: usize,
    pub timeout: Duration,
    pub allowed_hosts: Vec<String>,
}
pub fn validate_bind(bind: SocketAddr, token: Option<&str>) -> Result<()> {
    ensure!(
        token.is_none_or(|t| !t.trim().is_empty()),
        "bearer token is empty"
    );
    ensure!(
        bind.ip().is_loopback() || token.is_some(),
        "non-loopback HTTP requires GRAPHITI_TOKEN"
    );
    Ok(())
}
impl Config {
    pub fn from_env() -> Result<Self> {
        let get = |name: &str, fallback: &str| env::var(name).unwrap_or_else(|_| fallback.into());
        let c = Self {
            redis_url: get("FALKORDB_URL", "redis://127.0.0.1:6379"),
            graph: get("GRAPHITI_GRAPH", "graphiti_rs"),
            bind: get("GRAPHITI_BIND", "127.0.0.1:8080")
                .parse()
                .context("invalid GRAPHITI_BIND")?,
            token: env::var("GRAPHITI_TOKEN").ok(),
            api_base: get("MODEL_API_BASE", "https://api.openai.com/v1"),
            api_key: env::var("MODEL_API_KEY").ok(),
            extraction_model: get("EXTRACTION_MODEL", "gpt-4.1-mini"),
            embedding_provider: get("EMBEDDING_PROVIDER", "disabled"),
            embedding_model: get("EMBEDDING_MODEL", "text-embedding-3-small"),
            embedding_dim: get("EMBEDDING_DIM", "1536").parse()?,
            local_model_dir: env::var("LOCAL_MODEL_DIR").ok(),
            candidates: get("SEARCH_CANDIDATES", "200").parse()?,
            jobs: get("INGEST_JOBS", "2").parse()?,
            timeout: Duration::from_secs(get("REQUEST_TIMEOUT_SECS", "30").parse()?),
            allowed_hosts: get("HTTP_ALLOWED_HOSTS", "localhost,127.0.0.1,[::1]")
                .split(',')
                .map(|s| s.trim().to_owned())
                .collect(),
        };
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        validate_bind(self.bind, self.token.as_deref())?;
        ensure!(
            !self.graph.is_empty() && self.graph.len() <= 128,
            "invalid graph name"
        );
        ensure!(
            (50..=2000).contains(&self.candidates),
            "SEARCH_CANDIDATES must be 50..=2000"
        );
        ensure!((1..=8).contains(&self.jobs), "INGEST_JOBS must be 1..=8");
        ensure!(
            (1..=120).contains(&self.timeout.as_secs()),
            "timeout must be 1..=120 seconds"
        );
        ensure!(
            (1..=4096).contains(&self.embedding_dim),
            "EMBEDDING_DIM must be 1..=4096"
        );
        ensure!(
            ["disabled", "external", "local"].contains(&self.embedding_provider.as_str()),
            "unknown embedding provider"
        );
        if self.embedding_provider == "local" {
            ensure!(
                self.local_model_dir.is_some(),
                "local embeddings require LOCAL_MODEL_DIR"
            );
        }
        Ok(())
    }
}
