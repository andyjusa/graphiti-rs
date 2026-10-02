use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Clone, Copy, PartialEq)]
pub enum WriteMode {
    ReadOnly,
    Synthetic,
    Operational,
}
impl WriteMode {
    pub fn writable(self) -> bool {
        self != Self::ReadOnly
    }
}
#[derive(Clone, Copy, PartialEq)]
pub enum IngestionPipeline {
    Graphiti,
    CombinedPreview,
}
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct EntityType {
    pub name: String,
    pub description: String,
}
#[derive(Clone)]
pub struct CompatConfig {
    pub redis_url: String,
    pub pipeline: IngestionPipeline,
    pub entity_types: Vec<EntityType>,
    pub mode: WriteMode,
    pub journal_dir: Option<PathBuf>,
    pub llm_base: String,
    pub llm_model: String,
    pub llm_key_file: Option<PathBuf>,
    pub gemini_base: String,
    pub gemini_key_file: Option<PathBuf>,
    pub max_tokens: usize,
    pub timeout: Duration,
    pub capacity: usize,
    pub candidates: usize,
}
impl CompatConfig {
    pub fn from_env() -> Result<Self> {
        let get = |n: &str, d: &str| std::env::var(n).unwrap_or_else(|_| d.into());
        let entity_types = if let Some(path) = std::env::var_os("COMPAT_ENTITY_TYPES_FILE") {
            let data = std::fs::read(path).context("entity type file unavailable")?;
            ensure!(
                data.len() <= 256 * 1024,
                "entity type configuration exceeds bound"
            );
            serde_json::from_slice(&data).context("invalid entity type configuration")?
        } else {
            vec![]
        };
        let c = Self {
            pipeline: match get("COMPAT_INGESTION_PIPELINE", "graphiti").as_str() {
                "graphiti" => IngestionPipeline::Graphiti,
                "combined-preview" => IngestionPipeline::CombinedPreview,
                _ => anyhow::bail!("unknown ingestion pipeline"),
            },
            entity_types,
            redis_url: std::env::var("COMPAT_DB_URL")
                .context("COMPAT_DB_URL must be supplied explicitly")?,
            mode: match get("COMPAT_WRITE_MODE", "read-only").as_str() {
                "read-only" => WriteMode::ReadOnly,
                "synthetic" => WriteMode::Synthetic,
                "operational" => WriteMode::Operational,
                _ => anyhow::bail!("unknown compatibility write mode"),
            },
            journal_dir: std::env::var_os("COMPAT_JOURNAL_DIR").map(PathBuf::from),
            llm_base: get(
                "COMPAT_LLM_BASE",
                "https://factchat-cloud.mindlogic.ai/v1/gateway",
            ),
            llm_model: get("COMPAT_LLM_MODEL", "gpt-6-luna"),
            llm_key_file: std::env::var_os("COMPAT_LLM_KEY_FILE").map(PathBuf::from),
            gemini_base: get(
                "COMPAT_GEMINI_BASE",
                "https://generativelanguage.googleapis.com/v1beta",
            ),
            gemini_key_file: std::env::var_os("COMPAT_GEMINI_KEY_FILE").map(PathBuf::from),
            max_tokens: get("COMPAT_MAX_TOKENS", "0").parse()?,
            timeout: Duration::from_secs(get("COMPAT_TIMEOUT_SECS", "30").parse()?),
            capacity: get("COMPAT_QUEUE_CAPACITY", "32").parse()?,
            candidates: get("COMPAT_SEARCH_CANDIDATES", "200").parse()?,
        };
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        let mut names = std::collections::HashSet::new();
        ensure!(self.entity_types.len() <= 32, "too many entity types");
        for kind in &self.entity_types {
            ensure!(
                !kind.name.is_empty()
                    && kind.name != "Entity"
                    && kind.name.len() <= 128
                    && kind
                        .name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && names.insert(&kind.name)
                    && kind.description.len() <= 8192,
                "invalid entity type definition"
            );
        }
        ensure!(
            (1..=64).contains(&self.capacity),
            "invalid compatibility queue capacity"
        );
        ensure!(
            (10..=1000).contains(&self.candidates),
            "invalid compatibility candidate bound"
        );
        ensure!(
            (1..=120).contains(&self.timeout.as_secs()),
            "invalid provider timeout"
        );
        ensure!(
            self.llm_model == "gpt-6-luna",
            "compatibility model must remain gpt-6-luna"
        );
        if self.mode.writable() {
            ensure!(
                (1..=4096).contains(&self.max_tokens),
                "explicit bounded COMPAT_MAX_TOKENS required"
            );
            ensure!(
                self.journal_dir.is_some(),
                "private journal directory required"
            );
        }
        if self.mode == WriteMode::Synthetic {
            let url = reqwest::Url::parse(&self.redis_url)
                .map_err(|_| anyhow::anyhow!("invalid compatibility DB URL"))?;
            ensure!(
                loopback(&url) && url.port() == Some(16379),
                "synthetic mutation requires loopback DB port 16379"
            );
            ensure!(
                loopback(&reqwest::Url::parse(&self.llm_base)?)
                    && loopback(&reqwest::Url::parse(&self.gemini_base)?),
                "synthetic mode only calls loopback model mocks"
            );
        }
        if self.mode == WriteMode::Operational {
            ensure!(
                self.pipeline == IngestionPipeline::Graphiti,
                "operational mode requires the staged Graphiti pipeline"
            );
            let db = reqwest::Url::parse(&self.redis_url)
                .map_err(|_| anyhow::anyhow!("invalid compatibility DB URL"))?;
            ensure!(
                matches!(db.scheme(), "redis" | "rediss") && db.host_str().is_some(),
                "invalid operational DB URL"
            );
            ensure!(
                self.llm_base.trim_end_matches('/')
                    == "https://factchat-cloud.mindlogic.ai/v1/gateway",
                "operational LLM endpoint differs from the deployed profile"
            );
            ensure!(
                self.gemini_base.trim_end_matches('/')
                    == "https://generativelanguage.googleapis.com/v1beta",
                "operational embedding endpoint differs from the deployed profile"
            );
            read_secret(
                self.llm_key_file
                    .as_deref()
                    .context("operational LLM credential file required")?,
            )?;
            read_secret(
                self.gemini_key_file
                    .as_deref()
                    .context("operational embedding credential file required")?,
            )?;
        }
        Ok(())
    }
}
fn loopback(url: &reqwest::Url) -> bool {
    matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
    )
}
pub fn read_secret(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| anyhow::anyhow!("credential file unavailable"))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 8192,
        "invalid credential file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "credential file must be owner-private"
        );
    }
    let value =
        std::fs::read_to_string(path).map_err(|_| anyhow::anyhow!("credential file unreadable"))?;
    let value = value.trim();
    ensure!(
        !value.is_empty() && value.len() <= 4096 && !value.contains(['\n', '\r']),
        "invalid credential value"
    );
    Ok(value.into())
}
