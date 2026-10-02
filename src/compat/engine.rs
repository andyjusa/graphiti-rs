use super::{
    config::{CompatConfig, IngestionPipeline, WriteMode},
    dto::*,
    journal::Journal,
    policy,
    providers::Providers,
    store::LegacyStore,
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
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
    worker_alive: Arc<AtomicBool>,
}
// Owned by the spawned future: normal exit, panic, and cancellation all stop acceptance.
struct WorkerLifetime(Arc<AtomicBool>);
impl Drop for WorkerLifetime {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
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
            worker_alive: Arc::new(AtomicBool::new(false)),
        });
        if let Some(journal) = &engine.journal {
            let journal = journal.clone();
            let store = engine.store.clone();
            let notify = engine.notify.clone();
            let cancel = engine.cancel.clone();
            let mutation = engine.mutation.clone();
            let config = engine.config.clone();
            let providers = Providers::new(config.clone())?;
            engine.worker_alive.store(true, Ordering::Release);
            let worker = WorkerLifetime(engine.worker_alive.clone());
            tokio::spawn(async move {
                let _worker = worker;
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
                            let p = match config.pipeline {
                                IngestionPipeline::Graphiti => {
                                    let prepared =
                                        super::pipeline::prepare(&config, &providers, &store, &job)
                                            .await?;
                                    let _guard = mutation.lock().await;
                                    store
                                        .plan_observed(
                                            &prepared.payload,
                                            &prepared.nodes_before,
                                            &prepared.facts_before,
                                        )
                                        .await?
                                }
                                IngestionPipeline::CombinedPreview => {
                                    let extracted = providers.prepare(&job).await?;
                                    let _guard = mutation.lock().await;
                                    let resolved = store.resolve_exact(&extracted).await?;
                                    store.plan(&resolved).await?
                                }
                            };
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
        self.execute_inner(name, args, None).await
    }
    /// Offline maintenance only: reuse a captured vector without calling a provider.
    /// This entry point is deliberately absent from the MCP tool registry.
    pub async fn replay_search(&self, name: &str, args: Value, vector: &[f32]) -> Result<Value> {
        ensure!(
            self.config.mode == WriteMode::ReadOnly,
            "replay requires read-only mode"
        );
        ensure!(
            matches!(name, "search_nodes" | "search_memory_facts"),
            "replay only permits search tools"
        );
        super::providers::validate_vector(vector)?;
        self.execute_inner(name, args, Some(vector)).await
    }
    async fn execute_inner(
        &self,
        name: &str,
        args: Value,
        replay: Option<&[f32]>,
    ) -> Result<Value> {
        match name {
            "add_memory" => {
                let r: AddMemory = serde_json::from_value(args)?;
                let group = r.validate()?.to_owned();
                self.writable()?;
                let _guard = self.mutation.lock().await;
                self.writable()?;
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
                    // The observed SDK first takes each group's UUID-descending window.
                    episodes.extend(self.store.episodes(&group, limit).await?);
                }
                for e in &episodes {
                    timestamp(&e.created_at)?;
                }
                // Python's stable global sort retains group/window order on ties.
                episodes.sort_by(|a, b| {
                    timestamp(&b.created_at)
                        .unwrap()
                        .cmp(&timestamp(&a.created_at).unwrap())
                });
                episodes.truncate(limit);
                Ok(
                    json!({"message":if episodes.is_empty(){"No episodes found"}else{"Episodes retrieved successfully"},"episodes":episodes.iter().map(episode_public).collect::<Vec<_>>()}),
                )
            }
            "search_nodes" => {
                let r: NodeSearch = serde_json::from_value(args)?;
                search_text(&r.query)?;
                let groups = policy::groups(r.group_ids.as_deref())?;
                let limit = policy::limit(r.max_nodes)?;
                let _permit = self.reads.try_acquire()?;
                if groups.is_empty() {
                    return Ok(json!({"message":"No relevant nodes found","nodes":[]}));
                }
                let types = r
                    .entity_types
                    .as_deref()
                    .map(policy::node_labels)
                    .transpose()?;
                if r.query.trim().is_empty() {
                    return Ok(json!({"message":"No relevant nodes found","nodes":[]}));
                }
                ensure!(
                    types.as_ref().is_none_or(|t| !t.is_empty()),
                    "empty entity type filter is not a valid SDK label expression"
                );
                super::fulltext::query(&r.query, &groups[0])?;
                let vector = match replay {
                    Some(vector) => Some(vector.to_vec()),
                    None => self.query_vector(&r.query).await?,
                };
                // The deployed wrapper uses NODE_HYBRID_SEARCH_RRF's fixed limit 10.
                // Each method fetches twice that limit, independently for every group.
                let candidates = 20;
                ensure!(
                    candidates <= self.config.candidates,
                    "search candidate bound too small for SDK parity"
                );
                let mut lists = vec![];
                for group in groups {
                    let mut nodes = self
                        .store
                        .keyword_nodes(&group, &r.query, types.as_deref(), candidates)
                        .await?;
                    let keyword_ids = nodes.iter().map(|n| n.uuid.clone()).collect::<Vec<_>>();
                    let semantic = if let Some(vector) = &vector {
                        self.store
                            .semantic_nodes(&group, vector, types.as_deref(), candidates)
                            .await?
                    } else {
                        vec![]
                    };
                    let semantic_ids = semantic.iter().map(|n| n.uuid.clone()).collect::<Vec<_>>();
                    nodes.extend(semantic);
                    let mut seen = std::collections::HashSet::new();
                    nodes.retain(|n| seen.insert(n.uuid.clone()));
                    let order = fused_order(&keyword_ids, &semantic_ids);
                    nodes.retain(|n| order.contains_key(&n.uuid));
                    nodes.sort_by_key(|n| order[&n.uuid]);
                    nodes.truncate(limit.min(10));
                    lists.push(nodes.into_iter().map(|n| node_public(&n)).collect());
                }
                let nodes = round_robin(lists, limit);
                Ok(
                    json!({"message":if nodes.is_empty(){"No relevant nodes found"}else{"Nodes retrieved successfully"},"nodes":nodes}),
                )
            }
            "search_memory_facts" => {
                let r: FactSearch = serde_json::from_value(args)?;
                search_text(&r.query)?;
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
                if groups.is_empty() || r.query.trim().is_empty() {
                    return Ok(json!({"message":"No relevant facts found","facts":[]}));
                }
                super::fulltext::query(&r.query, &groups[0])?;
                let vector = match replay {
                    Some(vector) => Some(vector.to_vec()),
                    None => self.query_vector(&r.query).await?,
                };
                let candidates = 2 * limit;
                ensure!(
                    candidates <= self.config.candidates,
                    "search candidate bound too small for SDK parity"
                );
                let mut lists = vec![];
                for group in groups {
                    let mut facts = self
                        .store
                        .keyword_facts(&group, &r.query, r.center_node_uuid.as_deref(), candidates)
                        .await?;
                    let keyword_ids = facts.iter().map(|f| f.uuid.clone()).collect::<Vec<_>>();
                    let semantic = if let Some(vector) = &vector {
                        self.store
                            .semantic_facts(&group, vector, candidates)
                            .await?
                    } else {
                        vec![]
                    };
                    let semantic_ids = semantic.iter().map(|f| f.uuid.clone()).collect::<Vec<_>>();
                    facts.extend(semantic);
                    let mut seen = std::collections::HashSet::new();
                    facts.retain(|f| seen.insert(f.uuid.clone()));
                    let order = fused_order(&keyword_ids, &semantic_ids);
                    facts.retain(|f| order.contains_key(&f.uuid));
                    facts.sort_by_key(|f| order[&f.uuid]);
                    if let Some(center) = &r.center_node_uuid {
                        let mut sources = vec![];
                        for fact in &facts {
                            if !sources.contains(&fact.source_node_uuid) {
                                sources.push(fact.source_node_uuid.clone());
                            }
                        }
                        let sources =
                            super::resolution::deployed_falkor_center_order(&sources, center);
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
                self.writable()?;
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
                self.writable()?;
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
                if self.config.mode == WriteMode::Synthetic
                    && !self.worker_alive.load(Ordering::Acquire)
                {
                    return Ok(
                        json!({"status":"error","message":"Ingestion worker stopped; restart the server to recover durable jobs"}),
                    );
                }
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
        ensure!(
            self.worker_alive.load(Ordering::Acquire),
            "ingestion worker stopped; restart the server to recover durable jobs"
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
fn search_text(query: &str) -> Result<()> {
    // The SDK returns an empty search for blank text before invoking providers.
    ensure!(
        query.len() <= 4096 && !query.contains('\0'),
        "query exceeds limits or contains a null byte"
    );
    Ok(())
}
pub fn episode_public(e: &LegacyEpisode) -> Value {
    json!({"uuid":e.uuid,"name":e.name,"content":e.content,"source":e.source,"source_description":e.source_description,"created_at":e.created_at,"group_id":e.group_id})
}
pub fn node_public(n: &LegacyEntity) -> Value {
    let attrs: serde_json::Map<String, Value> = n
        .attributes
        .iter()
        .filter(|(key, _)| !key.to_lowercase().contains("embedding"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    json!({"uuid":n.uuid,"name":n.name,"labels":n.labels,"created_at":node_timestamp(&n.created_at),"summary":n.summary,"group_id":n.group_id,"attributes":attrs})
}
fn node_timestamp(value: &str) -> String {
    // The deployed group-aware node wrapper calls Python datetime.isoformat().
    let formatted = public_timestamp(value);
    formatted
        .strip_suffix('Z')
        .map(|s| format!("{s}+00:00"))
        .unwrap_or(formatted)
}
pub fn fact_public(f: &LegacyFact) -> Value {
    json!({"uuid":f.uuid,"name":f.name,"fact":f.fact,"group_id":f.group_id,"source_node_uuid":f.source_node_uuid,"target_node_uuid":f.target_node_uuid,"created_at":public_timestamp(&f.created_at),"valid_at":f.valid_at.as_deref().map(public_timestamp),"invalid_at":f.invalid_at.as_deref().map(public_timestamp),"expired_at":f.expired_at.as_deref().map(public_timestamp),"episodes":f.episodes,"attributes":public_attributes(&f.attributes, "fact_embedding")})
}
fn public_timestamp(value: &str) -> String {
    use chrono::{DateTime, SecondsFormat, Timelike};
    let Ok(parsed) = DateTime::parse_from_rfc3339(value) else {
        return value.to_owned();
    };
    // Observed Pydantic JSON uses UTC Z and Python datetime's microsecond precision.
    let parsed = parsed
        .with_nanosecond(parsed.nanosecond() / 1000 * 1000)
        .unwrap();
    let precision = if parsed.nanosecond() == 0 {
        SecondsFormat::Secs
    } else {
        SecondsFormat::Micros
    };
    parsed.to_rfc3339_opts(precision, true)
}
fn public_attributes(
    attrs: &serde_json::Map<String, Value>,
    vector_key: &str,
) -> serde_json::Map<String, Value> {
    attrs
        .iter()
        .filter(|(k, _)| k.as_str() != vector_key)
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
fn fused_order(
    keyword: &[String],
    semantic: &[String],
) -> std::collections::HashMap<String, usize> {
    super::resolution::rrf(&[keyword.to_vec(), semantic.to_vec()])
        .into_iter()
        .enumerate()
        .map(|(rank, id)| (id, rank))
        .collect()
}
