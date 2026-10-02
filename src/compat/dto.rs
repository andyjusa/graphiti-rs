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
    #[serde(default)]
    pub baseline: Option<Baseline>,
    pub episode: LegacyEpisode,
    pub entities: Vec<LegacyEntity>,
    pub facts: Vec<LegacyFact>,
    /// Existing facts updated only for historical invalidation, not new support.
    #[serde(default)]
    pub invalidated_fact_ids: Vec<String>,
}
/// Exact pre-write properties, retained durably with the prepared graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSnapshot {
    pub props: Map<String, Value>,
    pub labels: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FactSnapshot {
    pub props: Map<String, Value>,
    pub source: String,
    pub target: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Baseline {
    pub entities: std::collections::BTreeMap<String, Option<NodeSnapshot>>,
    pub facts: std::collections::BTreeMap<String, Option<FactSnapshot>>,
    pub mentions: std::collections::BTreeMap<String, String>,
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
        let invalidated: std::collections::HashSet<_> = self.invalidated_fact_ids.iter().collect();
        ensure!(
            invalidated.len() == self.invalidated_fact_ids.len(),
            "duplicate invalidation UUID"
        );
        let mut facts = std::collections::HashSet::new();
        for f in &self.facts {
            policy::uuid(&f.uuid)?;
            ensure!(facts.insert(&f.uuid), "duplicate fact UUID");
            ensure!(
                !ids.contains(&f.uuid) && f.uuid != e.uuid,
                "node/fact UUID collision"
            );
            policy::uuid(&f.source_node_uuid)?;
            policy::uuid(&f.target_node_uuid)?;
            let invalidation = invalidated.contains(&f.uuid);
            ensure!(
                f.group_id == e.group_id
                    && (invalidation
                        || ids.contains(&f.source_node_uuid) && ids.contains(&f.target_node_uuid)),
                "fact group or endpoint mismatch"
            );
            ensure!(
                (invalidation || f.episodes.contains(&e.uuid)) && f.episodes.len() <= 1024,
                "fact provenance must include its episode and respect bounds"
            );
            if invalidation {
                ensure!(
                    f.invalid_at.is_some() && f.expired_at.is_some(),
                    "invalidation requires historical timestamps"
                );
            }
            for id in &f.episodes {
                policy::uuid(id)?;
            }
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
            invalidated.iter().all(|id| facts.contains(*id)),
            "invalidation UUID missing from facts"
        );
        ensure!(
            e.entity_edges.len() == facts.len()
                && e.entity_edges
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    == facts,
            "entity_edges differs from prepared facts"
        );
        ensure!(!ids.contains(&e.uuid), "episode/entity UUID collision");
        if let Some(b) = &self.baseline {
            ensure!(
                b.entities.keys().collect::<std::collections::HashSet<_>>() == ids
                    && b.facts.keys().collect::<std::collections::HashSet<_>>() == facts
                    && b.mentions.keys().collect::<std::collections::HashSet<_>>() == ids,
                "prepared baseline identities differ"
            );
            let mut mention_ids = std::collections::HashSet::new();
            for id in b.mentions.values() {
                policy::uuid(id)?;
                ensure!(
                    mention_ids.insert(id)
                        && !ids.contains(id)
                        && !facts.contains(id)
                        && id != &e.uuid,
                    "MENTIONS UUID collision"
                );
            }
            for (id, before) in &b.entities {
                if let Some(before) = before {
                    ensure!(
                        before.props.get("uuid").and_then(Value::as_str) == Some(id)
                            && before.props.get("group_id").and_then(Value::as_str)
                                == Some(&e.group_id),
                        "baseline entity identity differs"
                    );
                    let after = self.entities.iter().find(|n| &n.uuid == id).unwrap();
                    ensure!(
                        before.props.get("created_at").and_then(Value::as_str)
                            == Some(&after.created_at),
                        "existing entity created_at cannot change"
                    );
                    ensure!(
                        before.labels.iter().all(|l| after.labels.contains(l)),
                        "existing structural labels cannot be removed"
                    );
                }
            }
            for (id, before) in &b.facts {
                if invalidated.contains(id) {
                    ensure!(
                        before.is_some(),
                        "invalidation must update an existing fact"
                    );
                }
                if let Some(before) = before {
                    ensure!(
                        before.props.get("uuid").and_then(Value::as_str) == Some(id)
                            && before.props.get("group_id").and_then(Value::as_str)
                                == Some(&e.group_id),
                        "baseline fact identity differs"
                    );
                    let after = self.facts.iter().find(|f| &f.uuid == id).unwrap();
                    ensure!(
                        before.props.get("created_at").and_then(Value::as_str)
                            == Some(&after.created_at),
                        "existing fact created_at cannot change"
                    );
                    if invalidated.contains(id) {
                        for key in ["uuid", "group_id", "name", "fact", "episodes"] {
                            let after_props = serde_json::to_value(after)?;
                            ensure!(
                                after_props.get(key) == before.props.get(key),
                                "invalidation cannot change fact identity/content/provenance"
                            );
                        }
                    }
                    ensure!(
                        after.source_node_uuid == before.source
                            && after.target_node_uuid == before.target,
                        "existing fact cannot be retargeted"
                    );
                }
            }
        }
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
