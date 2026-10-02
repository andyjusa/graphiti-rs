//! Staged ingestion for the deployed MCP profile (configured classification-only
//! entity models; generic relationships, no community/saga API).
use super::{
    config::CompatConfig, dto::*, journal::Job, node_resolution, providers::Providers, resolution,
    store::LegacyStore,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct PreparedContext {
    pub payload: Prepared,
    pub nodes_before: Vec<LegacyEntity>,
    pub facts_before: Vec<LegacyFact>,
}
#[derive(Deserialize)]
struct ExtractedEntities {
    extracted_entities: Vec<ExtractedEntity>,
}
#[derive(Deserialize)]
struct ExtractedEntity {
    name: String,
    entity_type_id: i64,
}
#[derive(Deserialize)]
struct NodeResolutions {
    entity_resolutions: Vec<NodeResolution>,
}
#[derive(Deserialize)]
struct NodeResolution {
    id: i64,
    name: String,
    duplicate_name: String,
}
#[derive(Deserialize)]
struct ExtractedEdges {
    edges: Vec<ExtractedEdge>,
}
#[derive(Deserialize)]
struct ExtractedEdge {
    source_entity_name: String,
    target_entity_name: String,
    relation_type: String,
    fact: String,
    #[serde(default)]
    valid_at: Option<String>,
    #[serde(default)]
    invalid_at: Option<String>,
}
#[derive(Deserialize)]
struct Summaries {
    summaries: Vec<Summary>,
}
#[derive(Deserialize)]
struct Summary {
    name: String,
    summary: String,
}

pub async fn prepare(
    config: &CompatConfig,
    providers: &Providers,
    store: &LegacyStore,
    job: &Job,
) -> Result<PreparedContext> {
    let previous = store
        .context_episodes(&job.group, &job.created, &job.request.source)
        .await?;
    let previous: Vec<_> = previous
        .into_iter()
        .filter(|ep| ep.uuid != job.id)
        .map(|ep| ep.content)
        .collect();
    let mut types = vec![
        json!({"entity_type_id":0,"entity_type_name":"Entity","entity_type_description":"Default entity classification. Use this entity type if the entity is not one of the other listed types."}),
    ];
    types.extend(config.entity_types.iter().enumerate().map(|(i,t)| json!({"entity_type_id":i+1,"entity_type_name":t.name,"entity_type_description":t.description})));
    let context = json!({"episode_content":job.request.episode_body,"episode_timestamp":prompt_time(&job.created, true)?,"previous_episodes":previous,"source_description":job.request.source_description,"custom_extraction_instructions":"","entity_types":types});
    let stage = match job.request.source.as_str() {
        "message" => "extract_nodes.extract_message",
        "json" => "extract_nodes.extract_json",
        _ => "extract_nodes.extract_text",
    };
    let response: ExtractedEntities =
        serde_json::from_value(providers.chat(stage, &context).await?)
            .context("invalid node extraction")?;
    ensure!(
        response.extracted_entities.len() <= 128,
        "node extraction exceeds bound"
    );
    let mut extracted = vec![];
    for node in response.extracted_entities {
        if node.name.trim().is_empty() {
            continue;
        }
        crate::domain::text(&node.name, 512, "entity name")?;
        let label = usize::try_from(node.entity_type_id)
            .ok()
            .and_then(|i| types.get(i))
            .and_then(|t| t["entity_type_name"].as_str())
            .unwrap_or("Entity");
        extracted.push(LegacyEntity {
            uuid: uuid::Uuid::new_v4().to_string(),
            group_id: job.group.clone(),
            name: node.name,
            summary: String::new(),
            created_at: job.created.clone(),
            labels: if label == "Entity" {
                vec!["Entity".into()]
            } else {
                vec!["Entity".into(), label.into()]
            },
            attributes: Default::default(),
            name_embedding: None,
        });
    }
    let name_vectors = providers
        .embed(&extracted.iter().map(|n| n.name.clone()).collect::<Vec<_>>())
        .await?;
    let mut candidates = vec![];
    let mut seen = HashSet::new();
    for (node, vector) in extracted.iter().zip(&name_vectors) {
        let keyword = store
            .keyword_nodes(&job.group, &node.name, None, 10)
            .await?;
        let semantic = store.semantic_nodes(&job.group, vector, None, 10).await?;
        for candidate in fuse_nodes(keyword, semantic) {
            if seen.insert(candidate.uuid.clone()) {
                candidates.push(candidate);
            }
        }
    }
    ensure!(candidates.len() <= 1280, "entity context exceeds bound");
    let candidate_index = node_resolution::CandidateIndex::new(&candidates);
    let mut resolved: Vec<_> = extracted
        .iter()
        .map(|node| {
            candidate_index
                .resolve(&node.name)
                .map(|i| candidates[i].clone())
        })
        .collect();
    let unresolved: Vec<_> = resolved
        .iter()
        .enumerate()
        .filter(|(_, n)| n.is_none())
        .map(|(i, _)| i)
        .collect();
    if !unresolved.is_empty() {
        let extracted_context:Vec<_>=unresolved.iter().enumerate().map(|(id,i)| {
            let node=&extracted[*i];
            let description=node.labels.iter().find(|l|l.as_str()!="Entity").and_then(|label|config.entity_types.iter().find(|t|&t.name==label)).map(|t|t.description.as_str()).filter(|description|!description.is_empty()).unwrap_or("Default Entity Type");
            json!({"id":id,"name":node.name,"entity_type":node.labels,"entity_type_description":description})
        }).collect();
        let existing_context: Vec<_> = candidates
            .iter()
            .map(|n| {
                let mut props = n.attributes.clone();
                props.insert("name".into(), json!(n.name));
                props.insert("entity_types".into(), json!(n.labels));
                Value::Object(props)
            })
            .collect();
        let context = json!({"extracted_nodes":extracted_context,"existing_nodes":existing_context,"episode_content":job.request.episode_body,"previous_episodes":previous});
        let response: NodeResolutions =
            serde_json::from_value(providers.chat("dedupe_nodes.nodes", &context).await?)
                .context("invalid node resolution")?;
        ensure!(
            response.entity_resolutions.len() <= 256,
            "node resolution exceeds bound"
        );
        // Match the observed defensive handling of invalid/missing/repeated IDs.
        let by_name: HashMap<_, _> = candidates.iter().map(|n| (n.name.as_str(), n)).collect();
        let mut processed = HashSet::new();
        for decision in response.entity_resolutions {
            let Ok(relative) = usize::try_from(decision.id) else {
                continue;
            };
            let Some(original) = unresolved.get(relative) else {
                continue;
            };
            if !processed.insert(relative) {
                continue;
            }
            ensure!(
                decision.name.len() <= 512 && decision.duplicate_name.len() <= 512,
                "node resolution text exceeds bound"
            );
            resolved[*original] = Some(
                by_name
                    .get(decision.duplicate_name.as_str())
                    .map(|n| (*n).clone())
                    .unwrap_or_else(|| extracted[*original].clone()),
            );
        }
    }
    let mut nodes_before = BTreeMap::new();
    let mut nodes = vec![];
    let mut node_ids = HashSet::new();
    let mut pointers = HashMap::new();
    for (new, old) in extracted.iter().zip(resolved) {
        let node = old.unwrap_or_else(|| new.clone());
        if node.uuid != new.uuid {
            nodes_before.insert(node.uuid.clone(), node.clone());
        }
        pointers.insert(new.uuid.clone(), node.uuid.clone());
        if node_ids.insert(node.uuid.clone()) {
            nodes.push(node);
        }
    }
    let edge_context = json!({"episode_content":job.request.episode_body,"nodes":extracted.iter().map(|n|json!({"name":n.name,"entity_types":n.labels})).collect::<Vec<_>>(),"previous_episodes":previous,"reference_time":prompt_time(&job.created, false)?,"edge_types":[],"custom_extraction_instructions":""});
    let response: ExtractedEdges =
        serde_json::from_value(providers.chat("extract_edges.edge", &edge_context).await?)
            .context("invalid edge extraction")?;
    ensure!(response.edges.len() <= 128, "edge extraction exceeds bound");
    let by_name: HashMap<_, _> = extracted
        .iter()
        .map(|n| (n.name.as_str(), &n.uuid))
        .collect();
    let mut new_facts = vec![];
    let mut exact_facts = HashSet::new();
    for edge in response.edges {
        let (Some(source), Some(target)) = (
            by_name.get(edge.source_entity_name.as_str()),
            by_name.get(edge.target_entity_name.as_str()),
        ) else {
            continue;
        };
        if edge.fact.trim().is_empty() {
            continue;
        }
        crate::domain::text(&edge.fact, 8192, "fact")?;
        crate::domain::text(&edge.relation_type, 512, "relation type")?;
        let (source, target) = (pointers[*source].clone(), pointers[*target].clone());
        if !exact_facts.insert((
            source.clone(),
            target.clone(),
            crate::domain::normalize(&edge.fact),
        )) {
            continue;
        }
        new_facts.push(LegacyFact {
            uuid: uuid::Uuid::new_v4().to_string(),
            group_id: job.group.clone(),
            source_node_uuid: source,
            target_node_uuid: target,
            name: edge.relation_type,
            fact: edge.fact,
            created_at: job.created.clone(),
            valid_at: extracted_time(edge.valid_at.as_deref()),
            invalid_at: extracted_time(edge.invalid_at.as_deref()),
            expired_at: None,
            episodes: vec![job.id.clone()],
            attributes: Default::default(),
            fact_embedding: None,
        });
    }
    let vectors = providers
        .embed(&new_facts.iter().map(|f| f.fact.clone()).collect::<Vec<_>>())
        .await?;
    let mut facts_before = BTreeMap::new();
    let mut facts: BTreeMap<String, LegacyFact> = BTreeMap::new();
    let mut resolved_ids = HashSet::new();
    let mut summary_edges = vec![];
    for (mut edge, vector) in new_facts.into_iter().zip(vectors) {
        edge.fact_embedding = Some(vector.clone());
        let direct = store
            .related_facts(&job.group, &edge.source_node_uuid, &edge.target_node_uuid)
            .await?;
        let ids: Vec<_> = direct.iter().map(|f| f.uuid.clone()).collect();
        let related = fuse_facts(
            store
                .keyword_facts_filtered(&job.group, &edge.fact, None, 10, Some(&ids))
                .await?,
            store
                .semantic_facts_filtered(&job.group, &vector, 10, Some(&ids))
                .await?,
        );
        let related_ids: HashSet<_> = related.iter().map(|f| f.uuid.as_str()).collect();
        let existing: Vec<_> = fuse_facts(
            store
                .keyword_facts(&job.group, &edge.fact, None, 10)
                .await?,
            store.semantic_facts(&job.group, &vector, 10).await?,
        )
        .into_iter()
        .filter(|f| !related_ids.contains(f.uuid.as_str()))
        .collect();
        let exact = related.iter().any(|f| {
            f.source_node_uuid == edge.source_node_uuid
                && f.target_node_uuid == edge.target_node_uuid
                && crate::domain::normalize(&f.fact) == crate::domain::normalize(&edge.fact)
        });
        let decision = if exact || related.is_empty() && existing.is_empty() {
            resolution::Decision::default()
        } else {
            let context = json!({"existing_edges":related.iter().enumerate().map(|(idx,f)|json!({"idx":idx,"fact":f.fact})).collect::<Vec<_>>(),"edge_invalidation_candidates":existing.iter().enumerate().map(|(idx,f)|json!({"idx":idx+related.len(),"fact":f.fact})).collect::<Vec<_>>(),"new_edge":edge.fact});
            let decision: resolution::Decision = serde_json::from_value(
                providers
                    .chat("dedupe_edges.resolve_edge", &context)
                    .await?,
            )
            .context("invalid edge resolution")?;
            ensure!(
                decision.duplicate_facts.len() <= 256 && decision.contradicted_facts.len() <= 256,
                "edge resolution exceeds bound"
            );
            decision
        };
        let (resolved, invalidated, _) =
            resolution::resolve_fact(&edge, &related, &existing, &job.id, &decision, &job.created)?;
        for old in related.iter().chain(&existing) {
            if (old.uuid == resolved.uuid || invalidated.iter().any(|f| f.uuid == old.uuid))
                && let Some(previous) = facts_before.insert(old.uuid.clone(), old.clone())
            {
                ensure!(
                    previous == *old,
                    "fact context changed across extraction steps"
                );
            }
        }
        if resolved.uuid == edge.uuid {
            summary_edges.push(resolved.clone());
        }
        resolved_ids.insert(resolved.uuid.clone());
        // A later invalidation of an earlier resolved fact must be retained.
        facts.insert(resolved.uuid.clone(), resolved);
        for updated in invalidated {
            facts.insert(updated.uuid.clone(), updated);
        }
        ensure!(
            facts.len() <= 256,
            "resolved and invalidated facts exceed bound"
        );
    }
    update_summaries(
        providers,
        &mut nodes,
        &summary_edges,
        &job.request.episode_body,
        &previous,
    )
    .await?;
    let name_vectors = providers
        .embed(&nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>())
        .await?;
    for (node, vector) in nodes.iter_mut().zip(name_vectors) {
        node.name_embedding = Some(vector);
    }
    let mut facts: Vec<_> = facts.into_values().collect();
    let vectors = providers
        .embed(&facts.iter().map(|f| f.fact.clone()).collect::<Vec<_>>())
        .await?;
    for (fact, vector) in facts.iter_mut().zip(vectors) {
        fact.fact_embedding = Some(vector);
    }
    let payload = Prepared {
        baseline: None,
        invalidated_fact_ids: facts
            .iter()
            .filter(|f| !resolved_ids.contains(&f.uuid))
            .map(|f| f.uuid.clone())
            .collect(),
        episode: LegacyEpisode {
            uuid: job.id.clone(),
            name: job.request.name.clone(),
            content: job.request.episode_body.clone(),
            source: job.request.source.clone(),
            source_description: job.request.source_description.clone(),
            group_id: job.group.clone(),
            created_at: job.created.clone(),
            valid_at: job.created.clone(),
            entity_edges: facts.iter().map(|f| f.uuid.clone()).collect(),
        },
        entities: nodes,
        facts,
    };
    payload.validate()?;
    Ok(PreparedContext {
        payload,
        nodes_before: nodes_before.into_values().collect(),
        facts_before: facts_before.into_values().collect(),
    })
}
fn fuse_nodes(mut keyword: Vec<LegacyEntity>, semantic: Vec<LegacyEntity>) -> Vec<LegacyEntity> {
    let order = resolution::rrf(&[
        keyword.iter().map(|n| n.uuid.clone()).collect(),
        semantic.iter().map(|n| n.uuid.clone()).collect(),
    ]);
    keyword.extend(semantic);
    let mut seen = HashSet::new();
    keyword.retain(|n| seen.insert(n.uuid.clone()));
    keyword.sort_by_key(|n| order.iter().position(|id| id == &n.uuid).unwrap());
    keyword.truncate(10);
    keyword
}
fn fuse_facts(mut keyword: Vec<LegacyFact>, semantic: Vec<LegacyFact>) -> Vec<LegacyFact> {
    let order = resolution::rrf(&[
        keyword.iter().map(|n| n.uuid.clone()).collect(),
        semantic.iter().map(|n| n.uuid.clone()).collect(),
    ]);
    keyword.extend(semantic);
    let mut seen = HashSet::new();
    keyword.retain(|n| seen.insert(n.uuid.clone()));
    keyword.sort_by_key(|n| order.iter().position(|id| id == &n.uuid).unwrap());
    keyword.truncate(10);
    keyword
}
/// Python discards unparseable extracted dates and assumes UTC for naive ISO dates.
pub fn extracted_time(value: Option<&str>) -> Option<String> {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
    let value = value?.trim();
    let time = timestamp(value)
        .ok()
        .map(|t| t.with_timezone(&Utc))
        .or_else(|| {
            [
                "%Y-%m-%dT%H:%M:%S%.f",
                "%Y-%m-%d %H:%M:%S%.f",
                "%Y-%m-%dT%H:%M",
            ]
            .iter()
            .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
            .map(|t| Utc.from_utc_datetime(&t))
        })
        .or_else(|| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .map(|d| Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap()))
        })?;
    Some(time.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
}
pub fn truncate_summary(text: &str) -> String {
    let chars: Vec<_> = text.chars().collect();
    if chars.len() <= 500 {
        return text.into();
    }
    let cut = &chars[..500];
    let mut end = 500;
    for (i, c) in cut.iter().enumerate() {
        if ".!?".contains(*c) && (i + 1 == cut.len() || cut[i + 1].is_whitespace()) {
            end = i + 1;
        }
    }
    cut[..end].iter().collect::<String>().trim_end().to_owned()
}
async fn update_summaries(
    providers: &Providers,
    nodes: &mut [LegacyEntity],
    new_edges: &[LegacyFact],
    episode: &str,
    previous: &[String],
) -> Result<()> {
    let mut needs = vec![];
    for (index, node) in nodes.iter_mut().enumerate() {
        let mut parts = vec![node.summary.clone()];
        for edge in new_edges {
            if edge.source_node_uuid == node.uuid {
                parts.push(edge.fact.clone());
            }
            if edge.target_node_uuid == node.uuid {
                parts.push(edge.fact.clone());
            }
        }
        let joined = parts.join("\n").trim().to_owned();
        if !joined.is_empty() && joined.chars().count() <= 2000 {
            node.summary = joined;
        } else {
            needs.push(index);
        }
    }
    for flight in needs.chunks(30) {
        let context = json!({"entities":flight.iter().map(|i|{let n=&nodes[*i];json!({"name":n.name,"summary":n.summary,"entity_types":n.labels,"attributes":n.attributes})}).collect::<Vec<_>>(),"episode_content":episode,"previous_episodes":previous});
        let response: Summaries = serde_json::from_value(
            providers
                .chat("extract_nodes.extract_summaries_batch", &context)
                .await?,
        )
        .context("invalid summary extraction")?;
        ensure!(
            response.summaries.len() <= 128,
            "summary response exceeds bound"
        );
        for summary in response.summaries {
            ensure!(
                summary.name.len() <= 512 && summary.summary.len() <= 65536,
                "summary text exceeds bound"
            );
            for i in flight {
                if nodes[*i].name.to_lowercase() == summary.name.to_lowercase() {
                    nodes[*i].summary = truncate_summary(&summary.summary);
                }
            }
        }
    }
    Ok(())
}

fn prompt_time(value: &str, iso: bool) -> Result<String> {
    let time = timestamp(value)?.with_timezone(&chrono::Utc);
    let mut formatted = time
        .format(if iso {
            "%Y-%m-%dT%H:%M:%S"
        } else {
            "%Y-%m-%d %H:%M:%S"
        })
        .to_string();
    if time.timestamp_subsec_micros() > 0 {
        formatted.push_str(&format!(".{:06}", time.timestamp_subsec_micros()));
    }
    formatted.push_str("+00:00");
    Ok(formatted)
}
