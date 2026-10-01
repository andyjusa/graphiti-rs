use super::{config::WriteMode, dto::*, policy};
use anyhow::{Context, Result, bail, ensure};
use redis::{Value as R, aio::ConnectionManager};
use serde_json::{Map, Value, json};
use std::time::Duration;
#[derive(Clone)]
pub struct LegacyStore {
    connection: ConnectionManager,
    mode: WriteMode,
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
    async fn query(
        &self,
        group: &str,
        query: &str,
        params: Value,
        write: bool,
    ) -> Result<Vec<Vec<Value>>> {
        if write {
            policy::write(group)?;
            ensure!(
                self.mode == WriteMode::Synthetic,
                "compatibility mutations require validated synthetic mode; production writes disabled"
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
        let rows = self
            .query(
                group,
                "MATCH (e:Episodic) RETURN properties(e) ORDER BY e.created_at DESC LIMIT $limit",
                json!({"limit":limit}),
                false,
            )
            .await?;
        let mut out = vec![];
        for row in rows {
            let p = map(&row[0])?;
            identity(
                &p,
                group,
                p.get("uuid")
                    .and_then(Value::as_str)
                    .context("missing UUID")?,
            )?;
            out.push(serde_json::from_value(Value::Object(p))?);
        }
        Ok(out)
    }
    pub async fn edge(&self, group: &str, id: &str) -> Result<Option<LegacyFact>> {
        policy::uuid(id)?;
        let rows=self.query(group,"MATCH (a:Entity)-[r:RELATES_TO {uuid:$id}]->(b:Entity) RETURN properties(r),a.uuid,b.uuid",json!({"id":id}),false).await?;
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
        let rows=self.query(group,"MATCH (n:Entity) WHERE (size($terms)=0 OR any(t IN $terms WHERE toLower(n.name) CONTAINS t OR toLower(n.summary) CONTAINS t)) AND (size($types)=0 OR any(t IN $types WHERE t IN labels(n) OR t IN n.labels)) RETURN properties(n),labels(n) ORDER BY n.uuid LIMIT $limit",json!({"terms":terms,"types":types.unwrap_or(&[]),"limit":limit}),false).await?;
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
        let rows=self.query(group,"MATCH (a:Entity)-[r:RELATES_TO]->(b:Entity) WHERE ($center='' OR a.uuid=$center OR b.uuid=$center) AND (size($terms)=0 OR any(t IN $terms WHERE toLower(r.fact) CONTAINS t)) RETURN properties(r),a.uuid,b.uuid ORDER BY r.uuid LIMIT $limit",json!({"center":center.unwrap_or(""),"terms":terms,"limit":limit}),false).await?;
        rows.iter().map(|r| fact(r, group)).collect()
    }
    pub async fn commit(&self, payload: &Prepared) -> Result<bool> {
        ensure!(
            self.mode == WriteMode::Synthetic,
            "production compatibility mutations are disabled"
        );
        payload.validate()?;
        let e = &payload.episode;
        if let Some(existing) = self.episode(&e.group_id, &e.uuid).await? {
            ensure!(
                existing == *e,
                "same-UUID update unsupported until baseline validation"
            );
            for fact in &payload.facts {
                ensure!(
                    self.edge(&e.group_id, &fact.uuid).await?.as_ref() == Some(fact),
                    "committed graph differs from prepared facts; reconciliation refused"
                );
            }
            for node in &payload.entities {
                let found = self
                    .node(&e.group_id, &node.uuid)
                    .await?
                    .context("prepared entity missing")?;
                ensure!(
                    found.uuid == node.uuid
                        && found.group_id == node.group_id
                        && found.name == node.name
                        && found.summary == node.summary
                        && found.created_at == node.created_at
                        && found.attributes == node.attributes
                        && found.name_embedding == node.name_embedding
                        && node.labels.iter().all(|x| found.labels.contains(x)),
                    "committed graph differs from prepared entities; reconciliation refused"
                );
            }
            return Ok(false);
        }
        let mut entities = vec![];
        for n in &payload.entities {
            let mut props = n.attributes.clone();
            props.extend(json!({"uuid":n.uuid,"group_id":n.group_id,"name":n.name,"summary":n.summary,"created_at":n.created_at,"labels":n.labels,"name_embedding":n.name_embedding}).as_object().unwrap().clone());
            entities.push(json!({"uuid":n.uuid,"props":props}));
        }
        let facts:Vec<_>=payload.facts.iter().map(|f|{let mut props=f.attributes.clone();props.extend(json!({"uuid":f.uuid,"group_id":f.group_id,"name":f.name,"fact":f.fact,"created_at":f.created_at,"valid_at":f.valid_at,"invalid_at":f.invalid_at,"expired_at":f.expired_at,"episodes":f.episodes,"source_node_uuid":f.source_node_uuid,"target_node_uuid":f.target_node_uuid,"fact_embedding":f.fact_embedding}).as_object().unwrap().clone());json!({"source":f.source_node_uuid,"target":f.target_node_uuid,"props":props,"uuid":f.uuid})}).collect();
        // Only new nodes/edges are accepted; updates require Python baseline comparison.
        let labels: std::collections::BTreeSet<_> = payload
            .entities
            .iter()
            .flat_map(|n| n.labels.iter().cloned())
            .filter(|l| l != "Entity")
            .collect();
        let label_clauses=labels.iter().map(|label|format!(" FOREACH (n IN [x IN $nodes WHERE '{label}' IN x.props.labels] | MERGE (v:Entity {{uuid:n.uuid}}) SET v:{label})")).collect::<String>();
        let query=r#"OPTIONAL MATCH (existing:Episodic {uuid:$id}) WITH collect(existing) AS existing
 OPTIONAL MATCH (n:Entity) WHERE n.uuid IN $ids WITH existing,collect(n) AS nodes
 OPTIONAL MATCH ()-[r:RELATES_TO]->() WHERE r.uuid IN $fact_ids WITH existing,nodes,collect(r) AS edges
 WHERE size(existing)=0 AND size(nodes)=0 AND size(edges)=0
 FOREACH (n IN $nodes | CREATE (v:Entity) SET v+=n.props SET v.name_embedding=CASE WHEN n.props.name_embedding IS NULL THEN null ELSE vecf32(n.props.name_embedding) END)
 FOREACH (f IN $facts | MERGE (a:Entity {uuid:f.source}) MERGE (b:Entity {uuid:f.target}) CREATE (a)-[r:RELATES_TO]->(b) SET r+=f.props SET r.fact_embedding=CASE WHEN f.props.fact_embedding IS NULL THEN null ELSE vecf32(f.props.fact_embedding) END)
 CREATE (e:Episodic) SET e+=$episode
 FOREACH (n IN $nodes | MERGE (v:Entity {uuid:n.uuid}) CREATE (e)-[m:MENTIONS]->(v) SET m.uuid=randomUUID(),m.group_id=$group,m.created_at=$created)
 RETURN e.uuid"#.replace(" FOREACH (f IN $facts",&format!("{label_clauses} FOREACH (f IN $facts"));
        let rows=self.query(&e.group_id,&query,json!({"id":e.uuid,"ids":payload.entities.iter().map(|n|n.uuid.clone()).collect::<Vec<_>>(),"fact_ids":e.entity_edges,"nodes":entities,"facts":facts,"episode":e,"group":e.group_id,"created":e.created_at}),true).await?;
        ensure!(
            !rows.is_empty(),
            "existing entity/edge/episode update unsupported; no mutation performed"
        );
        Ok(true)
    }
    pub async fn clear(&self, group: &str) -> Result<()> {
        policy::write(group)?;
        self.query(group,"MATCH (n) WITH collect(n) AS nodes FOREACH (n IN nodes | DETACH DELETE n) RETURN size(nodes)",json!({}),true).await?;
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
fn fact(row: &[Value], group: &str) -> Result<LegacyFact> {
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

fn entity(row: &[Value], group: &str) -> Result<LegacyEntity> {
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
