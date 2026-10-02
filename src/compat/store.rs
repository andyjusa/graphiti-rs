use super::{config::WriteMode, dto::*, policy};
use anyhow::{Context, Result, bail, ensure};
use redis::{Value as R, aio::ConnectionManager};
use serde_json::{Map, Value, json};
use std::time::Duration;
// Relationship-first reads preserve the deployed bound-node edge-index workaround.
// Closed query shapes are used; no arbitrary Cypher rewriting.
pub const EDGE_LOOKUP_QUERY: &str = "MATCH ()-[e:RELATES_TO {uuid:$id}]->() WITH *, startNode(e) AS n, endNode(e) AS m WHERE 'Entity' IN labels(n) AND 'Entity' IN labels(m) RETURN properties(e),n.uuid,m.uuid";
pub const EDGE_FACTS_QUERY: &str = "MATCH ()-[e:RELATES_TO]->() WITH *, startNode(e) AS n, endNode(e) AS m WHERE 'Entity' IN labels(n) AND 'Entity' IN labels(m) AND e.group_id=$group AND n.group_id=$group AND m.group_id=$group AND (size($terms)=0 OR any(t IN $terms WHERE toLower(e.fact) CONTAINS t)) RETURN properties(e),n.uuid,m.uuid ORDER BY e.uuid LIMIT $limit";
pub const EDGE_RELATED_QUERY: &str = "MATCH ()-[e:RELATES_TO]->() WITH *, startNode(e) AS n, endNode(e) AS m WHERE 'Entity' IN labels(n) AND 'Entity' IN labels(m) AND e.group_id=$group AND n.group_id=$group AND m.group_id=$group AND n.uuid=$source AND m.uuid=$target RETURN properties(e),n.uuid,m.uuid LIMIT 1001";
#[derive(Clone)]
pub struct LegacyStore {
    connection: ConnectionManager,
    pub(super) mode: WriteMode,
}
impl LegacyStore {
    pub async fn connect(url: &str, mode: WriteMode) -> Result<Self> {
        if mode == WriteMode::Synthetic {
            let u = reqwest::Url::parse(url)?;
            ensure!(
                matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
                    && u.port() == Some(16379),
                "synthetic store requires isolated loopback port 16379"
            );
        }
        let client = redis::Client::open(url)
            .map_err(|_| anyhow::anyhow!("invalid compatibility DB configuration"))?;
        let connection =
            tokio::time::timeout(Duration::from_secs(5), ConnectionManager::new(client))
                .await
                .context("database connection timed out")?
                .map_err(|_| anyhow::anyhow!("database connection failed"))?;
        Ok(Self { connection, mode })
    }
    pub async fn ping(&self) -> Result<()> {
        let mut c = self.connection.clone();
        let _: String = tokio::time::timeout(
            Duration::from_secs(5),
            redis::cmd("PING").query_async(&mut c),
        )
        .await
        .context("database ping timed out")?
        .map_err(|_| anyhow::anyhow!("database ping failed"))?;
        Ok(())
    }
    pub(super) async fn query(
        &self,
        group: &str,
        query: &str,
        params: Value,
        write: bool,
    ) -> Result<Vec<Vec<Value>>> {
        if write {
            policy::write(group)?;
            ensure!(
                self.mode.writable(),
                "compatibility mutations require a writable mode"
            );
        } else {
            policy::read(group)?;
        }
        let mut c = self.connection.clone();
        if !write {
            let exists: bool = tokio::time::timeout(
                Duration::from_secs(5),
                redis::cmd("EXISTS").arg(group).query_async(&mut c),
            )
            .await
            .context("database lookup timed out")?
            .map_err(|_| anyhow::anyhow!("database lookup failed"))?;
            if !exists {
                return Ok(vec![]);
            }
        }
        let mut prefix = "CYPHER ".to_owned();
        for (k, v) in params.as_object().context("invalid query parameters")? {
            ensure!(
                k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "invalid query parameter"
            );
            prefix.push_str(&format!("{k}={} ", crate::store::cypher_value(v)?));
        }
        prefix.push_str(query);
        let response: R = tokio::time::timeout(
            Duration::from_secs(6),
            redis::cmd(if write {
                "GRAPH.QUERY"
            } else {
                "GRAPH.RO_QUERY"
            })
            .arg(group)
            .arg(prefix)
            .arg("--compact")
            .arg("TIMEOUT")
            .arg(5000)
            .query_async(&mut c),
        )
        .await
        .context("database query timed out")?
        .map_err(|_| anyhow::anyhow!("legacy database query failed"))?;
        let response = response
            .extract_error()
            .map_err(|_| anyhow::anyhow!("legacy database query failed"))?;
        let R::Array(top) = response else {
            bail!("invalid legacy DB response")
        };
        let Some(R::Array(rows)) = top.get(1) else {
            return Ok(vec![]);
        };
        rows.iter()
            .map(|r| {
                let R::Array(cells) = r else {
                    bail!("invalid legacy DB row")
                };
                cells.iter().map(decode).collect()
            })
            .collect()
    }
    pub async fn raw_episode(&self, group: &str, id: &str) -> Result<Option<Map<String, Value>>> {
        policy::uuid(id)?;
        let rows = self
            .query(
                group,
                "MATCH (e:Episodic {uuid:$id}) RETURN properties(e)",
                json!({"id":id}),
                false,
            )
            .await?;
        ensure!(rows.len() <= 1, "ambiguous episode UUID");
        let result = rows
            .first()
            .map(|r| {
                r[0].as_object()
                    .cloned()
                    .context("invalid episode properties")
            })
            .transpose()?;
        if let Some(p) = &result {
            identity(p, group, id)?;
        }
        Ok(result)
    }
    pub async fn episode(&self, group: &str, id: &str) -> Result<Option<LegacyEpisode>> {
        self.raw_episode(group, id)
            .await?
            .map(|v| {
                serde_json::from_value(Value::Object(v))
                    .map_err(|_| anyhow::anyhow!("legacy episode missing required metadata"))
            })
            .transpose()
    }
    pub async fn episodes(&self, group: &str, limit: usize) -> Result<Vec<LegacyEpisode>> {
        ensure!(limit <= 100, "episode limit exceeds bound");
        ensure!(limit > 0, "episode limit must be positive");
        let rows = self.query(group,
            "MATCH (e:Episodic) WHERE e.group_id=$group RETURN properties(e),e.uuid ORDER BY e.uuid DESC LIMIT $limit",
            json!({"group":group,"limit":limit}),false).await?;
        let mut episodes = Vec::with_capacity(rows.len());
        for row in rows {
            let uuid = row[1].as_str().context("missing episode UUID")?;
            let props = map(&row[0])?;
            identity(&props, group, uuid)?;
            let episode: LegacyEpisode = serde_json::from_value(Value::Object(props))?;
            timestamp(&episode.created_at)?;
            episodes.push(episode);
        }
        Ok(episodes)
    }
    pub async fn context_episodes(
        &self,
        group: &str,
        reference: &str,
        source: &str,
    ) -> Result<Vec<LegacyEpisode>> {
        let reference = timestamp(reference)?;
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut best = vec![]; let mut after = String::new();
            loop {
                let rows = self.query(group,
                    "MATCH(e:Episodic) WHERE e.group_id=$group AND e.uuid>$after RETURN properties(e),e.uuid ORDER BY e.uuid LIMIT 128",
                    json!({"group":group,"after":after}),false).await?;
                for row in &rows {
                    after=row[1].as_str().context("missing context episode UUID")?.to_owned();
                    let props=map(&row[0])?; identity(&props,group,&after)?;
                    let ep:LegacyEpisode=serde_json::from_value(Value::Object(props))?;
                    let time=timestamp(&ep.valid_at)?;
                    if ep.source==source && time<=reference { best.push((ep,time)); }
                }
                best.sort_by(|a,b| b.1.cmp(&a.1).then_with(|| a.0.uuid.cmp(&b.0.uuid))); best.truncate(10);
                if rows.len()<128 { break; }
            }
            best.reverse(); Ok(best.into_iter().map(|(e,_)|e).collect())
        }).await.context("episode context scan timed out; no partial context used")?
    }
    pub async fn related_facts(
        &self,
        group: &str,
        source: &str,
        target: &str,
    ) -> Result<Vec<LegacyFact>> {
        policy::uuid(source)?;
        policy::uuid(target)?;
        let rows = self
            .query(
                group,
                EDGE_RELATED_QUERY,
                json!({"group":group,"source":source,"target":target}),
                false,
            )
            .await?;
        ensure!(
            rows.len() <= 1000,
            "related fact resolution window exhausted"
        );
        rows.iter().map(|r| fact(r, group)).collect()
    }
    pub async fn edge(&self, group: &str, id: &str) -> Result<Option<LegacyFact>> {
        policy::uuid(id)?;
        let rows = self
            .query(group, EDGE_LOOKUP_QUERY, json!({"id":id}), false)
            .await?;
        ensure!(rows.len() <= 1, "ambiguous fact UUID");
        rows.first().map(|r| fact(r, group)).transpose()
    }
    pub async fn node(&self, group: &str, id: &str) -> Result<Option<LegacyEntity>> {
        policy::uuid(id)?;
        let rows = self
            .query(
                group,
                "MATCH (n:Entity {uuid:$id}) RETURN properties(n),labels(n)",
                json!({"id":id}),
                false,
            )
            .await?;
        ensure!(rows.len() <= 1, "ambiguous entity UUID");
        rows.first().map(|r| entity(r, group)).transpose()
    }
    pub async fn nodes(
        &self,
        group: &str,
        query: &str,
        types: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<LegacyEntity>> {
        ensure!((1..=1000).contains(&limit), "candidate limit exceeds bound");
        let terms: Vec<_> = crate::domain::normalize(query)
            .split_whitespace()
            .take(8)
            .map(str::to_owned)
            .collect();
        let rows=self.query(group,"MATCH (n:Entity) WHERE n.group_id=$group AND (size($terms)=0 OR any(t IN $terms WHERE toLower(n.name) CONTAINS t OR toLower(n.summary) CONTAINS t)) AND (size($types)=0 OR any(t IN $types WHERE t IN labels(n) OR t IN n.labels)) RETURN properties(n),labels(n) ORDER BY n.uuid LIMIT $limit",json!({"group":group,"terms":terms,"types":types.unwrap_or(&[]),"limit":limit}),false).await?;
        rows.iter().map(|row| entity(row, group)).collect()
    }
    pub async fn facts(
        &self,
        group: &str,
        query: &str,
        center: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LegacyFact>> {
        ensure!((1..=1000).contains(&limit), "candidate limit exceeds bound");
        if let Some(center) = center {
            policy::uuid(center)?;
        }
        let terms: Vec<_> = crate::domain::normalize(query)
            .split_whitespace()
            .take(8)
            .map(str::to_owned)
            .collect();
        let rows = self
            .query(
                group,
                EDGE_FACTS_QUERY,
                json!({"group":group,"terms":terms,"limit":limit}),
                false,
            )
            .await?;
        rows.iter().map(|r| fact(r, group)).collect()
    }
    async fn fulltext_index(
        &self,
        group: &str,
        label: &str,
        kind: &str,
        fields: &[&str],
    ) -> Result<bool> {
        let rows = self.query(group,
            "CALL db.indexes() YIELD label,properties,types,entitytype,status WHERE label=$label AND entitytype=$kind AND all(field IN $fields WHERE field IN properties AND 'FULLTEXT' IN types[field]) RETURN status",
            json!({"label":label,"kind":kind,"fields":fields}),false).await?;
        ensure!(rows.len() <= 1, "ambiguous fulltext index");
        let Some(row) = rows.first() else {
            return Ok(false);
        };
        let status = row
            .first()
            .and_then(Value::as_str)
            .context("invalid fulltext index status")?;
        super::fulltext::require_operational(status)?;
        Ok(true)
    }
    pub async fn keyword_nodes(
        &self,
        group: &str,
        query: &str,
        types: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<LegacyEntity>> {
        ensure!((1..=1000).contains(&limit), "candidate limit exceeds bound");
        if self
            .fulltext_index(group, "Entity", "NODE", &["name", "summary", "group_id"])
            .await?
        {
            let Some(query) = super::fulltext::query(query, group)? else {
                return Ok(vec![]);
            };
            let rows = self.query(group,
                "CALL db.idx.fulltext.queryNodes('Entity',$query) YIELD node AS n,score WHERE n.group_id=$group AND (size($types)=0 OR any(t IN $types WHERE t IN labels(n) OR t IN n.labels)) RETURN properties(n),labels(n) ORDER BY score DESC LIMIT $limit",
                json!({"query":query,"group":group,"types":types.unwrap_or(&[]),"limit":limit}),false).await?;
            return rows.iter().map(|r| entity(r, group)).collect();
        }
        let mut nodes = self.nodes(group, query, types, limit).await?;
        nodes.sort_by(|a, b| {
            super::fulltext::substring_rank(query, &format!("{} {}", b.name, b.summary))
                .cmp(&super::fulltext::substring_rank(
                    query,
                    &format!("{} {}", a.name, a.summary),
                ))
                .then_with(|| a.uuid.cmp(&b.uuid))
        });
        Ok(nodes)
    }
    pub async fn keyword_facts(
        &self,
        group: &str,
        query: &str,
        center: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LegacyFact>> {
        self.keyword_facts_filtered(group, query, center, limit, None)
            .await
    }
    pub async fn keyword_facts_filtered(
        &self,
        group: &str,
        query: &str,
        center: Option<&str>,
        limit: usize,
        allowed: Option<&[String]>,
    ) -> Result<Vec<LegacyFact>> {
        if let Some(ids) = allowed {
            if ids.is_empty() {
                return Ok(vec![]);
            }
            ensure!(ids.len() <= 1000, "fact filter exceeds bound");
            for id in ids {
                policy::uuid(id)?;
            }
        }
        ensure!((1..=1000).contains(&limit), "candidate limit exceeds bound");
        if let Some(center) = center {
            policy::uuid(center)?;
        }
        if self
            .fulltext_index(
                group,
                "RELATES_TO",
                "RELATIONSHIP",
                &["name", "fact", "group_id"],
            )
            .await?
        {
            let Some(query) = super::fulltext::query(query, group)? else {
                return Ok(vec![]);
            };
            let rows = self.query(group,
                "CALL db.idx.fulltext.queryRelationships('RELATES_TO',$query) YIELD relationship AS e,score WITH e,score,startNode(e) AS n,endNode(e) AS m WHERE 'Entity' IN labels(n) AND 'Entity' IN labels(m) AND e.group_id=$group AND n.group_id=$group AND m.group_id=$group AND (size($ids)=0 OR e.uuid IN $ids) RETURN properties(e),n.uuid,m.uuid ORDER BY score DESC LIMIT $limit",
                json!({"query":query,"group":group,"limit":limit,"ids":allowed.unwrap_or(&[])}),false).await?;
            return rows.iter().map(|r| fact(r, group)).collect();
        }
        let terms: Vec<_> = crate::domain::normalize(query)
            .split_whitespace()
            .take(8)
            .map(str::to_owned)
            .collect();
        let fallback = EDGE_FACTS_QUERY.replace(
            "RETURN properties(e)",
            "AND (size($ids)=0 OR e.uuid IN $ids) RETURN properties(e)",
        );
        let rows = self
            .query(
                group,
                &fallback,
                json!({"group":group,"terms":terms,"limit":limit,"ids":allowed.unwrap_or(&[])}),
                false,
            )
            .await?;
        let mut facts = rows
            .iter()
            .map(|r| fact(r, group))
            .collect::<Result<Vec<_>>>()?;
        facts.sort_by(|a, b| {
            super::fulltext::substring_rank(query, &b.fact)
                .cmp(&super::fulltext::substring_rank(query, &a.fact))
                .then_with(|| a.uuid.cmp(&b.uuid))
        });
        Ok(facts)
    }
    // Rank every supported vector in the group before bounding the returned
    // rows. Graphiti's Falkor score is (1 + cosine) / 2, not raw cosine.
    pub async fn semantic_nodes(
        &self,
        group: &str,
        vector: &[f32],
        types: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<LegacyEntity>> {
        super::providers::validate_vector(vector)?;
        ensure!((1..=1000).contains(&limit), "candidate limit exceeds bound");
        let rows = self.query(group,
            "MATCH (n:Entity) WHERE n.group_id=$group AND n.name_embedding IS NOT NULL AND (size($types)=0 OR any(t IN $types WHERE t IN labels(n) OR t IN n.labels)) WITH n,(2-vec.cosineDistance(n.name_embedding,vecf32($vector)))/2 AS score WHERE score>$min RETURN properties(n),labels(n) ORDER BY score DESC LIMIT $limit",
            json!({"group":group,"vector":vector,"types":types.unwrap_or(&[]),"min":super::resolution::SEMANTIC_MIN_SCORE,"limit":limit}), false).await?;
        rows.iter().map(|r| entity(r, group)).collect()
    }
    pub async fn semantic_facts(
        &self,
        group: &str,
        vector: &[f32],
        limit: usize,
    ) -> Result<Vec<LegacyFact>> {
        self.semantic_facts_filtered(group, vector, limit, None)
            .await
    }
    pub async fn semantic_facts_filtered(
        &self,
        group: &str,
        vector: &[f32],
        limit: usize,
        allowed: Option<&[String]>,
    ) -> Result<Vec<LegacyFact>> {
        if let Some(ids) = allowed {
            if ids.is_empty() {
                return Ok(vec![]);
            }
            ensure!(ids.len() <= 1000, "fact filter exceeds bound");
            for id in ids {
                policy::uuid(id)?;
            }
        }
        super::providers::validate_vector(vector)?;
        ensure!((1..=1000).contains(&limit), "candidate limit exceeds bound");
        let rows = self.query(group,
            "MATCH ()-[e:RELATES_TO]->() WITH *,startNode(e) AS n,endNode(e) AS m WHERE 'Entity' IN labels(n) AND 'Entity' IN labels(m) AND e.group_id=$group AND n.group_id=$group AND m.group_id=$group AND e.fact_embedding IS NOT NULL AND (size($ids)=0 OR e.uuid IN $ids) WITH e,n,m,(2-vec.cosineDistance(e.fact_embedding,vecf32($vector)))/2 AS score WHERE score>$min RETURN properties(e),n.uuid,m.uuid ORDER BY score DESC LIMIT $limit",
            json!({"group":group,"vector":vector,"min":super::resolution::SEMANTIC_MIN_SCORE,"ids":allowed.unwrap_or(&[]),"limit":limit}), false).await?;
        rows.iter().map(|r| fact(r, group)).collect()
    }
    pub async fn neighbors(
        &self,
        group: &str,
        center: &str,
    ) -> Result<std::collections::HashSet<String>> {
        policy::uuid(center)?;
        let rows = self.query(group, "MATCH (c:Entity {uuid:$center})-[:RELATES_TO]-(n:Entity) WHERE c.group_id=$group AND n.group_id=$group RETURN DISTINCT n.uuid LIMIT 1001", json!({"center":center,"group":group}), false).await?;
        ensure!(
            rows.len() <= 1000,
            "center neighborhood exceeds bounded compatibility search"
        );
        rows.iter()
            .map(|r| {
                r[0].as_str()
                    .map(str::to_owned)
                    .context("invalid neighbor UUID")
            })
            .collect()
    }
    pub async fn plan(&self, payload: &Prepared) -> Result<Prepared> {
        payload.validate()?;
        if payload.baseline.is_some() {
            return Ok(payload.clone());
        }
        let group = &payload.episode.group_id;
        if self.episode(group, &payload.episode.uuid).await?.is_some() {
            // A legacy prepared receipt can still reconcile an exact previously committed graph.
            return Ok(payload.clone());
        }
        let mut p = payload.clone();
        let mut baseline = Baseline::default();
        for n in &mut p.entities {
            let rows = self
                .query(
                    group,
                    "MATCH(n:Entity {uuid:$id}) RETURN properties(n),labels(n)",
                    json!({"id":n.uuid}),
                    false,
                )
                .await?;
            ensure!(rows.len() <= 1, "ambiguous entity UUID");
            let before = rows
                .first()
                .map(|r| -> Result<NodeSnapshot> {
                    let old = entity(r, group)?;
                    n.created_at = old.created_at;
                    let mut attrs = old.attributes;
                    attrs.extend(n.attributes.clone());
                    n.attributes = attrs;
                    for label in old.labels {
                        if !n.labels.contains(&label) {
                            n.labels.push(label);
                        }
                    }
                    Ok(NodeSnapshot {
                        props: map(&r[0])?,
                        labels: serde_json::from_value(r[1].clone())?,
                    })
                })
                .transpose()?;
            baseline.entities.insert(n.uuid.clone(), before);
            baseline
                .mentions
                .insert(n.uuid.clone(), uuid::Uuid::new_v4().to_string());
        }
        for f in &mut p.facts {
            let rows = self
                .query(group, EDGE_LOOKUP_QUERY, json!({"id":f.uuid}), false)
                .await?;
            ensure!(rows.len() <= 1, "ambiguous fact UUID");
            let before = rows
                .first()
                .map(|r| -> Result<FactSnapshot> {
                    let old = fact(r, group)?;
                    ensure!(
                        old.source_node_uuid == f.source_node_uuid
                            && old.target_node_uuid == f.target_node_uuid,
                        "existing fact cannot be retargeted"
                    );
                    f.created_at = old.created_at;
                    let mut attrs = old.attributes;
                    attrs.extend(f.attributes.clone());
                    f.attributes = attrs;
                    let mut episodes = old.episodes;
                    for id in &f.episodes {
                        if !episodes.contains(id) {
                            episodes.push(id.clone());
                        }
                    }
                    f.episodes = episodes;
                    Ok(FactSnapshot {
                        props: map(&r[0])?,
                        source: f.source_node_uuid.clone(),
                        target: f.target_node_uuid.clone(),
                    })
                })
                .transpose()?;
            if p.invalidated_fact_ids.contains(&f.uuid) {
                ensure!(before.is_some(), "invalidation cannot create a fact");
            }
            baseline.facts.insert(f.uuid.clone(), before);
        }
        p.baseline = Some(baseline);
        p.validate()?;
        Ok(p)
    }
    /// Refuse to overwrite a node/fact that changed while model decisions ran.
    pub async fn plan_observed(
        &self,
        payload: &Prepared,
        nodes: &[LegacyEntity],
        facts: &[LegacyFact],
    ) -> Result<Prepared> {
        let mut planned = self.plan(payload).await?;
        let before = planned
            .baseline
            .as_ref()
            .context("new ingestion requires durable before state")?;
        let group = &planned.episode.group_id;
        for old in nodes {
            let snap = before
                .entities
                .get(&old.uuid)
                .and_then(Option::as_ref)
                .context("model entity context missing from plan")?;
            ensure!(
                entity(
                    &[Value::Object(snap.props.clone()), json!(snap.labels)],
                    group
                )? == *old,
                "entity changed during model resolution"
            );
        }
        for old in facts {
            let snap = before
                .facts
                .get(&old.uuid)
                .and_then(Option::as_ref)
                .context("model fact context missing from plan")?;
            ensure!(
                fact(
                    &[
                        Value::Object(snap.props.clone()),
                        json!(snap.source),
                        json!(snap.target)
                    ],
                    group
                )? == *old,
                "fact changed during model resolution"
            );
        }
        // The staged pipeline has already resolved each fact's attributes.
        // Exact reuse/invalidation carries old values; generic model reuse clears
        // them. Do not restore cleared attributes through the legacy merge plan.
        for (resolved, observed) in planned.facts.iter_mut().zip(&payload.facts) {
            ensure!(resolved.uuid == observed.uuid, "planned fact order changed");
            resolved.attributes = observed.attributes.clone();
        }
        planned.validate()?;
        Ok(planned)
    }
    /// A bounded, exact-text fast path only. Fuzzy/LLM entity resolution is a release gate.
    pub async fn resolve_exact(&self, payload: &Prepared) -> Result<Prepared> {
        payload.validate()?;
        ensure!(
            payload.baseline.is_none(),
            "resolution must precede durable planning"
        );
        let group = &payload.episode.group_id;
        let mut p = payload.clone();
        let mut nodes: Vec<LegacyEntity> = vec![];
        let mut remap = std::collections::HashMap::new();
        for n in &p.entities {
            let candidates = self.nodes(group, &n.name, None, 1000).await?;
            ensure!(
                candidates.len() < 1000,
                "entity resolution window exhausted; refusing incomplete resolution"
            );
            let exact: Vec<_> = candidates
                .into_iter()
                .filter(|old| {
                    crate::domain::normalize(&old.name) == crate::domain::normalize(&n.name)
                })
                .collect();
            ensure!(exact.len() <= 1, "ambiguous normalized entity name");
            let resolved = if let Some(old) = exact.into_iter().next() {
                old
            } else if let Some(old) = nodes.iter().find(|old| {
                crate::domain::normalize(&old.name) == crate::domain::normalize(&n.name)
            }) {
                old.clone()
            } else {
                n.clone()
            };
            remap.insert(n.uuid.clone(), resolved.uuid.clone());
            if !nodes.iter().any(|old| old.uuid == resolved.uuid) {
                nodes.push(resolved);
            }
        }
        let mut facts: Vec<LegacyFact> = vec![];
        for f in &mut p.facts {
            f.source_node_uuid = remap[&f.source_node_uuid].clone();
            f.target_node_uuid = remap[&f.target_node_uuid].clone();
            let rows = self
                .query(
                    group,
                    EDGE_RELATED_QUERY,
                    json!({"group":group,"source":f.source_node_uuid,"target":f.target_node_uuid}),
                    false,
                )
                .await?;
            ensure!(rows.len() <= 1000, "fact resolution window exhausted");
            let related: Vec<_> = rows.iter().map(|r| fact(r, group)).collect::<Result<_>>()?;
            let exact: Vec<_> = related
                .into_iter()
                .filter(|old| {
                    crate::domain::normalize(&old.fact) == crate::domain::normalize(&f.fact)
                })
                .collect();
            ensure!(exact.len() <= 1, "ambiguous normalized fact");
            if let Some(mut old) = exact.into_iter().next() {
                if !old.episodes.contains(&p.episode.uuid) {
                    old.episodes.push(p.episode.uuid.clone());
                }
                *f = old;
            }
            if !facts.iter().any(|old| {
                old.source_node_uuid == f.source_node_uuid
                    && old.target_node_uuid == f.target_node_uuid
                    && crate::domain::normalize(&old.fact) == crate::domain::normalize(&f.fact)
            }) {
                facts.push(f.clone());
            }
        }
        p.entities = nodes;
        p.facts = facts;
        p.episode.entity_edges = p.facts.iter().map(|f| f.uuid.clone()).collect();
        p.validate()?;
        Ok(p)
    }
    pub async fn commit(&self, payload: &Prepared) -> Result<bool> {
        ensure!(
            self.mode.writable(),
            "compatibility mutations require a writable mode"
        );
        let payload = self.plan(payload).await?;
        let e = &payload.episode;
        if let Some(existing) = self.episode(&e.group_id, &e.uuid).await? {
            ensure!(
                existing == *e,
                "same-UUID episode differs; reconciliation refused"
            );
            for f in &payload.facts {
                ensure!(
                    self.edge(&e.group_id, &f.uuid).await?.as_ref() == Some(f),
                    "committed graph differs from prepared facts; reconciliation refused"
                );
            }
            for n in &payload.entities {
                let found = self
                    .node(&e.group_id, &n.uuid)
                    .await?
                    .context("prepared entity missing")?;
                ensure!(
                    found == *n
                        || (found.uuid == n.uuid
                            && found.group_id == n.group_id
                            && found.name == n.name
                            && found.summary == n.summary
                            && found.created_at == n.created_at
                            && found.attributes == n.attributes
                            && found.name_embedding == n.name_embedding
                            && n.labels.iter().all(|x| found.labels.contains(x))),
                    "committed graph differs from prepared entities; reconciliation refused"
                );
            }
            if let Some(b) = &payload.baseline {
                let rows = self.query(&e.group_id, "MATCH(e:Episodic {uuid:$id})-[m:MENTIONS]->(n:Entity) RETURN m.uuid,n.uuid,m.group_id,m.created_at", json!({"id":e.uuid}), false).await?;
                ensure!(
                    rows.len() == b.mentions.len()
                        && rows.iter().all(|r| r.len() == 4
                            && r[1]
                                .as_str()
                                .and_then(|id| b.mentions.get(id))
                                .map(|id| json!(id))
                                == Some(r[0].clone())
                            && r[2] == json!(e.group_id)
                            && r[3] == json!(e.created_at)),
                    "committed MENTIONS differ from durable plan"
                );
            } else {
                self.verify_receipt(&payload).await?;
            }
            return Ok(false);
        }
        let baseline = payload
            .baseline
            .as_ref()
            .context("missing durable before state")?;
        let nodes: Vec<_> = payload.entities.iter().map(|n| {
            let mut props = n.attributes.clone();
            props.extend(json!({"uuid":n.uuid,"group_id":n.group_id,"name":n.name,"summary":n.summary,"created_at":n.created_at,"labels":n.labels,"name_embedding":n.name_embedding}).as_object().unwrap().clone());
            json!({"uuid":n.uuid,"props":props,"mention":baseline.mentions[&n.uuid]})
        }).collect();
        let facts: Vec<_> = payload.facts.iter().map(|f| {
            let mut props = f.attributes.clone();
            props.extend(json!({"uuid":f.uuid,"group_id":f.group_id,"name":f.name,"fact":f.fact,"created_at":f.created_at,"valid_at":f.valid_at,"invalid_at":f.invalid_at,"expired_at":f.expired_at,"episodes":f.episodes,"source_node_uuid":f.source_node_uuid,"target_node_uuid":f.target_node_uuid,"fact_embedding":f.fact_embedding}).as_object().unwrap().clone());
            json!({"source":f.source_node_uuid,"target":f.target_node_uuid,"props":props,"uuid":f.uuid})
        }).collect();
        let before_nodes: Vec<_> = baseline
            .entities
            .values()
            .filter_map(|v| v.as_ref())
            .collect();
        let before_facts: Vec<_> = baseline.facts.values().filter_map(|v| v.as_ref()).collect();
        let labels: std::collections::BTreeSet<_> = payload
            .entities
            .iter()
            .flat_map(|n| n.labels.iter().cloned())
            .filter(|l| l != "Entity")
            .collect();
        let label_clauses = labels.iter().map(|label|format!(" FOREACH (n IN [x IN $nodes WHERE '{label}' IN x.props.labels] | MERGE (v:Entity {{uuid:n.uuid}}) SET v:{label})")).collect::<String>();
        // All CAS predicates precede every mutation. Aggregates after a failed WHERE must not revive it.
        let query = r#"OPTIONAL MATCH (e {uuid:$id}) WITH collect(e) AS episodes
 OPTIONAL MATCH (n) WHERE n.uuid IN $ids WITH episodes,collect(n) AS existing_nodes
 OPTIONAL MATCH (a)-[r]->(b) WHERE r.uuid IN $fact_ids
 WITH episodes,existing_nodes,[x IN collect({props:properties(r),source:a.uuid,target:b.uuid,source_group:a.group_id,target_group:b.group_id,entity_endpoints:'Entity' IN labels(a) AND 'Entity' IN labels(b),kind:type(r)}) WHERE x.props IS NOT NULL] AS existing_facts
 WHERE size(episodes)=0 AND size(existing_nodes)=size($before_nodes) AND size(existing_facts)=size($before_facts)
 AND all(before IN $before_nodes WHERE single(n IN existing_nodes WHERE n.uuid=before.props.uuid AND size(keys(n))=size(keys(before.props)) AND all(k IN keys(before.props) WHERE k='name_embedding' OR n[k]=before.props[k]) AND n.name_embedding=CASE WHEN before.props.name_embedding IS NULL THEN n.name_embedding ELSE vecf32(before.props.name_embedding) END AND size(labels(n))=size(before.labels) AND all(l IN labels(n) WHERE l IN before.labels)))
 AND all(before IN $before_facts WHERE single(f IN existing_facts WHERE f.kind='RELATES_TO' AND f.entity_endpoints AND f.source_group=$group AND f.target_group=$group AND f.props.uuid=before.props.uuid AND f.source=before.source AND f.target=before.target AND size(keys(f.props))=size(keys(before.props)) AND all(k IN keys(before.props) WHERE k='fact_embedding' OR f.props[k]=before.props[k]) AND f.props.fact_embedding=CASE WHEN before.props.fact_embedding IS NULL THEN f.props.fact_embedding ELSE vecf32(before.props.fact_embedding) END))
 FOREACH (n IN $nodes | MERGE (v:Entity {uuid:n.uuid}) SET v+=n.props SET v.name_embedding=CASE WHEN n.props.name_embedding IS NULL THEN null ELSE vecf32(n.props.name_embedding) END)
 FOREACH (f IN $facts | MERGE (a:Entity {uuid:f.source}) MERGE (b:Entity {uuid:f.target}) MERGE (a)-[r:RELATES_TO {uuid:f.uuid}]->(b) SET r=f.props SET r.fact_embedding=CASE WHEN f.props.fact_embedding IS NULL THEN null ELSE vecf32(f.props.fact_embedding) END)
 CREATE (e:Episodic) SET e+=$episode
 FOREACH (n IN $nodes | MERGE (v:Entity {uuid:n.uuid}) CREATE(e)-[m:MENTIONS]->(v) SET m.uuid=n.mention,m.group_id=$group,m.created_at=$created)
 RETURN e.uuid"#.replace(" FOREACH (f IN $facts", &format!("{label_clauses} FOREACH (f IN $facts"));
        // Missing vectors compare as null-safe absence, rather than null=null (unknown).
        let query = query.replace("AND n.name_embedding=CASE", "AND (before.props.name_embedding IS NULL AND n.name_embedding IS NULL OR n.name_embedding=CASE").replace("END AND size(labels(n))", "END) AND size(labels(n))").replace("AND f.props.fact_embedding=CASE", "AND (before.props.fact_embedding IS NULL AND f.props.fact_embedding IS NULL OR f.props.fact_embedding=CASE").replace("vecf32(before.props.fact_embedding) END))", "vecf32(before.props.fact_embedding) END)))");
        let rows = self.query(&e.group_id, &query, json!({"id":e.uuid,"ids":payload.entities.iter().map(|n|&n.uuid).collect::<Vec<_>>(),"fact_ids":e.entity_edges,"before_nodes":before_nodes,"before_facts":before_facts,"nodes":nodes,"facts":facts,"episode":e,"group":e.group_id,"created":e.created_at}), true).await?;
        ensure!(
            !rows.is_empty(),
            "prepared before state changed; no mutation performed"
        );
        Ok(true)
    }
    /// Historical committed receipts allow later legitimate shared-provenance updates.
    /// Prepared recovery remains stricter and checks the full after-state in commit().
    pub async fn verify_receipt(&self, p: &Prepared) -> Result<()> {
        p.validate()?;
        let e = &p.episode;
        ensure!(
            self.episode(&e.group_id, &e.uuid).await?.as_ref() == Some(e),
            "committed receipt episode differs"
        );
        for f in &p.facts {
            let actual = self
                .edge(&e.group_id, &f.uuid)
                .await?
                .context("committed receipt fact missing")?;
            ensure!(
                actual.source_node_uuid == f.source_node_uuid
                    && actual.target_node_uuid == f.target_node_uuid
                    && actual.created_at == f.created_at
                    && (p.invalidated_fact_ids.contains(&f.uuid)
                        || actual.episodes.contains(&e.uuid)),
                "committed receipt fact identity/provenance differs"
            );
        }
        for n in &p.entities {
            ensure!(
                self.node(&e.group_id, &n.uuid)
                    .await?
                    .is_some_and(|actual| actual.created_at == n.created_at),
                "committed receipt entity identity differs"
            );
        }
        let rows=self.query(&e.group_id,"MATCH(e:Episodic {uuid:$id})-[m:MENTIONS]->(n:Entity) RETURN m.uuid,n.uuid,m.group_id,m.created_at",json!({"id":e.uuid}),false).await?;
        ensure!(
            rows.len() == p.entities.len()
                && rows
                    .iter()
                    .all(|r| r.len() == 4
                        && r[0].as_str().is_some_and(|id| policy::uuid(id).is_ok()))
                && p.entities.iter().all(|n| rows
                    .iter()
                    .filter(|r| r[1] == json!(n.uuid)
                        && r[2] == json!(e.group_id)
                        && r[3] == json!(e.created_at)
                        && p.baseline
                            .as_ref()
                            .is_none_or(|b| r[0] == json!(b.mentions[&n.uuid])))
                    .count()
                    == 1),
            "committed receipt mentions differ"
        );
        Ok(())
    }
    pub async fn apply_mutation(&self, op: &super::journal::Mutation) -> Result<()> {
        policy::write(&op.group)?;
        match op.kind.as_str() {
            "clear_graph" => self.clear(&op.group).await,
            "delete_episode" => {
                let id = op.uuid.as_deref().context("missing episode UUID")?;
                policy::uuid(id)?;
                self.query(&op.group, "MATCH(n {uuid:$id}) WHERE n.group_id=$group AND (n:Entity OR n:Episodic OR n:Community) WITH collect(n) AS nodes FOREACH(n IN nodes | DETACH DELETE n) RETURN size(nodes)", json!({"id":id,"group":op.group}), true).await?;
                Ok(())
            }
            "delete_entity_edge" => {
                let id = op.uuid.as_deref().context("missing fact UUID")?;
                policy::uuid(id)?;
                self.query(&op.group, "MATCH()-[r:MENTIONS|RELATES_TO|HAS_MEMBER {uuid:$id}]->() WHERE r.group_id=$group WITH collect(r) AS edges FOREACH(r IN edges | DELETE r) RETURN size(edges)", json!({"id":id,"group":op.group}), true).await?;
                Ok(())
            }
            _ => bail!("unknown destructive operation"),
        }
    }
    pub async fn clear(&self, group: &str) -> Result<()> {
        policy::write(group)?;
        self.query(group,"MATCH (n) WHERE n.group_id=$group AND (n:Entity OR n:Episodic OR n:Community) WITH collect(n) AS nodes FOREACH (n IN nodes | DETACH DELETE n) RETURN size(nodes)",json!({"group":group}),true).await?;
        Ok(())
    }
}
fn map(value: &Value) -> Result<Map<String, Value>> {
    value
        .as_object()
        .cloned()
        .context("invalid legacy property map")
}
fn identity(p: &Map<String, Value>, group: &str, id: &str) -> Result<()> {
    ensure!(
        p.get("uuid").and_then(Value::as_str) == Some(id)
            && p.get("group_id").and_then(Value::as_str) == Some(group),
        "legacy identity/group mismatch"
    );
    policy::uuid(id)
}
fn attributes(p: &Map<String, Value>, known: &[&str]) -> Map<String, Value> {
    p.iter()
        .filter(|(k, _)| !known.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
pub(super) fn fact(row: &[Value], group: &str) -> Result<LegacyFact> {
    let mut p = map(&row[0])?;
    identity(
        &p,
        group,
        p.get("uuid")
            .and_then(Value::as_str)
            .context("missing UUID")?,
    )?;
    let attrs = attributes(
        &p,
        &[
            "uuid",
            "group_id",
            "name",
            "fact",
            "created_at",
            "valid_at",
            "invalid_at",
            "expired_at",
            "episodes",
            "source_node_uuid",
            "target_node_uuid",
            "fact_embedding",
        ],
    );
    p.insert("source_node_uuid".into(), row[1].clone());
    p.insert("target_node_uuid".into(), row[2].clone());
    p.insert("attributes".into(), Value::Object(attrs));
    serde_json::from_value(Value::Object(p)).map_err(Into::into)
}
fn decode(v: &R) -> Result<Value> {
    let R::Array(cell) = v else {
        bail!("invalid compact value")
    };
    ensure!(cell.len() == 2, "invalid compact cell");
    let kind: i64 = redis::from_redis_value(&cell[0])?;
    Ok(match kind {
        1 => Value::Null,
        2 => json!(redis::from_redis_value::<String>(&cell[1])?),
        3 => json!(redis::from_redis_value::<i64>(&cell[1])?),
        4 => json!(redis::from_redis_value::<String>(&cell[1])? == "true"),
        5 => json!(redis::from_redis_value::<f64>(&cell[1])?),
        6 => {
            let R::Array(values) = &cell[1] else {
                bail!("invalid compact array")
            };
            Value::Array(values.iter().map(decode).collect::<Result<_>>()?)
        }
        12 => {
            let R::Array(values) = &cell[1] else {
                bail!("invalid compact vector")
            };
            json!(
                values
                    .iter()
                    .map(redis::from_redis_value::<f32>)
                    .collect::<std::result::Result<Vec<_>, _>>()?
            )
        }
        10 => {
            let R::Array(values) = &cell[1] else {
                bail!("invalid compact map")
            };
            ensure!(values.len() % 2 == 0, "invalid compact map pairs");
            let mut map = Map::new();
            for pair in values.chunks(2) {
                map.insert(redis::from_redis_value(&pair[0])?, decode(&pair[1])?);
            }
            Value::Object(map)
        }
        _ => bail!("unsupported legacy value type {kind}"),
    })
}

pub(super) fn entity(row: &[Value], group: &str) -> Result<LegacyEntity> {
    let mut p = map(&row[0])?;
    identity(
        &p,
        group,
        p.get("uuid")
            .and_then(Value::as_str)
            .context("missing UUID")?,
    )?;
    let mut labels: Vec<String> = serde_json::from_value(row[1].clone())?;
    if let Some(stored) = p.get("labels") {
        for label in serde_json::from_value::<Vec<String>>(stored.clone())? {
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
    }
    let known = [
        "uuid",
        "group_id",
        "name",
        "summary",
        "created_at",
        "labels",
        "name_embedding",
    ];
    let attrs = attributes(&p, &known);
    p.insert("attributes".into(), Value::Object(attrs));
    p.insert("labels".into(), json!(labels));
    serde_json::from_value(Value::Object(p)).map_err(Into::into)
}
