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
                            let p = providers.prepare(&job).await?;
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
                        "same-UUID update unsupported until baseline validation"
                    );
                    return Ok(
                        json!({"message":format!("Episode {} already committed in {}",id,group)}),
                    );
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
                    json!({"message":format!("Found {} episodes",episodes.len()),"episodes":episodes.iter().map(episode_public).collect::<Vec<_>>()}),
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
                let vector = self.query_vector(&r.query).await?;
                let candidates = (self.config.candidates / groups.len()).max(1);
                let mut lists = vec![];
                for group in groups {
                    let mut nodes = self
                        .store
                        .nodes(&group, &r.query, r.entity_types.as_deref(), candidates)
                        .await?;
                    if vector.is_some() {
                        nodes.extend(
                            self.store
                                .nodes(&group, "", r.entity_types.as_deref(), candidates)
                                .await?,
                        );
                    }
                    let mut seen = std::collections::HashSet::new();
                    nodes.retain(|n| seen.insert(n.uuid.clone()));
                    nodes.sort_by(|a, b| {
                        score(
                            &r.query,
                            &b.name,
                            b.name_embedding.as_deref(),
                            vector.as_deref(),
                        )
                        .total_cmp(&score(
                            &r.query,
                            &a.name,
                            a.name_embedding.as_deref(),
                            vector.as_deref(),
                        ))
                        .then_with(|| a.uuid.cmp(&b.uuid))
                    });
                    nodes.truncate(limit);
                    lists.push(nodes.into_iter().map(|n| node_public(&n)).collect());
                }
                let nodes = round_robin(lists, limit);
                Ok(json!({"message":format!("Found {} nodes",nodes.len()),"nodes":nodes}))
            }
            "search_memory_facts" => {
                let r: FactSearch = serde_json::from_value(args)?;
                crate::domain::text(&r.query, 4096, "query")?;
                let groups = policy::groups(r.group_ids.as_deref())?;
                let limit = policy::limit(r.max_facts)?;
                if let Some(id) = &r.center_node_uuid {
                    policy::uuid(id)?;
                }
                let _permit = self.reads.try_acquire()?;
                let vector = self.query_vector(&r.query).await?;
                let candidates = (self.config.candidates / groups.len()).max(1);
                let mut lists = vec![];
                for group in groups {
                    let mut facts = self
                        .store
                        .facts(&group, &r.query, r.center_node_uuid.as_deref(), candidates)
                        .await?;
                    if vector.is_some() {
                        facts.extend(
                            self.store
                                .facts(&group, "", r.center_node_uuid.as_deref(), candidates)
                                .await?,
                        );
                    }
                    let mut seen = std::collections::HashSet::new();
                    facts.retain(|f| seen.insert(f.uuid.clone()));
                    facts.sort_by(|a, b| {
                        score(
                            &r.query,
                            &b.fact,
                            b.fact_embedding.as_deref(),
                            vector.as_deref(),
                        )
                        .total_cmp(&score(
                            &r.query,
                            &a.fact,
                            a.fact_embedding.as_deref(),
                            vector.as_deref(),
                        ))
                        .then_with(|| a.uuid.cmp(&b.uuid))
                    });
                    facts.truncate(limit);
                    lists.push(facts.into_iter().map(|f| fact_public(&f)).collect());
                }
                let facts = round_robin(lists, limit);
                Ok(json!({"message":format!("Found {} facts",facts.len()),"facts":facts}))
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
                // Provenance/orphan semantics differ in the patched baseline; expose a truthful error.
                anyhow::bail!(
                    "legacy deletion parity is not validated; compatibility deletion disabled"
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
                anyhow::bail!(
                    "legacy clear journal accounting is not validated; compatibility clear disabled"
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
