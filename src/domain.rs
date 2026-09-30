use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractedEntity {
    pub name: String,
    #[serde(default)]
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractedFact {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub fact: String,
    /// Existing fact IDs explicitly replaced by this fact.
    #[serde(default)]
    pub supersedes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub entities: Vec<ExtractedEntity>,
    pub facts: Vec<ExtractedFact>,
}
impl Extraction {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.entities.len() <= 128 && self.facts.len() <= 256,
            "extraction exceeds entity/fact limits"
        );
        let mut names = HashSet::new();
        for entity in &self.entities {
            text(&entity.name, 256, "entity name")?;
            ensure!(entity.summary.len() <= 4096, "entity summary too long");
            ensure!(
                names.insert(normalize(&entity.name)),
                "duplicate normalized entity name"
            );
        }
        for fact in &self.facts {
            text(&fact.fact, 8192, "fact")?;
            text(&fact.relation, 256, "relation")?;
            ensure!(
                names.contains(&normalize(&fact.source))
                    && names.contains(&normalize(&fact.target)),
                "fact references an unknown entity"
            );
            ensure!(fact.supersedes.len() <= 16, "too many superseded facts");
            for id in &fact.supersedes {
                text(id, 128, "superseded ID")?;
            }
        }
        Ok(())
    }
}
pub fn text(value: &str, max: usize, field: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= max,
        "{field} must contain 1..={max} bytes"
    );
    ensure!(!value.contains('\0'), "{field} contains a null byte");
    Ok(())
}
pub fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
pub fn stable_id(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
pub fn valid_at(start: i64, end: Option<i64>, at: i64) -> bool {
    start <= at && end.is_none_or(|end| at < end)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddEpisode {
    pub group_id: String,
    #[serde(default)]
    pub episode_id: Option<String>,
    pub content: String,
    #[serde(default)]
    pub source: String,
    pub reference_time: DateTime<Utc>,
    /// Supply validated structured data to ingest without calling an LLM.
    #[serde(default)]
    pub extraction: Option<Extraction>,
}
impl AddEpisode {
    pub fn validate(&self) -> Result<()> {
        text(&self.group_id, 128, "group_id")?;
        text(&self.content, 65536, "content")?;
        ensure!(self.source.len() <= 1024, "source too long");
        if let Some(id) = &self.episode_id {
            text(id, 128, "episode_id")?;
        }
        if let Some(extraction) = &self.extraction {
            extraction.validate()?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub group_id: String,
    pub content: String,
    pub source: String,
    pub reference_time: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    pub group_id: String,
    pub source_id: String,
    pub target_id: String,
    pub source_name: String,
    pub target_name: String,
    pub relation: String,
    pub fact: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub episodes: Vec<String>,
    pub embedding: Option<Vec<f32>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResult {
    pub episode_id: String,
    pub inserted: bool,
    pub fact_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EpisodeKey {
    pub group_id: String,
    pub episode_id: String,
}
impl EpisodeKey {
    pub fn validate(&self) -> Result<()> {
        text(&self.group_id, 128, "group_id")?;
        text(&self.episode_id, 128, "episode_id")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub group_id: String,
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
}
fn default_limit() -> usize {
    10
}
impl SearchRequest {
    pub fn validate(&self) -> Result<()> {
        text(&self.group_id, 128, "group_id")?;
        text(&self.query, 2048, "query")?;
        ensure!((1..=50).contains(&self.limit), "limit must be 1..=50");
        Ok(())
    }
}
