use super::{
    config::{CompatConfig, WriteMode},
    dto::*,
    journal::Journal,
    policy,
    providers::Providers,
    store::LegacyStore,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio_util::sync::CancellationToken;

pub struct Engine {
    pub store: LegacyStore,
    config: CompatConfig,
    journal: Option<Arc<Journal>>,
    notify: Arc<Notify>,
    mutation: Arc<Mutex<()>>,
    reads: Semaphore,
    cancel: CancellationToken,
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl Engine {
    pub async fn new(config: CompatConfig) -> Result<Arc<Self>> {
        config.validate()?;
        let journal = if config.mode == WriteMode::Synthetic {
            Some(Arc::new(Journal::open(
                config.journal_dir.as_ref().unwrap(),
                config.capacity,
            )?))
        } else {
            None
        };
        let store = LegacyStore::connect(&config.redis_url, config.mode).await?;
        let engine = Arc::new(Self {
            store,
            config,
            journal,
            notify: Arc::new(Notify::new()),
            mutation: Arc::new(Mutex::new(())),
            reads: Semaphore::new(2),
            cancel: CancellationToken::new(),
        });
        if let Some(journal) = &engine.journal {
            let journal = journal.clone();
            let store = engine.store.clone();
            let notify = engine.notify.clone();
            let cancel = engine.cancel.clone();
            let mutation = engine.mutation.clone();
            let providers = Providers::new(engine.config.clone())?;
            tokio::spawn(async move {
                loop {
                    let notified = notify.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    if cancel.is_cancelled() {
                        break;
                    }
                    let recovery_guard = mutation.lock().await;
                    let pending = match journal.mutations() {
                        Ok(pending) => pending,
                        Err(_) => {
                            tracing::error!(
                                "compatibility destructive journal unavailable; worker stopped"
                            );
                            break;
                        }
                    };
                    let mut recovered = true;
                    for op in pending {
                        if store.apply_mutation(&op).await.is_err()
                            || journal.finish_mutation(&op).is_err()
                        {
                            tracing::error!(
                                "compatibility destructive recovery failed; worker stopped"
                            );
                            recovered = false;
                            break;
                        }
                    }
                    drop(recovery_guard);
                    if !recovered {
                        break;
                    }
                    let job = match journal.next() {
                        Ok(Some(job)) => job,
                        Ok(None) => {
                            tokio::select! {_=cancel.cancelled()=>break,_=&mut notified=>{}}
                            continue;
                        }
                        Err(_) => {
                            tracing::error!("compatibility journal unavailable; worker stopped");
                            break;
                        }
                    };
                    let result: Result<()> = async {
                        let payload = if let Some(p) = &job.prepared {
                            p.clone()
                        } else {
                            let extracted = providers.prepare(&job).await?;
                            let _guard = mutation.lock().await;
                            let resolved = store.resolve_exact(&extracted).await?;
                            let p = store.plan(&resolved).await?;
                            journal.prepared(&job, &p)?;
                            p
                        };
                        let _guard = mutation.lock().await;
                        job.validate_prepared(&payload)?;
                        store.commit(&payload).await?;
                        Ok(())
                    }
                    .await;
                    if journal.finish(&job, result.is_ok()).is_err() {
                        tracing::error!(
                            "compatibility journal completion unavailable; worker stopped"
                        );
                        break;
                    }
                    if result.is_err() {
                        tracing::warn!(
                            "compatibility ingestion failed; details and episode content withheld"
                        );
                    }
                }
            });
            engine.notify.notify_one();
        }
        Ok(engine)
    }
    pub async fn execute(&self, name: &str, args: Value) -> Result<Value> {
        match name {
            "add_memory" => {
                let r: AddMemory = serde_json::from_value(args)?;
                let group = r.validate()?.to_owned();
                self.writable()?;
                let _guard = self.mutation.lock().await;
                if let Some(id) = &r.uuid
                    && let Some(e) = self.store.episode(&group, id).await?
                {
                    ensure!(
                        e.name == r.name
                            && e.content == r.episode_body
                            && e.source == r.source
                            && e.source_description == r.source_description,
                        "same-UUID episode input differs; content replacement disabled"
                    );
                    let journal = self.journal.as_ref().unwrap();
                    if !journal.tracked(&group, id)? {
                        return Ok(
                            json!({"message":format!("Episode {} already committed in {}",id,group)}),
                        );
                    }
                    if let Some(prepared) = journal.committed(&group, id)? {
                        self.store.verify_receipt(&prepared).await?;
                    }
                }
                let receipt = self.journal.as_ref().unwrap().accept(r)?;
                self.notify.notify_one();
                Ok(
                    json!({"message":format!("Episode {} {} in {}; acceptance is not durable graph completion",receipt.uuid,receipt.state,group)}),
                )
            }
            "get_episode" => {
                let r: Key = serde_json::from_value(args)?;
                r.read()?;
                let _permit = self.reads.try_acquire()?;
                let episode = self.store.episode(&r.group_id, &r.uuid).await?;
                Ok(match episode {
                    Some(e) => json!({"found":true,"episode":episode_public(&e)}),
                    None => json!({"found":false,"episode":null}),
                })
            }
            "get_entity_edge" => {
                let r: Key = serde_json::from_value(args)?;
                r.read()?;
                let _permit = self.reads.try_acquire()?;
                let f = self.store.edge(&r.group_id, &r.uuid).await?;
                Ok(f.as_ref()
                    .map(fact_public)
                    .unwrap_or_else(|| json!({"error":"Entity edge not found"})))
            }
            "get_episodes" => {
                let r: Episodes = serde_json::from_value(args)?;
                let groups = policy::groups(r.group_ids.as_deref())?;
                let limit = policy::limit(r.max_episodes)?;
                let _permit = self.reads.try_acquire()?;
                let mut episodes = vec![];
                for group in groups {
                    episodes.extend(self.store.episodes(&group, 100).await?);
                    for e in &episodes {
                        timestamp(&e.created_at)?;
                    }
                    episodes.sort_by(|a, b| {
                        timestamp(&b.created_at)
                            .unwrap()
                            .cmp(&timestamp(&a.created_at).unwrap())
                            .then_with(|| a.group_id.cmp(&b.group_id))
                            .then_with(|| a.uuid.cmp(&b.uuid))
                    });
                    episodes.truncate(limit);
                }
                for e in &episodes {
                    timestamp(&e.created_at)?;
                }
                episodes.sort_by(|a, b| {
                    timestamp(&b.created_at)
                        .unwrap()
                        .cmp(&timestamp(&a.created_at).unwrap())
                        .then_with(|| a.group_id.cmp(&b.group_id))
                        .then_with(|| a.uuid.cmp(&b.uuid))
                });
                episodes.truncate(limit);
                Ok(
                    json!({"message":if episodes.is_empty(){"No episodes found"}else{"Episodes retrieved successfully"},"episodes":episodes.iter().map(episode_public).collect::<Vec<_>>()}),
                )
            }
            "search_nodes" => {
                let r: NodeSearch = serde_json::from_value(args)?;
                crate::domain::text(&r.query, 4096, "query")?;
                let groups = policy::groups(r.group_ids.as_deref())?;
                let limit = policy::limit(r.max_nodes)?;
                if let Some(types) = &r.entity_types {
                    ensure!(
                        types.len() <= 32 && types.iter().all(|x| x.len() <= 128),
                        "entity type filter exceeds limits"
                    );
                }
                let _permit = self.reads.try_acquire()?;
                if groups.is_empty() {
                    return Ok(json!({"message":"No relevant nodes found","nodes":[]}));
                }
                let vector = self.query_vector(&r.query).await?;
                let candidates = (self.config.candidates / groups.len().max(1)).max(1);
                let mut lists = vec![];
                for group in groups {
                    let mut nodes = self
                        .store
                        .nodes(&group, &r.query, r.entity_types.as_deref(), candidates)
                        .await?;
                    let keyword_ids = nodes.iter().map(|n| n.uuid.clone()).collect();
                    if vector.is_some() {
                        nodes.extend(
                            self.store
                                .nodes(&group, "", r.entity_types.as_deref(), candidates)
                                .await?,
                        );
                    }
                    let mut seen = std::collections::HashSet::new();
                    nodes.retain(|n| seen.insert(n.uuid.clone()));
                    let order = fused_order(
                        &nodes,
                        &keyword_ids,
                        &r.query,
                        vector.as_deref(),
                        |n| &n.uuid,
                        |n| format!("{} {}", n.name, n.summary),
                        |n| n.name_embedding.as_deref(),
                    );
                    nodes.retain(|n| order.contains_key(&n.uuid));
                    nodes.sort_by_key(|n| order[&n.uuid]);
                    nodes.truncate(limit);
                    lists.push(nodes.into_iter().map(|n| node_public(&n)).collect());
                }
                let nodes = round_robin(lists, limit);
                Ok(
                    json!({"message":if nodes.is_empty(){"No relevant nodes found"}else{"Nodes retrieved successfully"},"nodes":nodes}),
                )
            }
            "search_memory_facts" => {
                let r: FactSearch = serde_json::from_value(args)?;
                crate::domain::text(&r.query, 4096, "query")?;
                let groups = policy::groups(r.group_ids.as_deref())?;
                let limit = policy::limit(r.max_facts)?;
                if let Some(id) = &r.center_node_uuid {
                    policy::uuid(id)?;
                    ensure!(
                        groups.len() == 1,
                        "center_node_uuid requires exactly one group_id"
                    );
                }
                let _permit = self.reads.try_acquire()?;
                if groups.is_empty() {
                    return Ok(json!({"message":"No relevant facts found","facts":[]}));
                }
                let vector = self.query_vector(&r.query).await?;
                let candidates = (self.config.candidates / groups.len().max(1)).max(1);
                let mut lists = vec![];
                for group in groups {
                    let mut facts = self
                        .store
                        .facts(&group, &r.query, r.center_node_uuid.as_deref(), candidates)
                        .await?;
                    let keyword_ids = facts.iter().map(|f| f.uuid.clone()).collect();
                    if vector.is_some() {
                        facts.extend(
                            self.store
                                .facts(&group, "", r.center_node_uuid.as_deref(), candidates)
                                .await?,
                        );
                    }
                    let mut seen = std::collections::HashSet::new();
                    facts.retain(|f| seen.insert(f.uuid.clone()));
                    let order = fused_order(
                        &facts,
                        &keyword_ids,
                        &r.query,
                        vector.as_deref(),
                        |f| &f.uuid,
                        |f| f.fact.clone(),
                        |f| f.fact_embedding.as_deref(),
                    );
                    facts.retain(|f| order.contains_key(&f.uuid));
                    facts.sort_by_key(|f| order[&f.uuid]);
                    if let Some(center) = &r.center_node_uuid {
                        let neighbors = self.store.neighbors(&group, center).await?;
                        let mut sources = vec![];
                        for fact in &facts {
                            if !sources.contains(&fact.source_node_uuid) {
                                sources.push(fact.source_node_uuid.clone());
                            }
                        }
                        let sources = super::resolution::center_order(&sources, center, &neighbors);
                        facts.sort_by_key(|f| {
                            sources
                                .iter()
                                .position(|id| id == &f.source_node_uuid)
                                .unwrap()
                        });
                    }
                    facts.truncate(limit);
                    lists.push(facts.into_iter().map(|f| fact_public(&f)).collect());
                }
                let facts = round_robin(lists, limit);
                Ok(
                    json!({"message":if facts.is_empty(){"No relevant facts found"}else{"Facts retrieved successfully"},"facts":facts}),
                )
            }
            "delete_episode" | "delete_entity_edge" => {
                let r: Key = serde_json::from_value(args)?;
                r.write()?;
                self.writable()?;
                let _guard = self.mutation.lock().await;
                ensure!(
                    !self.journal.as_ref().unwrap().pending(&r.group_id)?,
                    "group has pending ingestion; deletion refused"
                );
                let journal = self.journal.as_ref().unwrap();
                let retry = journal.mutations()?.iter().any(|op| {
                    op.group == r.group_id && op.kind == name && op.uuid.as_deref() == Some(&r.uuid)
                });
                if !retry {
                    if name == "delete_episode" {
                        ensure!(
                            self.store.episode(&r.group_id, &r.uuid).await?.is_some(),
                            "Exact episode delete failed"
                        );
                    } else {
                        ensure!(
                            self.store.edge(&r.group_id, &r.uuid).await?.is_some(),
                            "Exact entity edge delete failed"
                        );
                    }
                }
                let op = journal.begin_mutation(&r.group_id, name, Some(&r.uuid))?;
                self.store.apply_mutation(&op).await?;
                journal.finish_mutation(&op)?;
                Ok(
                    json!({"message":format!("{} with UUID {} deleted successfully", if name=="delete_episode" {"Episode"} else {"Entity edge"},r.uuid)}),
                )
            }
            "clear_graph" => {
                let r: Clear = serde_json::from_value(args)?;
                let group = policy::clear(r.group_ids.as_deref())?;
                self.writable()?;
                let _guard = self.mutation.lock().await;
                ensure!(
                    !self.journal.as_ref().unwrap().pending(group)?,
                    "group has pending ingestion; clear refused"
                );
                let journal = self.journal.as_ref().unwrap();
                let op = journal.begin_mutation(group, "clear_graph", None)?;
                self.store.apply_mutation(&op).await?;
                journal.finish_mutation(&op)?;
                Ok(
                    json!({"message":format!("Graph data cleared successfully for group ID: {group}")}),
                )
            }
            "get_status" => {
                let ok = self.store.ping().await.is_ok();
                Ok(
                    json!({"status":if ok{"ok"}else{"error"},"message":if ok{"Graphiti MCP server is running and connected to falkordb database"}else{"Database connection failed"}}),
                )
            }
            _ => anyhow::bail!("unknown tool"),
        }
    }
    fn writable(&self) -> Result<()> {
        ensure!(
            self.config.mode == WriteMode::Synthetic,
            "production compatibility mutations are disabled"
        );
        Ok(())
    }
    async fn query_vector(&self, query: &str) -> Result<Option<Vec<f32>>> {
        if self.config.mode == WriteMode::ReadOnly {
            return Ok(None);
        }
        let mut v = Providers::new(self.config.clone())?
            .embed(&[query.into()])
            .await?;
        Ok(v.pop())
    }
}
pub fn episode_public(e: &LegacyEpisode) -> Value {
    json!({"uuid":e.uuid,"name":e.name,"content":e.content,"source":e.source,"source_description":e.source_description,"created_at":e.created_at,"group_id":e.group_id})
}
pub fn node_public(n: &LegacyEntity) -> Value {
    json!({"uuid":n.uuid,"name":n.name,"labels":n.labels,"created_at":n.created_at,"summary":n.summary,"group_id":n.group_id,"attributes":safe_attributes(&n.attributes)})
}
pub fn fact_public(f: &LegacyFact) -> Value {
    json!({"uuid":f.uuid,"name":f.name,"fact":f.fact,"group_id":f.group_id,"source_node_uuid":f.source_node_uuid,"target_node_uuid":f.target_node_uuid,"created_at":f.created_at,"valid_at":f.valid_at,"invalid_at":f.invalid_at,"expired_at":f.expired_at,"episodes":f.episodes,"attributes":safe_attributes(&f.attributes)})
}
fn safe_attributes(attrs: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    attrs
        .iter()
        .filter(|(k, _)| !k.to_lowercase().contains("embedding"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
pub fn round_robin(lists: Vec<Vec<Value>>, limit: usize) -> Vec<Value> {
    let mut iterators: Vec<_> = lists.into_iter().map(Vec::into_iter).collect();
    let mut out = vec![];
    while out.len() < limit {
        let start = out.len();
        for list in &mut iterators {
            if let Some(item) = list.next() {
                out.push(item);
                if out.len() == limit {
                    break;
                }
            }
        }
        if start == out.len() {
            break;
        }
    }
    out
}
fn fused_order<T>(
    items: &[T],
    keyword: &std::collections::HashSet<String>,
    query: &str,
    vector: Option<&[f32]>,
    id: impl Fn(&T) -> &str,
    text: impl Fn(&T) -> String,
    embedding: impl Fn(&T) -> Option<&[f32]>,
) -> std::collections::HashMap<String, usize> {
    let mut words: Vec<_> = items.iter().filter(|v| keyword.contains(id(v))).collect();
    words.sort_by(|a, b| {
        score(query, &text(b), None, None).total_cmp(&score(query, &text(a), None, None))
    });
    let mut lists = vec![words.into_iter().map(|v| id(v).to_owned()).collect()];
    if let Some(vector) = vector {
        let mut semantic: Vec<_> = items
            .iter()
            .filter(|v| {
                embedding(v).is_some_and(|e| {
                    e.len() == vector.len()
                        && e.iter().all(|x| x.is_finite())
                        && score("", "", Some(e), Some(vector))
                            > super::resolution::SEMANTIC_MIN_SCORE
                })
            })
            .collect();
        semantic.sort_by(|a, b| {
            score("", "", embedding(b), Some(vector)).total_cmp(&score(
                "",
                "",
                embedding(a),
                Some(vector),
            ))
        });
        lists.push(semantic.into_iter().map(|v| id(v).to_owned()).collect());
    }
    super::resolution::rrf(&lists)
        .into_iter()
        .enumerate()
        .map(|(rank, id)| (id, rank))
        .collect()
}
fn score(query: &str, text: &str, vector: Option<&[f32]>, q: Option<&[f32]>) -> f64 {
    let query = crate::domain::normalize(query);
    let text = crate::domain::normalize(text);
    let keyword = query
        .split_whitespace()
        .filter(|t| text.contains(t))
        .count() as f64;
    let semantic = match (vector, q) {
        (Some(v), Some(q))
            if v.len() == 3072 && q.len() == 3072 && v.iter().all(|x| x.is_finite()) =>
        {
            let dot = v
                .iter()
                .zip(q)
                .map(|(a, b)| *a as f64 * *b as f64)
                .sum::<f64>();
            let norm = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt()
                * q.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
            if norm > 0.0 { dot / norm } else { 0.0 }
        }
        _ => 0.0,
    };
    keyword + semantic
}
