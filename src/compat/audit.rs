//! Read-only restoration inspection through the same native decoders as MCP.
//! No raw memory fields are returned, no indexes or records are created.
use super::{
    config::WriteMode,
    dto::*,
    policy,
    providers::validate_vector,
    store::{LegacyStore, entity, fact},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, time::Duration};

#[derive(Default, Serialize)]
pub struct ReadAudit {
    pub entities: u64,
    pub episodes: u64,
    pub facts: u64,
    pub entity_vectors: u64,
    pub fact_vectors: u64,
    /// Historical copies excluded by the MCP group guard, retained unchanged.
    pub mis_scoped_entities: u64,
    pub mis_scoped_episodes: u64,
    pub mis_scoped_facts: u64,
    pub identity_sha256: String,
    pub vector_sha256: String,
}
pub async fn inspect(store: &LegacyStore, group: &str) -> Result<ReadAudit> {
    ensure!(
        store.mode == WriteMode::ReadOnly,
        "restore audit requires read-only storage"
    );
    policy::read(group)?;
    tokio::time::timeout(Duration::from_secs(120), inspect_inner(store, group))
        .await
        .context("restore audit timed out; no partial success")?
}
async fn count(store: &LegacyStore, group: &str, query: &str) -> Result<u64> {
    let rows = store.query(group, query, json!({}), false).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    rows[0][0].as_u64().context("invalid restore count")
}
fn vector_digest(hash: &mut Sha256, id: &str, vector: &Option<Vec<f32>>) -> Result<bool> {
    if let Some(vector) = vector {
        validate_vector(vector)?;
        hash.update(id.as_bytes());
        hash.update([0]);
        for value in vector {
            hash.update(value.to_le_bytes());
        }
        return Ok(true);
    }
    Ok(false)
}
async fn inspect_inner(store: &LegacyStore, group: &str) -> Result<ReadAudit> {
    let mut report = ReadAudit::default();
    let mut identities = Sha256::new();
    let mut vectors = Sha256::new();
    for kind in ["entity", "episode", "fact"] {
        let (count_query, page_query) = match kind {
            "entity" => (
                "MATCH(n:Entity) RETURN count(n)",
                "MATCH(n:Entity) WHERE n.uuid>$after RETURN properties(n),labels(n),n.uuid ORDER BY n.uuid LIMIT 128",
            ),
            "episode" => (
                "MATCH(n:Episodic) RETURN count(n)",
                "MATCH(n:Episodic) WHERE n.uuid>$after RETURN properties(n),n.uuid ORDER BY n.uuid LIMIT 128",
            ),
            _ => (
                "MATCH()-[e:RELATES_TO]->() RETURN count(e)",
                "MATCH()-[e:RELATES_TO]->() WITH *,startNode(e) AS n,endNode(e) AS m WHERE e.uuid>$after RETURN properties(e),n.uuid,m.uuid,n.group_id,m.group_id,labels(n),labels(m),e.uuid ORDER BY e.uuid LIMIT 128",
            ),
        };
        let expected = count(store, group, count_query).await?;
        let mut after = String::new();
        let mut seen = BTreeSet::new();
        loop {
            let rows = store
                .query(group, page_query, json!({"after":after}), false)
                .await?;
            for row in &rows {
                let id = row
                    .last()
                    .and_then(Value::as_str)
                    .context("missing restore UUID")?;
                policy::uuid(id)?;
                ensure!(seen.insert(id.to_owned()), "duplicate restore UUID");
                ensure!(id > after.as_str(), "unordered restore identities");
                after = id.to_owned();
                let stored_group = row[0]["group_id"]
                    .as_str()
                    .context("missing restored group identity")?;
                ensure!(!stored_group.is_empty(), "empty restored group identity");
                let mis_scoped = u64::from(stored_group != group);
                identities.update(kind.as_bytes());
                identities.update([0]);
                identities.update(id.as_bytes());
                identities.update([0]);
                identities.update(stored_group.as_bytes());
                identities.update([0]);
                match kind {
                    "entity" => {
                        // Maintenance inspects the stored identity explicitly.
                        // MCP continues to decode against the requested group.
                        let node = entity(row, stored_group)?;
                        timestamp(&node.created_at)?;
                        report.entity_vectors +=
                            u64::from(vector_digest(&mut vectors, id, &node.name_embedding)?);
                        report.entities += 1;
                        report.mis_scoped_entities += mis_scoped;
                    }
                    "episode" => {
                        let episode: LegacyEpisode = serde_json::from_value(row[0].clone())
                            .context("invalid restored episode metadata")?;
                        ensure!(
                            episode.group_id == stored_group && episode.uuid == id,
                            "restored episode identity mismatch"
                        );
                        timestamp(&episode.created_at)?;
                        timestamp(&episode.valid_at)?;
                        for edge in &episode.entity_edges {
                            policy::uuid(edge)?;
                        }
                        report.episodes += 1;
                        report.mis_scoped_episodes += mis_scoped;
                    }
                    _ => {
                        ensure!(
                            row[3] == stored_group && row[4] == stored_group,
                            "restored fact endpoint group mismatch"
                        );
                        for labels in [&row[5], &row[6]] {
                            ensure!(
                                labels
                                    .as_array()
                                    .is_some_and(|labels| labels.iter().any(|l| l == "Entity")),
                                "restored fact endpoint is not an Entity"
                            );
                        }
                        let edge = fact(row, stored_group)?;
                        timestamp(&edge.created_at)?;
                        policy::uuid(&edge.source_node_uuid)?;
                        policy::uuid(&edge.target_node_uuid)?;
                        for date in [&edge.valid_at, &edge.invalid_at, &edge.expired_at]
                            .into_iter()
                            .flatten()
                        {
                            timestamp(date)?;
                        }
                        for episode in &edge.episodes {
                            policy::uuid(episode)?;
                        }
                        report.fact_vectors +=
                            u64::from(vector_digest(&mut vectors, id, &edge.fact_embedding)?);
                        report.facts += 1;
                        report.mis_scoped_facts += mis_scoped;
                    }
                }
            }
            if rows.len() < 128 {
                break;
            }
        }
        ensure!(
            seen.len() as u64 == expected,
            "restore audit omitted records (missing/duplicate UUID or concurrent change)"
        );
    }
    report.identity_sha256 = format!("{:x}", identities.finalize());
    report.vector_sha256 = format!("{:x}", vectors.finalize());
    Ok(report)
}
