use super::policy;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
#[derive(Clone, Serialize, Deserialize)]
pub struct AddMemory {
    pub name: String,
    pub episode_body: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default = "text")]
    pub source: String,
    #[serde(default)]
    pub source_description: String,
    #[serde(default)]
    pub uuid: Option<String>,
}
fn text() -> String {
    "text".into()
}
impl AddMemory {
    pub fn validate(&self) -> Result<&str> {
        let group = self.group_id.as_deref().unwrap_or(policy::DEFAULT_GROUP);
        policy::write(group)?;
        crate::domain::text(&self.name, 512, "name")?;
        crate::domain::text(&self.episode_body, 65536, "episode_body")?;
        ensure!(
            self.source_description.len() <= 4096,
            "source_description too long"
        );
        ensure!(
            ["text", "message", "json"].contains(&self.source.as_str()),
            "invalid source"
        );
        if let Some(id) = &self.uuid {
            policy::uuid(id)?;
        }
        Ok(group)
    }
}
#[derive(Deserialize)]
pub struct Key {
    pub uuid: String,
    pub group_id: String,
}
impl Key {
    pub fn read(&self) -> Result<()> {
        policy::read(&self.group_id)?;
        policy::uuid(&self.uuid)
    }
    pub fn write(&self) -> Result<()> {
        policy::write(&self.group_id)?;
        policy::uuid(&self.uuid)
    }
}
#[derive(Deserialize)]
pub struct NodeSearch {
    pub query: String,
    #[serde(default)]
    pub group_ids: Option<Vec<String>>,
    #[serde(default = "ten")]
    pub max_nodes: i64,
    #[serde(default)]
    pub entity_types: Option<Vec<String>>,
}
#[derive(Deserialize)]
pub struct FactSearch {
    pub query: String,
    #[serde(default)]
    pub group_ids: Option<Vec<String>>,
    #[serde(default = "ten")]
    pub max_facts: i64,
    #[serde(default)]
    pub center_node_uuid: Option<String>,
}
#[derive(Deserialize)]
pub struct Episodes {
    #[serde(default)]
    pub group_ids: Option<Vec<String>>,
    #[serde(default = "ten")]
    pub max_episodes: i64,
}
#[derive(Deserialize)]
pub struct Clear {
    #[serde(default)]
    pub group_ids: Option<Vec<String>>,
}
fn ten() -> i64 {
    10
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LegacyEpisode {
    pub uuid: String,
    pub name: String,
    pub content: String,
    pub source: String,
    pub source_description: String,
    pub group_id: String,
    pub created_at: String,
    pub valid_at: String,
    #[serde(default)]
    pub entity_edges: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LegacyEntity {
    pub uuid: String,
    pub group_id: String,
    pub name: String,
    pub summary: String,
    pub created_at: String,
    pub labels: Vec<String>,
    #[serde(default)]
    pub attributes: Map<String, Value>,
    #[serde(default)]
    pub name_embedding: Option<Vec<f32>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LegacyFact {
    pub uuid: String,
    pub group_id: String,
    pub source_node_uuid: String,
    pub target_node_uuid: String,
    pub name: String,
    pub fact: String,
    pub created_at: String,
    #[serde(default)]
    pub valid_at: Option<String>,
    #[serde(default)]
    pub invalid_at: Option<String>,
    #[serde(default)]
    pub expired_at: Option<String>,
    pub episodes: Vec<String>,
    #[serde(default)]
    pub attributes: Map<String, Value>,
    #[serde(default)]
    pub fact_embedding: Option<Vec<f32>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prepared {
    pub episode: LegacyEpisode,
    pub entities: Vec<LegacyEntity>,
    pub facts: Vec<LegacyFact>,
}
impl Prepared {
    pub fn validate(&self) -> Result<()> {
        let e = &self.episode;
        policy::write(&e.group_id)?;
        policy::uuid(&e.uuid)?;
        timestamp(&e.created_at)?;
        timestamp(&e.valid_at)?;
        ensure!(
            self.entities.len() <= 128 && self.facts.len() <= 256,
            "prepared graph exceeds limits"
        );
        crate::domain::text(&e.name, 512, "episode name")?;
        crate::domain::text(&e.content, 65536, "episode content")?;
        ensure!(
            ["text", "message", "json"].contains(&e.source.as_str())
                && e.source_description.len() <= 4096,
            "invalid episode source metadata"
        );
        let mut ids = std::collections::HashSet::new();
        for n in &self.entities {
            policy::uuid(&n.uuid)?;
            ensure!(ids.insert(&n.uuid), "duplicate entity UUID");
            ensure!(n.group_id == e.group_id, "entity group mismatch");
            timestamp(&n.created_at)?;
            ensure!(
                n.labels.len() <= 16
                    && n.labels.iter().all(|s| !s.is_empty()
                        && s.len() <= 128
                        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')),
                "unsupported entity label"
            );
            attrs(&n.attributes)?;
            if let Some(v) = &n.name_embedding {
                super::providers::validate_vector(v)?;
            }
        }
        let mut facts = std::collections::HashSet::new();
        for f in &self.facts {
            policy::uuid(&f.uuid)?;
            ensure!(facts.insert(&f.uuid), "duplicate fact UUID");
            ensure!(
                f.group_id == e.group_id
                    && ids.contains(&f.source_node_uuid)
                    && ids.contains(&f.target_node_uuid),
                "fact group or endpoint mismatch"
            );
            ensure!(
                f.episodes == vec![e.uuid.clone()],
                "new fact provenance must name its episode"
            );
            timestamp(&f.created_at)?;
            for t in [&f.valid_at, &f.invalid_at, &f.expired_at]
                .into_iter()
                .flatten()
            {
                timestamp(t)?;
            }
            attrs(&f.attributes)?;
            if let Some(v) = &f.fact_embedding {
                super::providers::validate_vector(v)?;
            }
        }
        ensure!(
            e.entity_edges
                .iter()
                .collect::<std::collections::HashSet<_>>()
                == facts,
            "entity_edges differs from prepared facts"
        );
        Ok(())
    }
}
pub fn timestamp(value: &str) -> Result<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| anyhow::anyhow!("invalid RFC3339 timestamp"))
}
pub fn attrs(attrs: &Map<String, Value>) -> Result<()> {
    ensure!(attrs.len() <= 64, "too many attributes");
    for (k, v) in attrs {
        ensure!(
            !k.is_empty() && k.len() <= 512 && !k.contains('\0'),
            "invalid attribute key"
        );
        ensure!(
            ![
                "uuid",
                "group_id",
                "name",
                "summary",
                "created_at",
                "valid_at",
                "invalid_at",
                "expired_at",
                "episodes",
                "source_node_uuid",
                "target_node_uuid",
                "fact",
                "labels",
                "name_embedding",
                "fact_embedding"
            ]
            .contains(&k.as_str()),
            "attribute overwrites a reserved property"
        );
        ensure!(
            v.is_string()
                || v.is_boolean()
                || v.is_number()
                || v.as_array().is_some_and(|a| a.len() <= 256
                    && a.iter()
                        .all(|v| v.is_string() || v.is_boolean() || v.is_number())),
            "unsupported nested attribute value"
        );
    }
    Ok(())
}
